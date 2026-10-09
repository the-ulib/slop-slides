/* SlopSlide slide editor. The app adds it to the stage's slide preview in edit mode (see
   src-tauri/src/protocol.rs); it is never part of deck.html or an export.

   Click selects an element, drag (or arrow keys) moves it. Its corner and edge handles
   stretch it while the opposite side stays put (Shift keeps the aspect ratio, Alt scales
   from the center); the round handle above it rotates it and the diamond next to it tilts it
   in depth, in 3D with perspective (Shift snaps to 15°). Double-click a handle to reset.
   Double-click (or Enter) edits its text, Escape selects the parent. Delete (or Backspace, or
   { type: "slop:edit-delete" } from the app) removes it.
   Every change is posted to the app as the slide's new markup:
   { type: "slop:edit-commit", slide, markup, select }. Whether anything is selected is posted
   as { type: "slop:edit-selection", slide, selected } whenever that changes.

   It runs on the pasteboard (pasteboard.js), which pans and zooms the view; dragging empty space
   pans too. The slide no longer clips, so content that runs past its edge stays visible (dimmed
   outside the slide, in the color of the panel around it), and the slide's frame turns into the
   app's accent color. Content moved far away can always be panned to and dragged back.
   Anything that runs past the slide's edge or is cut off by its own box is outlined with a red
   dashed wire and reported as { type: "slop:edit-overflow", slide, items } so the app can offer
   to tidy the layout. */
