/* SlopSlide player. Keys: ←/→, space, PageUp/PageDown, Home/End, F for full screen, R for review marks. */
(function () {
  var root = document.documentElement;
  // The runtime-css block sets this before the body renders; without it the stylesheet lists
  // the slides for viewers that run no JavaScript.
  root.setAttribute("data-slop-player", "");
  var params = new URLSearchParams(location.search);
  var embed = params.has("embed");
  var framed = window.parent !== window;
  if (params.has("static")) root.setAttribute("data-slop-static", "");

  var deck = document.querySelector(".deck");
  var all = Array.prototype.filter.call(document.querySelectorAll(".slide"), function (el) {
    return !el.parentElement || !el.parentElement.closest(".slide");
  });
  // Hidden slides (data-hidden) are skipped by the show; the editor still embeds them.
  var slides = embed
    ? all
    : all.filter(function (el) {
        return !el.hasAttribute("data-hidden");
      });
  if (!deck || slides.length === 0) return;
  var current = -1;

  function fit() {
    var scale = Math.min(window.innerWidth / 1920, window.innerHeight / 1080);
    var x = (window.innerWidth - 1920 * scale) / 2;
    var y = (window.innerHeight - 1080 * scale) / 2;
    deck.style.transform = "translate(" + x + "px," + y + "px) scale(" + scale + ")";
  }

  function indexFor(ref) {
    if (!ref) return 0;
    // A hidden slide's id lands on the next shown slide (or the last one).
    for (var i = 0; i < all.length; i++) {
      if (all[i].id !== ref) continue;
      for (var j = i; j < all.length; j++) {
        var index = slides.indexOf(all[j]);
        if (index >= 0) return index;
      }
      return slides.length - 1;
    }
    // The editor addresses slides without an id by position, as "#2".
    var n = parseInt(ref.replace(/^#/, ""), 10);
    return isNaN(n) ? 0 : Math.min(slides.length - 1, Math.max(0, n - 1));
  }

  function show(index) {
    index = Math.min(slides.length - 1, Math.max(0, index));
    if (index === current) return;
    current = index;
    slides.forEach(function (el, i) {
      el.classList.toggle("active", i === index);
    });
    if (embed) return;
    var id = slides[index].id || String(index + 1);
    try {
      history.replaceState(null, "", "#" + id);
    } catch (e) {
      // Sandboxed frames and some file:// contexts refuse URL updates; navigation still works.
    }
    if (framed) window.parent.postMessage({ type: "slop:slide", id: slides[index].id || null, index: index }, "*");
  }

  // Review marks drawn in the editor (the slopslide:review block), hidden until R or ?review
  // shows them. The editor draws them itself, so embedded previews leave them out.
  var review = null;
  var reviewData = document.getElementById("slopslide-review");
  if (reviewData && !embed) {
    try {
      review = JSON.parse(reviewData.textContent);
    } catch (e) {
      review = null;
    }
  }
  var SVG = "http://www.w3.org/2000/svg";
  // Widths in screen pixels, as the editor draws them.
  var INK = { pen: { width: 4, opacity: 1 }, highlighter: { width: 28, opacity: 0.4 } };

  // SVG path data for a smooth curve through a stroke's points (fractions of the slide), as the
  // app draws it: from the first point to halfway to the next, then from halfway to halfway,
  // bent by each point between as a quadratic Bézier control point, to the last point.
  function smoothPath(points) {
    var xy = points.map(function (p) {
      return [p[0] * 1920, p[1] * 1080];
    });
    var at = function (x, y) {
      return Math.round(x * 10) / 10 + " " + Math.round(y * 10) / 10;
    };
    var halfway = function (i) {
      return at((xy[i][0] + xy[i + 1][0]) / 2, (xy[i][1] + xy[i + 1][1]) / 2);
    };
    var last = xy.length - 1;
    if (last < 2) {
      return xy
        .map(function (p, i) {
          return (i ? "L" : "M") + at(p[0], p[1]);
        })
        .join("");
    }
    var d = "M" + at(xy[0][0], xy[0][1]) + "L" + halfway(0);
    for (var i = 1; i < last; i++) {
      d += "Q" + at(xy[i][0], xy[i][1]) + " " + (i === last - 1 ? at(xy[last][0], xy[last][1]) : halfway(i));
    }
    return d;
  }

  function drawReview(slide, strokes) {
    var svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "slop-review");
    svg.setAttribute("viewBox", "0 0 1920 1080");
    svg.setAttribute("preserveAspectRatio", "none");
    svg.setAttribute("aria-hidden", "true");
    strokes.forEach(function (stroke) {
      var ink = stroke && INK[stroke.tool];
      var points = stroke && stroke.points;
      if (!ink || !Array.isArray(points) || points.length === 0) return;
      var d = smoothPath(points);
      // A tap never moves; a tiny step makes the round cap draw it as a dot.
      if (points.every(function (p) { return p[0] === points[0][0] && p[1] === points[0][1]; })) d += "l0.01 0";
      var path = document.createElementNS(SVG, "path");
      path.setAttribute("d", d);
      path.setAttribute("fill", "none");
      path.setAttribute("stroke", String(stroke.color));
      path.setAttribute("stroke-width", String(ink.width));
      path.setAttribute("stroke-opacity", String(ink.opacity));
      path.setAttribute("stroke-linecap", "round");
      path.setAttribute("stroke-linejoin", "round");
      path.setAttribute("vector-effect", "non-scaling-stroke");
      svg.appendChild(path);
    });
    slide.appendChild(svg);
  }

  /** Shows or hides the review marks; false when the deck has none. */
  function toggleReview(on) {
    if (!review || typeof review !== "object") return false;
    if (on) {
      all.forEach(function (slide) {
        var strokes = slide.id && review[slide.id];
        if (Array.isArray(strokes) && !slide.querySelector(":scope > .slop-review")) drawReview(slide, strokes);
      });
    }
    if (on) root.setAttribute("data-slop-review", "");
    else root.removeAttribute("data-slop-review");
    return true;
  }
  if (params.has("review")) toggleReview(true);

  window.addEventListener("resize", fit);
  fit();
  show(indexFor(embed ? params.get("slide") : decodeURIComponent(location.hash.slice(1))));

  function navigate(key) {
    if (key === "ArrowRight" || key === "ArrowDown" || key === "PageDown" || key === " ") show(current + 1);
    else if (key === "ArrowLeft" || key === "ArrowUp" || key === "PageUp") show(current - 1);
    else if (key === "Home") show(0);
    else if (key === "End") show(slides.length - 1);
    else if ((key === "f" || key === "F") && !framed) {
      if (document.fullscreenElement) document.exitFullscreen();
      else root.requestFullscreen && root.requestFullscreen();
    } else return false;
    return true;
  }

  window.addEventListener("keydown", function (event) {
    if (framed) {
      var mod = event.metaKey || event.ctrlKey;
      window.parent.postMessage({ type: "slop:key", key: event.key, mod: mod, shift: event.shiftKey }, "*");
    }
    if (embed) return;
    var plain = !event.metaKey && !event.ctrlKey && !event.altKey;
    if ((event.key === "r" || event.key === "R") && plain) {
      if (toggleReview(!root.hasAttribute("data-slop-review"))) event.preventDefault();
    } else if (navigate(event.key)) event.preventDefault();
  });

  if (embed) return;
  // The presenter forwards keys pressed while its drawing tools have focus.
  window.addEventListener("message", function (event) {
    if (framed && event.data && event.data.type === "slop:go") navigate(String(event.data.key));
  });
  window.addEventListener("hashchange", function () {
    show(indexFor(decodeURIComponent(location.hash.slice(1))));
  });
  document.addEventListener("click", function (event) {
    if (event.target.closest("a, button, input, select, textarea, video, [contenteditable]")) return;
    show(event.clientX < window.innerWidth / 4 ? current - 1 : current + 1);
  });
  var touchX = null;
  document.addEventListener("touchstart", function (event) {
    touchX = event.touches[0].clientX;
  });
  document.addEventListener("touchend", function (event) {
    if (touchX === null) return;
    var dx = event.changedTouches[0].clientX - touchX;
    if (Math.abs(dx) > 40) show(current + (dx < 0 ? 1 : -1));
    touchX = null;
  });
})();