(function () {
  var slide = document.querySelector(".deck > .slide.active");
  var pasteboard = window.slopPasteboard;
  if (!slide || !pasteboard || window.parent === window) return;

  // Attributes the editor adds while it works; stripped from the markup it saves.
  var EDITOR_ATTRS = ["contenteditable", "data-slop-selected", "data-slop-hover", "data-slop-editing", "data-slop-typing"];
  var MOVED = "data-moved";
  var DRAG_THRESHOLD = 3;
  var DOUBLE_CLICK_MS = 400;
  var NUDGE_SAVE_MS = 500;
  var ROTATE_SNAP = 15;
  // Degrees of tilt per screen pixel dragged, and how far away the viewer of a tilt stands.
  var TILT_PER_PX = 0.5;
  var PERSPECTIVE = 1000;
  // The tilt the editor puts in front of an element's inline `transform`.
  var TILT = /^perspective\([\d.]+px\) rotateX\((-?[\d.]+)deg\) rotateY\((-?[\d.]+)deg\)\s*/;
  var MIN_SCALE = 0.1;
  var OVERFLOW_TOLERANCE = 2;

  var style = document.createElement("style");
  style.textContent =
    // The slide shows what runs past its edge.
    ".deck, .deck > .slide.active { overflow: visible !important; }" +
    ".deck > .slide.active, .deck > .slide.active * { -webkit-user-select: none; user-select: none; }" +
    "[data-slop-hover] { outline: 3px dashed rgba(59, 130, 246, 0.7) !important; outline-offset: 4px; }" +
    // The selection frame is drawn by the handle overlay, so it keeps its size on screen.
    "[data-slop-selected] { cursor: move !important; }" +
    "[data-slop-editing] { outline: 3px solid #3b82f6 !important; outline-offset: 4px; }" +
    "[data-slop-ui] { position: fixed; z-index: 2147483647; pointer-events: none; box-sizing: border-box; }" +
    "[data-slop-ui] * { position: absolute; box-sizing: border-box; }" +
    // The frame and its stretch handles lie in the element's plane, tilted with it.
    "[data-slop-ui] > [data-plane] { inset: 0; outline: 2px solid #3b82f6; }" +
    "[data-slop-ui] [data-handle] { pointer-events: auto; background: #fff;" +
    " border: 2px solid #3b82f6; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3); touch-action: none;" +
    " width: 16px; height: 16px; }" +
    "[data-slop-ui] [data-handle=scale] { border-radius: 3px; }" +
    "[data-slop-ui] [data-axis] { border-radius: 8px; }" +
    "[data-slop-ui] [data-axis=x] { height: 24px; }" +
    "[data-slop-ui] [data-axis=y] { width: 24px; }" +
    "[data-slop-ui][data-narrow] [data-axis=y], [data-slop-ui][data-flat] [data-axis=x] { display: none; }" +
    "[data-slop-ui] > [data-handle=rotate] { border-radius: 50%; width: 18px; height: 18px; margin: 0 0 0 -9px;" +
    " left: 50%; top: -48px; cursor: grab; }" +
    "[data-slop-ui] > [data-handle=tilt] { width: 14px; height: 14px; margin: 2px 0 0 18px; left: 50%; top: -48px;" +
    " transform: rotate(45deg); cursor: move; }" +
    "[data-slop-overflow] { position: fixed; inset: 0; z-index: 2147483646; pointer-events: none; }" +
    "[data-slop-overflow][data-quiet] { display: none; }" +
    "[data-slop-overflow] > * { position: absolute; box-sizing: border-box; }" +
    "[data-slop-overflow] > [data-wire] { outline: 3px dashed #ef4444; outline-offset: -1px; background: rgba(239, 68, 68, 0.08); }" +
    // The slide's frame marks edit mode, and dims everything outside the slide.
    "[data-slop-frame] { outline: 2px solid var(--slop-accent, #3b82f6); outline-offset: 4px; box-shadow: 0 0 0 9999px" +
    " color-mix(in srgb, var(--slop-canvas, #f4f4f5) 55%, transparent); }" +
    "[data-slop-ui] > [data-stem] { pointer-events: none; width: 2px; height: 32px; margin: 0 0 0 -1px;" +
    " left: 50%; top: -32px; border: 0; box-shadow: none; background: #3b82f6; }" +
    // While typing, overlays (like invisible hover zones) let clicks through to the text.
    ".deck > .slide.active[data-slop-typing] * { pointer-events: none !important; }" +
    ".deck > .slide.active[data-slop-typing] [data-slop-editing]," +
    " .deck > .slide.active[data-slop-typing] [data-slop-editing] * { pointer-events: auto !important; }" +
    ".deck > .slide.active [data-slop-editing], .deck > .slide.active [data-slop-editing] * {" +
    " -webkit-user-select: text; user-select: text; cursor: text !important; }";
  document.head.appendChild(style);

  // Scale, rotate, and tilt handles, outside the slide so they never end up in its markup. They
  // sit just outside the frame, so even small text stays clickable (and double-clickable) inside
  // it. The rotate and tilt handles stay flat, so they can be grabbed however far it is tilted.
  var ui = document.createElement("div");
  ui.setAttribute("data-slop-ui", "");
  ui.style.display = "none";
  ui.innerHTML =
    '<div data-stem></div><div data-handle="rotate" title="Rotate (Shift snaps to 15°, double-click resets)"></div>' +
    '<div data-handle="tilt" title="Tilt in 3D (Shift snaps to 15°, double-click resets)"></div><div data-plane>' +
    [[-1, -1], [0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0]]
      .map(function (d) {
        var cursor = d[0] === 0 ? "ns" : d[1] === 0 ? "ew" : d[0] === d[1] ? "nwse" : "nesw";
        var axis = d[0] === 0 ? ' data-axis="y"' : d[1] === 0 ? ' data-axis="x"' : "";
        var w = d[0] === 0 ? 24 : 16;
        var h = d[1] === 0 ? 24 : 16;
        var outside = function (dir, size) {
          return dir < 0 ? -size - 2 : dir > 0 ? 2 : -size / 2;
        };
        return (
          '<div data-handle="scale" data-dir="' + d[0] + " " + d[1] + '"' + axis +
          ' title="Stretch (Shift keeps the aspect ratio, Alt scales from the center, double-click resets)"' +
          ' style="left: ' + (d[0] + 1) * 50 + "%; top: " + (d[1] + 1) * 50 + "%; margin: " + outside(d[1], h) + "px 0 0 " +
          outside(d[0], w) + "px; cursor: " + cursor + '-resize"></div>'
        );
      })
      .join("") +
    "</div>";
  document.body.appendChild(ui);
  var plane = ui.querySelector("[data-plane]");

  var wires = document.createElement("div");
  wires.setAttribute("data-slop-overflow", "");
  document.body.appendChild(wires);

  var saved = serialize();
  var selected = null;
  var hovered = null;
  var editing = null;
  var drag = null;
  var nudgeTimer = null;
  var lastDown = { el: null, at: 0 };
  var overflowFrame = 0;
  var reportedOverflow = null;
  // The handles keep to the selection as the view pans and zooms.
  pasteboard.onChange(placeHandles);
  // Space and 0 are text while typing, and Space+drag selects it.
  pasteboard.busy = function () {
    return !!editing;
  };

  /** The slide's markup without anything the editor or the player added. */
  function serialize() {
    var copy = slide.cloneNode(true);
    copy.classList.remove("active");
    [copy].concat(Array.prototype.slice.call(copy.querySelectorAll("*"))).forEach(function (el) {
      EDITOR_ATTRS.forEach(function (name) {
        el.removeAttribute(name);
      });
    });
    return copy.outerHTML;
  }

  function commit() {
    clearTimeout(nudgeTimer);
    nudgeTimer = null;
    var markup = serialize();
    if (markup === saved) return;
    saved = markup;
    window.parent.postMessage(
      { type: "slop:edit-commit", slide: slide.id, markup: markup, select: pathOf(selected) },
      "*",
    );
  }

  /** Child-index path from the slide to `el`, to select it again after the preview reloads. */
  function pathOf(el) {
    if (!el || !slide.contains(el) || el === slide) return null;
    var path = [];
    for (var node = el; node !== slide; node = node.parentElement) {
      path.unshift(Array.prototype.indexOf.call(node.parentElement.children, node));
    }
    return path;
  }

  function atPath(path) {
    var el = slide;
    for (var i = 0; path && i < path.length && el; i++) el = el.children[path[i]];
    return el && el !== slide ? el : null;
  }

  /** Whether `el` shows anything: text, media, a background, a border, or a shadow. */
  function paints(el) {
    var style = getComputedStyle(el);
    if (style.visibility === "hidden" || style.opacity === "0") return false;
    if (el.textContent.trim() || /^(img|svg|video|canvas|picture|iframe|object|embed)$/i.test(el.tagName)) return true;
    var bg = style.backgroundColor;
    var clear = !bg || bg === "transparent" || /rgba\(.*,\s*0\)$/.test(bg);
    var border = ["Top", "Right", "Bottom", "Left"].some(function (side) {
      return parseFloat(style["border" + side + "Width"]) > 0 && style["border" + side + "Style"] !== "none";
    });
    return !clear || (style.backgroundImage || "none") !== "none" || border || (style.boxShadow || "none") !== "none";
  }

  /** The element to select for `el`: not inline text runs, whole SVG drawings. */
  function selectable(el) {
    if (!el || el.nodeType !== 1 || !slide.contains(el) || el === slide) return null;
    var svg = el.closest("svg");
    while (svg && svg.parentElement && svg.parentElement.closest("svg")) svg = svg.parentElement.closest("svg");
    if (svg && slide.contains(svg)) el = svg;
    while (el.parentElement !== slide && getComputedStyle(el).display === "inline") el = el.parentElement;
    return el;
  }

  /**
   * What a pointer at (x, y) on `target` picks: the top-most element there that shows
   * something, so invisible overlays (hover zones, empty wrappers) do not get in the way.
   */
  function pickable(target, x, y) {
    var stack = document.elementsFromPoint && x !== undefined ? document.elementsFromPoint(x, y) : [];
    if (!stack.length) {
      for (var node = target && target.nodeType === 1 ? target : target && target.parentElement; node; node = node.parentElement) {
        stack.push(node);
      }
    }
    for (var i = 0; i < stack.length; i++) {
      if (stack[i] === slide) return null;
      var el = selectable(stack[i]);
      if (el && paints(el)) return el;
    }
    return null;
  }

  function mark(el, name, on) {
    if (!el) return;
    if (on) el.setAttribute(name, "");
    else el.removeAttribute(name);
  }

  function select(el) {
    if (el === selected) return;
    var had = !!selected;
    mark(selected, "data-slop-selected", false);
    selected = el;
    mark(selected, "data-slop-selected", true);
    placeHandles();
    if (had !== !!el) window.parent.postMessage({ type: "slop:edit-selection", slide: slide.id, selected: !!el }, "*");
  }

  function hover(el) {
    if (el === hovered) return;
    mark(hovered, "data-slop-hover", false);
    hovered = el;
    mark(hovered, "data-slop-hover", true);
  }

  /** The element's inline `translate` offset in slide pixels. */
  function offsetOf(el) {
    var parts = (el.style.translate || "").match(/-?[\d.]+(?=px)/g) || [];
    return { x: parseFloat(parts[0] || "0"), y: parseFloat(parts[1] || "0") };
  }

  function moveTo(el, x, y) {
    x = Math.round(x);
    y = Math.round(y);
    setInline(el, "translate", x === 0 && y === 0 ? null : x + "px " + y + "px");
  }

  /** The element's rotation in degrees and its (uniform) scale, inline or from the stylesheet. */
  function angleOf(el) {
    var match = (el.style.rotate || getComputedStyle(el).rotate || "").match(/^(-?[\d.]+)deg$/);
    return match ? parseFloat(match[1]) : 0;
  }
  function scaleOf(el) {
    return parseScale(el.style.scale || getComputedStyle(el).scale);
  }
  function parseScale(value) {
    var parts = String(value || "").trim().split(/\s+/).map(parseFloat);
    var x = isFinite(parts[0]) && parts[0] > 0 ? parts[0] : 1;
    var y = isFinite(parts[1]) && parts[1] > 0 ? parts[1] : x;
    return { x: x, y: y };
  }

  /** What the stylesheet alone gives `el` for `prop`. */
  function sheetValue(el, prop) {
    var inline = el.style[prop];
    if (!inline) return getComputedStyle(el)[prop];
    var priority = el.style.getPropertyPriority(prop);
    el.style[prop] = "";
    var value = getComputedStyle(el)[prop];
    el.style.setProperty(prop, inline, priority);
    return value;
  }

  /** The element's 3D tilt in degrees: around its horizontal (x) and vertical (y) axis. */
  function tiltOf(el) {
    var match = (el.style.transform || "").match(TILT);
    return match ? { x: parseFloat(match[1]), y: parseFloat(match[2]) } : { x: 0, y: 0 };
  }

  /**
   * Tilts `el` in depth with an inline `transform` (there is no separate property for it), seen
   * with perspective. It is `!important` so entrance animations, which animate `transform`,
   * cannot undo it, and keeps whatever `transform` the element already had after the tilt.
   */
  function tiltTo(el, x, y) {
    x = wrapAngle(x);
    y = wrapAngle(y);
    var inline = el.style.transform;
    var sheet = sheetValue(el, "transform");
    // An entrance animation that ended leaves the identity matrix.
    sheet = !sheet || sheet === "none" || sheet === "matrix(1, 0, 0, 1, 0, 0)" ? "" : sheet;
    var rest = inline ? inline.replace(TILT, "") : sheet;
    if (x || y) {
      var tilt = "perspective(" + PERSPECTIVE + "px) rotateX(" + x + "deg) rotateY(" + y + "deg)";
      setInline(el, "transform", rest ? tilt + " " + rest : tilt, "important");
    } else {
      setInline(el, "transform", rest && rest !== sheet ? rest : null);
    }
  }

  /** Whole degrees in (-180, 180]. */
  function wrapAngle(degrees) {
    degrees = Math.round(degrees) % 360;
    if (degrees > 180) degrees -= 360;
    if (degrees <= -180) degrees += 360;
    return degrees;
  }

  function rotateTo(el, degrees) {
    degrees = wrapAngle(degrees);
    var sheet = sheetValue(el, "rotate");
    var same = degrees === 0 ? !sheet || sheet === "none" || sheet === "0deg" : sheet === degrees + "deg";
    setInline(el, "rotate", same ? null : degrees + "deg");
  }

  function scaleTo(el, x, y) {
    x = Math.max(MIN_SCALE, Math.round(x * 1000) / 1000);
    y = Math.max(MIN_SCALE, Math.round(y * 1000) / 1000);
    var sheet = parseScale(sheetValue(el, "scale"));
    setInline(el, "scale", sheet.x === x && sheet.y === y ? null : x === y ? String(x) : x + " " + y);
  }

  /** Sets (or with null, drops) an inline transform; `data-moved` marks any hand transform. */
  function setInline(el, prop, value, priority) {
    if (value === null) el.style.removeProperty(prop);
    else el.style.setProperty(prop, value, priority || "");
    if (!el.getAttribute("style")) el.removeAttribute("style");
    mark(el, MOVED, !!(el.style.translate || el.style.rotate || el.style.scale || TILT.test(el.style.transform || "")));
    placeHandles();
  }

  /** The center of the element's box on screen; a tilt in perspective would shift its bounds. */
  function centerOf(el) {
    var inline = el.style.transform || "";
    var tilted = TILT.test(inline);
    var priority = el.style.getPropertyPriority("transform");
    if (tilted) el.style.setProperty("transform", inline.replace(TILT, "") || "none", priority);
    var rect = el.getBoundingClientRect();
    if (tilted) el.style.setProperty("transform", inline, priority);
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }

  /** The element's untransformed size, in slide pixels. */
  function sizeOf(el) {
    var computed = getComputedStyle(el);
    return {
      w: el.offsetWidth || parseFloat(computed.width) || 0,
      h: el.offsetHeight || parseFloat(computed.height) || 0,
    };
  }

  /** Screen pixels per slide pixel. */
  function zoom() {
    var value = slide.getBoundingClientRect().width / slide.offsetWidth;
    return isFinite(value) && value > 0 ? value : 1;
  }

  /** Lays the handles over the selection's box, turned with it; they keep their size on screen. */
  function placeHandles() {
    scheduleOverflow();
    if (!selected || editing || !slide.contains(selected)) {
      ui.style.display = "none";
      return;
    }
    var center = centerOf(selected);
    var size = sizeOf(selected);
    var scale = scaleOf(selected);
    var w = size.w * scale.x * zoom();
    var h = size.h * scale.y * zoom();
    ui.style.display = "";
    ui.style.left = center.x - w / 2 + "px";
    ui.style.top = center.y - h / 2 + "px";
    ui.style.width = w + "px";
    ui.style.height = h + "px";
    ui.style.transform = "rotate(" + angleOf(selected) + "deg)";
    // The frame tilts like the element; its perspective scales with it to look the same.
    var tilt = tiltOf(selected);
    var k = zoom() * Math.sqrt(scale.x * scale.y);
    plane.style.transform =
      tilt.x || tilt.y ? "perspective(" + PERSPECTIVE * k + "px) rotateX(" + tilt.x + "deg) rotateY(" + tilt.y + "deg)" : "";
    // Edge handles only where there is room for them between the corners.
    mark(ui, "data-narrow", w < 40);
    mark(ui, "data-flat", h < 40);
  }

  ui.addEventListener("pointerdown", function (event) {
    var kind = event.target.getAttribute && event.target.getAttribute("data-handle");
    if (event.button !== 0 || !kind || !selected) return;
    event.preventDefault();
    event.stopPropagation();
    if (nudgeTimer) commit();
    var dir = (event.target.getAttribute("data-dir") || "0 0").split(" ").map(Number);
    drag = {
      mode: kind,
      el: selected,
      center: centerOf(selected),
      start: { x: event.clientX, y: event.clientY },
      angle: angleOf(selected),
      tilt: tiltOf(selected),
      scale: scaleOf(selected),
      offset: offsetOf(selected),
      size: sizeOf(selected),
      zoom: zoom(),
      dir: { x: dir[0], y: dir[1] },
      moved: false,
    };
    if (event.target.setPointerCapture) {
      try {
        event.target.setPointerCapture(event.pointerId);
      } catch (e) {
        // Synthetic events have no active pointer to capture.
      }
    }
  });
  ui.addEventListener("dblclick", function (event) {
    var kind = event.target.getAttribute && event.target.getAttribute("data-handle");
    event.stopPropagation();
    if (!kind || !selected) return;
    if (kind === "rotate") rotateTo(selected, 0);
    else if (kind === "tilt") tiltTo(selected, 0, 0);
    else scaleTo(selected, 1, 1);
    commit();
  });

  /** Turns the screen vector (x, y) by `degrees`. */
  function turn(x, y, degrees) {
    var a = (degrees * Math.PI) / 180;
    return { x: x * Math.cos(a) - y * Math.sin(a), y: x * Math.sin(a) + y * Math.cos(a) };
  }

  /** Applies a handle drag to the pointer at `event`. */
  function transformTo(event) {
    if (drag.mode === "rotate") {
      var c = drag.center;
      var start = Math.atan2(drag.start.y - c.y, drag.start.x - c.x);
      var degrees = drag.angle + ((Math.atan2(event.clientY - c.y, event.clientX - c.x) - start) * 180) / Math.PI;
      rotateTo(drag.el, event.shiftKey ? Math.round(degrees / ROTATE_SNAP) * ROTATE_SNAP : degrees);
      return;
    }
    if (drag.mode === "tilt") {
      // Like rolling a ball: the side the pointer pulls towards turns away. Dragging follows
      // the element's own axes, however it is rotated.
      var pulled = turn(event.clientX - drag.start.x, event.clientY - drag.start.y, -drag.angle);
      var snap = function (deg) {
        return event.shiftKey ? Math.round(deg / ROTATE_SNAP) * ROTATE_SNAP : deg;
      };
      tiltTo(drag.el, snap(drag.tilt.x - pulled.y * TILT_PER_PX), snap(drag.tilt.y + pulled.x * TILT_PER_PX));
      return;
    }
    stretchTo(event.clientX - drag.start.x, event.clientY - drag.start.y, event.shiftKey, event.altKey);
  }

  /**
   * Stretches along the dragged handle's axes, working in the element's own (turned) frame
   * in slide pixels: the handle follows the pointer and the opposite side stays put, or with
   * `fromCenter` the center does. `keepRatio` scales both axes by the same factor.
   */
  function stretchTo(dx, dy, keepRatio, fromCenter) {
    var d = drag.dir;
    var w = drag.size.w;
    var h = drag.size.h;
    var s0 = drag.scale;
    var moved = turn(dx / drag.zoom, dy / drag.zoom, -drag.angle);
    // The dragged handle's new spot, relative to the element's center before the drag.
    var hx = (d.x * w * s0.x) / 2 + moved.x;
    var hy = (d.y * h * s0.y) / 2 + moved.y;
    var reach = fromCenter ? 2 : 1;
    var sx = d.x && w ? (reach * d.x * (hx + (fromCenter ? 0 : (d.x * w * s0.x) / 2))) / w : s0.x;
    var sy = d.y && h ? (reach * d.y * (hy + (fromCenter ? 0 : (d.y * h * s0.y) / 2))) / h : s0.y;
    if (keepRatio) {
      var fx = sx / s0.x;
      var fy = sy / s0.y;
      var f = !d.x ? fy : !d.y ? fx : Math.abs(fx - 1) > Math.abs(fy - 1) ? fx : fy;
      sx = s0.x * f;
      sy = s0.y * f;
    }
    sx = Math.max(MIN_SCALE, sx);
    sy = Math.max(MIN_SCALE, sy);
    // Where the center goes so the opposite side stays put (only along the dragged axes).
    var cx = fromCenter || !d.x ? 0 : (d.x * w * (sx - s0.x)) / 2;
    var cy = fromCenter || !d.y ? 0 : (d.y * h * (sy - s0.y)) / 2;
    var shift = turn(cx, cy, drag.angle);
    scaleTo(drag.el, sx, sy);
    moveTo(drag.el, drag.offset.x + shift.x, drag.offset.y + shift.y);
  }

  /** Which of the slide's edges `rect` runs past, with by how many slide pixels. */
  function pastEdges(rect, bounds, k) {
    var past = [];
    var add = function (side, by) {
      by /= k;
      if (by > OVERFLOW_TOLERANCE) past.push({ side: side, by: Math.round(by) });
    };
    add("top", bounds.top - rect.top);
    add("right", rect.right - bounds.right);
    add("bottom", rect.bottom - bounds.bottom);
    add("left", bounds.left - rect.left);
    return past;
  }

  /** Whether `el` itself holds text or media, as opposed to decoration like a glow that bleeds off the edge. */
  function hasContent(el) {
    if (el.closest("[aria-hidden=true]")) return false;
    if (/^(img|svg|video|canvas|picture|iframe|object|embed)$/i.test(el.tagName)) return true;
    return Array.prototype.some.call(el.childNodes, function (node) {
      return node.nodeType === 3 && node.textContent.trim();
    });
  }

  /** Elements that run past the slide's edge or are cut off by their own box; the innermost ones. */
  function findOverflow() {
    var bounds = slide.getBoundingClientRect();
    var k = zoom();
    var hits = [];
    Array.prototype.forEach.call(slide.querySelectorAll("*"), function (el) {
      var svg = el.closest("svg");
      if ((svg && svg !== el) || el.closest(".notes")) return;
      var rect = el.getBoundingClientRect();
      if (!rect.width && !rect.height) return;
      var past = hasContent(el) && paints(el) ? pastEdges(rect, bounds, k) : [];
      var style = getComputedStyle(el);
      var clips = style.overflowX !== "visible" || style.overflowY !== "visible";
      var cut = clips && (el.scrollWidth > el.clientWidth + OVERFLOW_TOLERANCE || el.scrollHeight > el.clientHeight + OVERFLOW_TOLERANCE);
      if (past.length || cut) hits.push({ el: el, rect: rect, past: past, cut: cut });
    });
    return hits.filter(function (hit) {
      return !hits.some(function (other) {
        return other !== hit && hit.el.contains(other.el);
      });
    });
  }

  function describeOverflow(hit) {
    var text = hit.el.textContent.replace(/\s+/g, " ").trim();
    var name = "<" + hit.el.tagName.toLowerCase() + ">" + (text ? ' "' + text.slice(0, 40) + (text.length > 40 ? "…" : "") + '"' : "");
    var what = hit.past.map(function (p) {
      return "runs past the " + p.side + " edge by " + p.by + "px";
    });
    if (hit.cut) what.push("is cut off by its own box");
    return name + " " + what.join(" and ");
  }

  function drawOverflow(hits) {
    wires.textContent = "";
    hits.forEach(function (hit) {
      var left = Math.max(hit.rect.left, 0);
      var top = Math.max(hit.rect.top, 0);
      var right = Math.min(hit.rect.right, window.innerWidth);
      var bottom = Math.min(hit.rect.bottom, window.innerHeight);
      var wire = document.createElement("div");
      wire.setAttribute("data-wire", "");
      wire.style.cssText = "left:" + left + "px;top:" + top + "px;width:" + Math.max(0, right - left) + "px;height:" + Math.max(0, bottom - top) + "px";
      wires.appendChild(wire);
    });
  }

  function refreshOverflow() {
    overflowFrame = 0;
    var hits = findOverflow();
    drawOverflow(hits);
    var items = hits.map(describeOverflow);
    var key = JSON.stringify(items);
    if (key === reportedOverflow) return;
    reportedOverflow = key;
    window.parent.postMessage({ type: "slop:edit-overflow", slide: slide.id, items: items }, "*");
  }

  /** Re-checks for overflow once per frame, however many changes came in. */
  function scheduleOverflow() {
    if (!overflowFrame) overflowFrame = window.requestAnimationFrame(refreshOverflow);
  }

  function startEditing(el, x, y) {
    if (!el || editing || /^(img|svg|video|canvas|iframe|hr|br)$/i.test(el.tagName)) return;
    if (!el.textContent.trim()) return;
    select(el);
    editing = el;
    el.setAttribute("contenteditable", "true");
    el.setAttribute("data-slop-editing", "");
    slide.setAttribute("data-slop-typing", "");
    el.focus();
    placeHandles();
    var range = x !== undefined && document.caretRangeFromPoint ? document.caretRangeFromPoint(x, y) : null;
    if (!range || !el.contains(range.startContainer)) {
      range = document.createRange();
      range.selectNodeContents(el);
    }
    var selection = window.getSelection();
    if (selection) {
      selection.removeAllRanges();
      selection.addRange(range);
    }
  }

  function finishEditing() {
    if (!editing) return;
    editing.removeAttribute("contenteditable");
    editing.removeAttribute("data-slop-editing");
    slide.removeAttribute("data-slop-typing");
    editing = null;
    var selection = window.getSelection();
    if (selection) selection.removeAllRanges();
    placeHandles();
    commit();
  }

  function removeSelected() {
    var el = selected;
    if (!el) return;
    finishEditing();
    select(null);
    if (el.parentElement) el.parentElement.removeChild(el);
    commit();
  }

  document.addEventListener("pointerdown", function (event) {
    // The pasteboard takes the middle button and Space+drag before this sees them.
    if (event.button !== 0) return;
    wires.removeAttribute("data-quiet");
    if (editing && editing.contains(event.target)) return;
    finishEditing();
    if (nudgeTimer) commit();
    var el = pickable(event.target, event.clientX, event.clientY);
    var now = Date.now();
    var double = el && el === lastDown.el && now - lastDown.at < DOUBLE_CLICK_MS;
    lastDown = { el: el, at: double ? 0 : now };
    select(el);
    if (!el) {
      pasteboard.startPan(event);
      return;
    }
    if (double) {
      // Otherwise the browser moves focus to what was clicked, which may be an overlay.
      event.preventDefault();
      startEditing(el, event.clientX, event.clientY);
      return;
    }
    event.preventDefault();
    var start = offsetOf(el);
    drag = { mode: "move", el: el, x: event.clientX, y: event.clientY, from: start, zoom: zoom(), moved: false };
    if (el.setPointerCapture) {
      try {
        el.setPointerCapture(event.pointerId);
      } catch (e) {
        // Synthetic events have no active pointer to capture.
      }
    }
  });

  document.addEventListener("pointermove", function (event) {
    if (!drag) {
      var onHandle = ui.contains(event.target);
      hover(editing || onHandle ? null : pickable(event.target, event.clientX, event.clientY));
      return;
    }
    if (drag.mode !== "move") {
      drag.moved = true;
      transformTo(event);
      return;
    }
    var dx = event.clientX - drag.x;
    var dy = event.clientY - drag.y;
    if (!drag.moved && Math.abs(dx) < DRAG_THRESHOLD && Math.abs(dy) < DRAG_THRESHOLD) return;
    drag.moved = true;
    lastDown = { el: null, at: 0 };
    hover(null);
    moveTo(drag.el, drag.from.x + dx / drag.zoom, drag.from.y + dy / drag.zoom);
  });

  function endDrag() {
    if (!drag) return;
    var moved = drag.moved;
    drag = null;
    if (moved) commit();
  }
  document.addEventListener("pointerup", endDrag);
  document.addEventListener("pointercancel", endDrag);
  document.documentElement.addEventListener("pointerleave", function () {
    hover(null);
  });
  window.addEventListener("resize", placeHandles);
  window.addEventListener("load", scheduleOverflow);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(scheduleOverflow);
  scheduleOverflow();
  document.addEventListener("input", placeHandles);
  // Clicking elsewhere in the app ends text editing, keeping the text.
  window.addEventListener("blur", finishEditing);
  document.addEventListener("dblclick", function (event) {
    var el = pickable(event.target, event.clientX, event.clientY);
    if (el && !editing) startEditing(el, event.clientX, event.clientY);
  });

  // Pasted text arrives as plain text, so editing never brings in foreign markup.
  document.addEventListener("paste", function (event) {
    if (!editing) return;
    event.preventDefault();
    var text = event.clipboardData ? event.clipboardData.getData("text/plain") : "";
    document.execCommand("insertText", false, text);
  });
  document.addEventListener("drop", function (event) {
    if (editing) event.preventDefault();
  });

  // Capture phase, so keys the editor handles never reach the player (which forwards keys
  // to the app for slide navigation).
  window.addEventListener(
    "keydown",
    function (event) {
      if (editing) {
        if (event.key === "Escape" || (event.key === "Enter" && !event.shiftKey)) {
          event.preventDefault();
          finishEditing();
        } else if (event.key === "Enter") {
          event.preventDefault();
          document.execCommand("insertLineBreak");
        }
        event.stopImmediatePropagation();
        return;
      }
      if (!selected) return;
      if (event.metaKey || event.ctrlKey) {
        // Undo and other shortcuts go to the app, after any pending nudge is saved.
        if (nudgeTimer) commit();
        return;
      }
      var step = event.shiftKey ? 10 : 1;
      var nudge = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[event.key];
      if (nudge) {
        var at = offsetOf(selected);
        moveTo(selected, at.x + nudge[0], at.y + nudge[1]);
        clearTimeout(nudgeTimer);
        nudgeTimer = setTimeout(commit, NUDGE_SAVE_MS);
      } else if (event.key === "Escape") {
        select(selected.parentElement === slide ? null : selected.parentElement);
      } else if (event.key === "Enter") {
        startEditing(selected);
      } else if (event.key === "Backspace" || event.key === "Delete") {
        removeSelected();
      } else {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
    },
    true,
  );

  window.addEventListener("message", function (event) {
    var data = event.data;
    if (event.source !== window.parent || !data) return;
    if (data.type === "slop:edit-select") {
      finishEditing();
      select(atPath(data.path));
      // Clearing the selection for a screenshot hides the wires too, until the next interaction.
      if (data.quiet) mark(wires, "data-quiet", true);
    } else if (data.type === "slop:edit-delete") {
      removeSelected();
    }
  });
})();
