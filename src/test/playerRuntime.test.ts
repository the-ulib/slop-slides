/**
 * The player embedded in every deck.html (src-tauri/assets/runtime.js). It runs in the
 * editor's slide iframes, in the presenter, and in exported files opened in any browser.
 */
import { readFileSync } from "node:fs";
import { JSDOM, type DOMWindow } from "jsdom";
import { afterEach, describe, expect, it, vi } from "vitest";

import RUNTIME from "../../src-tauri/assets/runtime.js?raw";
import { strokePath, toPixels } from "../lib/ink";

const DECK = `<!DOCTYPE html><html><body>
<main class="deck">
  <section class="slide" id="intro"><button>Click me</button><section class="slide nested"></section></section>
  <section class="slide"><p>No id</p></section>
  <section class="slide" id="end"><a href="#x">link</a></section>
</main></body></html>`;

let doms: JSDOM[] = [];

afterEach(() => {
  doms.forEach((d) => d.window.close());
  doms = [];
});

interface PlayerOptions {
  /** Query string and hash, e.g. `?embed&slide=end` or `#2`. */
  at?: string;
  html?: string;
  /** Pretend to be inside an iframe; receives the player's postMessages. */
  parent?: { postMessage: (data: unknown, origin: string) => void };
  size?: [number, number];
}

function player({ at = "", html = DECK, parent, size = [1920, 1080] }: PlayerOptions = {}) {
  const dom = new JSDOM(html, {
    url: `https://example.test/deck.html${at}`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
    beforeParse(window) {
      Object.defineProperty(window, "innerWidth", { value: size[0], configurable: true });
      Object.defineProperty(window, "innerHeight", { value: size[1], configurable: true });
      if (parent) Object.defineProperty(window, "parent", { value: parent, configurable: true });
    },
  });
  doms.push(dom);
  dom.window.eval(RUNTIME);
  const { window } = dom;
  const doc = window.document;
  const topSlides = () => [...doc.querySelectorAll<HTMLElement>(".deck > .slide")];
  return {
    window,
    doc,
    active: () => topSlides().findIndex((s) => s.classList.contains("active")),
    activeCount: () => doc.querySelectorAll(".slide.active").length,
    key: (key: string) => {
      const event = new window.KeyboardEvent("keydown", { key, cancelable: true, bubbles: true });
      window.dispatchEvent(event);
      return event;
    },
    click: (target: Element, clientX: number) =>
      target.dispatchEvent(new window.MouseEvent("click", { clientX, bubbles: true })),
  };
}

const hashOf = (window: DOMWindow) => window.location.hash;

describe("player: standalone", () => {
  it("shows the first slide and records it in the URL", () => {
    const p = player();
    expect(p.active()).toBe(0);
    expect(p.activeCount()).toBe(1);
    expect(hashOf(p.window)).toBe("#intro");
  });

  it("starts at the slide named in the hash, by id or 1-based number", () => {
    expect(player({ at: "#end" }).active()).toBe(2);
    expect(player({ at: "#2" }).active()).toBe(1);
    expect(player({ at: "#99" }).active()).toBe(2);
    expect(player({ at: "#0" }).active()).toBe(0);
    expect(player({ at: "#unknown" }).active()).toBe(0);
  });

  it("decodes ids in the hash", () => {
    const html = DECK.replace(`id="end"`, `id="the end"`);
    expect(player({ html, at: "#the%20end" }).active()).toBe(2);
  });

  it("names slides without an id by their number", () => {
    const p = player();
    p.key("ArrowRight");
    expect(hashOf(p.window)).toBe("#2");
  });

  it.each([
    ["ArrowRight", 1],
    ["ArrowDown", 1],
    ["PageDown", 1],
    [" ", 1],
    ["End", 2],
  ])("%j moves forward", (key, expected) => {
    const p = player();
    const event = p.key(key);
    expect(p.active()).toBe(expected);
    expect(event.defaultPrevented).toBe(true);
  });

  it.each([["ArrowLeft"], ["ArrowUp"], ["PageUp"]])("%j moves back", (key) => {
    const p = player({ at: "#end" });
    p.key(key);
    expect(p.active()).toBe(1);
  });

  it("Home jumps to the start and navigation stops at either end", () => {
    const p = player({ at: "#end" });
    p.key("ArrowRight");
    expect(p.active()).toBe(2);
    p.key("Home");
    expect(p.active()).toBe(0);
    p.key("ArrowLeft");
    expect(p.active()).toBe(0);
  });

  it("leaves other keys to the page", () => {
    const p = player();
    expect(p.key("a").defaultPrevented).toBe(false);
    expect(p.active()).toBe(0);
  });

  it("F toggles full screen", () => {
    const p = player();
    const request = vi.fn();
    p.doc.documentElement.requestFullscreen = request;
    expect(p.key("f").defaultPrevented).toBe(true);
    expect(request).toHaveBeenCalledOnce();
    const exit = vi.fn();
    Object.defineProperty(p.doc, "fullscreenElement", { value: p.doc.documentElement, configurable: true });
    p.doc.exitFullscreen = exit;
    p.key("F");
    expect(exit).toHaveBeenCalledOnce();
  });

  it("follows hash changes", () => {
    const p = player();
    p.window.location.hash = "#end";
    p.window.dispatchEvent(new p.window.HashChangeEvent("hashchange"));
    expect(p.active()).toBe(2);
  });

  it("clicking the right of the slide advances, the left quarter goes back", () => {
    const p = player();
    const slide = p.doc.querySelector("#intro p, #intro") as Element;
    p.click(slide, 1500);
    expect(p.active()).toBe(1);
    p.click(p.doc.body, 100);
    expect(p.active()).toBe(0);
  });

  it("clicks on interactive elements do not navigate", () => {
    const p = player();
    p.click(p.doc.querySelector("button")!, 1500);
    expect(p.active()).toBe(0);
    p.key("End");
    p.click(p.doc.querySelector("a")!, 100);
    expect(p.active()).toBe(2);
  });

  it("swipes navigate", () => {
    const p = player();
    const touch = (type: string, clientX: number) => {
      const event = new p.window.Event(type, { bubbles: true }) as Event & Record<string, unknown>;
      event[type === "touchstart" ? "touches" : "changedTouches"] = [{ clientX }];
      p.doc.dispatchEvent(event);
    };
    touch("touchstart", 500);
    touch("touchend", 400);
    expect(p.active()).toBe(1);
    touch("touchstart", 400);
    touch("touchend", 480);
    expect(p.active()).toBe(0);
    touch("touchstart", 400);
    touch("touchend", 420);
    expect(p.active()).toBe(0); // short drags are not swipes
  });

  it("scales the 1920×1080 stage to fit and centers it", () => {
    const p = player({ size: [960, 1080] });
    const deck = p.doc.querySelector<HTMLElement>(".deck")!;
    expect(deck.style.transform).toBe("translate(0px,270px) scale(0.5)");
    Object.defineProperty(p.window, "innerWidth", { value: 3840 });
    Object.defineProperty(p.window, "innerHeight", { value: 1080 });
    p.window.dispatchEvent(new p.window.Event("resize"));
    expect(deck.style.transform).toBe("translate(960px,0px) scale(1)");
  });

  it("ignores slides nested inside slides", () => {
    const p = player();
    p.key("End");
    expect(p.active()).toBe(2);
    expect(p.doc.querySelector(".nested")!.classList.contains("active")).toBe(false);
  });

  it("does nothing on a page without slides", () => {
    const p = player({ html: `<main class="deck"></main>` });
    expect(p.doc.querySelector<HTMLElement>(".deck")!.style.transform).toBe("");
    expect(hashOf(p.window)).toBe("");
    const none = player({ html: `<section class="slide" id="a"></section>` });
    expect(none.activeCount()).toBe(0);
  });
});

describe("player: embedded in the editor", () => {
  it("shows the requested slide without touching the URL", () => {
    const p = player({ at: "?embed&slide=end" });
    expect(p.active()).toBe(2);
    expect(hashOf(p.window)).toBe("");
  });

  it("finds unnamed slides by the positional ids the editor gives them", () => {
    // deck.rs `load` names a slide without an id "#<n>"; slideUrl passes that through.
    expect(player({ at: "?embed&slide=%232" }).active()).toBe(1);
    expect(player({ at: "?embed&slide=2" }).active()).toBe(1);
    expect(player({ at: "?embed&slide=%23" }).active()).toBe(0);
  });

  it("does not navigate on its own, but forwards keys to the editor", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ at: "?embed&slide=intro", parent });
    const event = p.key("ArrowRight");
    expect(p.active()).toBe(0);
    expect(event.defaultPrevented).toBe(false);
    expect(parent.postMessage).toHaveBeenCalledWith({ type: "slop:key", key: "ArrowRight", mod: false, shift: false }, "*");
    p.click(p.doc.body, 1500);
    expect(p.active()).toBe(0);
  });

  it("marks thumbnails as static so animations show their final frame", () => {
    expect(player({ at: "?embed&slide=intro&static" }).doc.documentElement.hasAttribute("data-slop-static")).toBe(true);
    expect(player({ at: "?embed&slide=intro" }).doc.documentElement.hasAttribute("data-slop-static")).toBe(false);
  });
});

describe("player: in the presenter", () => {
  it("reports each slide shown to the editor", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ at: "#intro", parent });
    expect(parent.postMessage).toHaveBeenLastCalledWith({ type: "slop:slide", id: "intro", index: 0 }, "*");
    p.key("ArrowRight");
    expect(parent.postMessage).toHaveBeenCalledWith({ type: "slop:key", key: "ArrowRight", mod: false, shift: false }, "*");
    expect(parent.postMessage).toHaveBeenLastCalledWith({ type: "slop:slide", id: null, index: 1 }, "*");
    expect(p.active()).toBe(1);
  });

  it("tells the presenter when a modifier is held", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ parent });
    p.window.dispatchEvent(new p.window.KeyboardEvent("keydown", { key: "z", metaKey: true }));
    expect(parent.postMessage).toHaveBeenLastCalledWith({ type: "slop:key", key: "z", mod: true, shift: false }, "*");
    p.window.dispatchEvent(new p.window.KeyboardEvent("keydown", { key: "z", ctrlKey: true }));
    expect(parent.postMessage).toHaveBeenLastCalledWith({ type: "slop:key", key: "z", mod: true, shift: false }, "*");
    p.window.dispatchEvent(new p.window.KeyboardEvent("keydown", { key: "Z", metaKey: true, shiftKey: true }));
    expect(parent.postMessage).toHaveBeenLastCalledWith({ type: "slop:key", key: "Z", mod: true, shift: true }, "*");
  });

  it("navigates on keys the presenter forwards while its drawing tools have focus", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ parent });
    const go = (key: unknown) =>
      p.window.dispatchEvent(new p.window.MessageEvent("message", { data: { type: "slop:go", key } }));
    go("ArrowRight");
    expect(p.active()).toBe(1);
    go("End");
    expect(p.active()).toBe(2);
    go("ArrowLeft");
    expect(p.active()).toBe(1);
    go("x");
    p.window.dispatchEvent(new p.window.MessageEvent("message", { data: "ArrowRight" }));
    p.window.dispatchEvent(new p.window.MessageEvent("message", { data: null }));
    expect(p.active()).toBe(1);
  });

  it("ignores forwarded keys when not in the presenter", () => {
    const go = (p: ReturnType<typeof player>) =>
      p.window.dispatchEvent(new p.window.MessageEvent("message", { data: { type: "slop:go", key: "End" } }));
    const standalone = player();
    go(standalone);
    expect(standalone.active()).toBe(0);
    const embedded = player({ at: "?embed&slide=intro", parent: { postMessage: vi.fn() } });
    go(embedded);
    expect(embedded.active()).toBe(0);
  });

  it("leaves full screen to the app", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ parent });
    const request = vi.fn();
    p.doc.documentElement.requestFullscreen = request;
    expect(p.key("f").defaultPrevented).toBe(false);
    expect(request).not.toHaveBeenCalled();
  });

  it("does not report a slide that is already showing", () => {
    const parent = { postMessage: vi.fn() };
    const p = player({ parent });
    parent.postMessage.mockClear();
    p.key("Home");
    expect(parent.postMessage.mock.calls.filter(([d]) => (d as { type: string }).type === "slop:slide")).toEqual([]);
  });
});

describe("player: hidden slides", () => {
  const HIDDEN = `<!DOCTYPE html><html><body>
<main class="deck">
  <section class="slide" id="a"></section>
  <section class="slide" id="b" data-hidden></section>
  <section class="slide" id="c"></section>
  <section class="slide" id="d" data-hidden></section>
</main></body></html>`;
  const play = (at: string, parent?: PlayerOptions["parent"]) => {
    const p = player({ at, html: HIDDEN, parent });
    return { ...p, activeId: () => p.doc.querySelector(".slide.active")?.id };
  };

  it("skips hidden slides when navigating", () => {
    const p = play("#a");
    expect(p.activeId()).toBe("a");
    p.key("ArrowRight");
    expect(p.activeId()).toBe("c");
    p.key("ArrowRight");
    expect(p.activeId()).toBe("c");
    p.key("ArrowLeft");
    expect(p.activeId()).toBe("a");
    p.key("End");
    expect(p.activeId()).toBe("c");
  });

  it("starting on a hidden slide lands on the next shown one, or the last", () => {
    expect(play("#b").activeId()).toBe("c");
    expect(play("#d").activeId()).toBe("c");
  });

  it("numbers slides by shown position", () => {
    expect(play("#2").activeId()).toBe("c");
  });

  it("never reports a hidden slide to the presenter", () => {
    const postMessage = vi.fn();
    play("#b", { postMessage });
    const ids = postMessage.mock.calls.map(([data]) => (data as { id?: string }).id);
    expect(ids).toEqual(["c"]);
  });

  it("still renders a hidden slide when the editor embeds it", () => {
    expect(play("?embed&slide=b").activeId()).toBe("b");
    expect(play("?embed&slide=%232").activeId()).toBe("b");
  });
});

describe("player: section markers", () => {
  const SECTIONED = DECK.replace(
    `<section class="slide" id="end">`,
    `<div class="deck-section" data-title="Last part"></div>\n  <section class="slide" id="end">`,
  ).replace(
    `<main class="deck">`,
    `<main class="deck">\n  <div class="deck-section" data-title="First part"></div>`,
  );

  it("steps through slides only, never stopping on a marker", () => {
    const p = player({ html: SECTIONED });
    const seen: string[] = [];
    for (let i = 0; i < 3; i++) {
      seen.push(p.doc.querySelector(".slide.active")?.id ?? "");
      p.key("ArrowRight");
    }
    expect(seen).toEqual(["intro", "", "end"]);
    expect(p.activeCount()).toBe(1);
    expect(p.doc.querySelectorAll(".deck-section.active")).toHaveLength(0);
  });

  it("addresses slides by position without counting markers", () => {
    const p = player({ html: SECTIONED, at: "#2" });
    expect(p.active()).toBe(1);
    const embedded = player({ html: SECTIONED, at: "?embed&slide=end" });
    expect(embedded.doc.querySelector(".slide.active")?.id).toBe("end");
  });

  it("is hidden by the runtime stylesheet, on screen and in print", () => {
    const css = readFileSync("src-tauri/assets/runtime.css", "utf8");
    expect(css).toMatch(/\.deck > \.deck-section\s*\{[^}]*display:\s*none\s*!important/);
  });
});

describe("player: review marks", () => {
  const MARKS = {
    intro: [
      { tool: "pen", color: "#ef4444", points: [[0.1, 0.2], [0.5, 0.5]] },
      { tool: "highlighter", color: "#facc15", points: [[0.25, 0.25]] },
    ],
    gone: [{ tool: "pen", color: "#000", points: [[0, 0]] }],
  };
  const REVIEWED = DECK.replace(
    "</body>",
    `<script type="application/json" id="slopslide-review">${JSON.stringify(MARKS)}</script></body>`,
  );
  const shown = (p: ReturnType<typeof player>) => p.doc.documentElement.hasAttribute("data-slop-review");

  it("starts hidden and toggles with R", () => {
    const p = player({ html: REVIEWED });
    expect(shown(p)).toBe(false);
    expect(p.doc.querySelector(".slop-review")).toBeNull();
    expect(p.key("r").defaultPrevented).toBe(true);
    expect(shown(p)).toBe(true);
    p.key("R");
    expect(shown(p)).toBe(false);
    p.key("r");
    expect(p.doc.querySelectorAll(".slop-review")).toHaveLength(1);
  });

  it("draws each slide's strokes over that slide, in slide pixels", () => {
    const p = player({ html: REVIEWED, at: "?review" });
    expect(shown(p)).toBe(true);
    const svg = p.doc.querySelector("#intro > svg.slop-review")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 1920 1080");
    const [pen, dot] = [...svg.querySelectorAll("path")];
    expect(pen!.getAttribute("d")).toBe("M192 216L960 540");
    expect(pen!.getAttribute("stroke")).toBe("#ef4444");
    expect(pen!.getAttribute("vector-effect")).toBe("non-scaling-stroke");
    expect(dot!.getAttribute("d")).toBe("M480 270l0.01 0");
    expect(dot!.getAttribute("stroke-opacity")).toBe("0.4");
  });

  it("draws longer strokes as the same smooth curve the app draws", () => {
    const points: [number, number][] = [[0, 0], [0.5, 0], [0.5, 0.5], [0, 0.5]];
    const html = DECK.replace(
      "</body>",
      `<script type="application/json" id="slopslide-review">${JSON.stringify({ intro: [{ tool: "pen", color: "#000", points }] })}</script></body>`,
    );
    const p = player({ html, at: "?review" });
    const path = p.doc.querySelector("#intro > svg.slop-review path")!;
    expect(path.getAttribute("d")).toBe("M0 0L480 0Q960 0 960 270Q960 540 0 540");
    expect(path.getAttribute("d")).toBe(strokePath(toPixels(points, 1920, 1080)));
  });

  it("leaves Cmd+R to the browser and decks without marks alone", () => {
    const p = player({ html: REVIEWED });
    const event = new p.window.KeyboardEvent("keydown", { key: "r", metaKey: true, cancelable: true });
    p.window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(shown(p)).toBe(false);
    const plain = player();
    expect(plain.key("r").defaultPrevented).toBe(false);
    expect(shown(plain)).toBe(false);
  });

  it("leaves the marks to the editor in embedded previews", () => {
    const p = player({ html: REVIEWED, at: "?embed&review" });
    expect(p.doc.querySelector(".slop-review")).toBeNull();
  });

  it("ignores a damaged review block", () => {
    const p = player({ html: REVIEWED.replace('{"intro"', '{intro') });
    expect(p.key("r").defaultPrevented).toBe(false);
    expect(p.active()).toBe(0);
  });

  it("is hidden by the runtime stylesheet until shown", () => {
    const css = readFileSync("src-tauri/assets/runtime.css", "utf8");
    expect(css).toMatch(/\.slide > \.slop-review\s*\{[^}]*display:\s*none/);
    expect(css).toMatch(/html\[data-slop-review\] \.slide > \.slop-review\s*\{[^}]*display:\s*block/);
  });
});

describe("player: without JavaScript", () => {
  const CSS = readFileSync("src-tauri/assets/runtime.css", "utf8");
  // Chat apps on iOS open decks in Quick Look, which renders HTML but runs no scripts.
  function styled(flagged: boolean) {
    const html = DECK.replace("<html>", `<html${flagged ? " data-slop-player" : ""}><head><style>${CSS}</style></head>`)
      .replace('<section class="slide"><p>No id</p>', '<section class="slide" data-hidden><p class="reveal">No id</p>');
    const dom = new JSDOM(html, { pretendToBeVisual: true });
    doms.push(dom);
    const doc = dom.window.document;
    const style = (selector: string) => dom.window.getComputedStyle(doc.querySelector(selector)!);
    return { doc, style };
  }

  it("is switched on by the player", () => {
    expect(player().doc.documentElement.hasAttribute("data-slop-player")).toBe(true);
  });

  it("lists every shown slide, in the flow, when no script flags the player", () => {
    const { style } = styled(false);
    expect(style("#intro").visibility).toBe("visible");
    expect(style("#intro").opacity).toBe("1");
    expect(style("#intro").position).toBe("relative");
    expect(style("#end").visibility).toBe("visible");
    expect(style("[data-hidden]").display).toBe("none");
    expect(style("body").overflow).not.toBe("hidden");
  });

  it("keeps slides visible and the page scrolling despite the deck's own styles", () => {
    const deckStyles =
      "<style>html, body { height: 100%; overflow: hidden } .slide { opacity: 0; visibility: hidden } .reveal { opacity: 0 }</style>";
    const html = DECK.replace("<html>", `<html><head><style>${CSS}</style>${deckStyles}</head>`).replace(
      "<p>No id</p>",
      '<p class="reveal">No id</p>',
    );
    const dom = new JSDOM(html, { pretendToBeVisual: true });
    doms.push(dom);
    const style = (selector: string) => dom.window.getComputedStyle(dom.window.document.querySelector(selector)!);
    expect(style("#intro").visibility).toBe("visible");
    expect(style("#intro").opacity).toBe("1");
    expect(style(".reveal").opacity).toBe("1");
    expect(style("body").overflow).toBe("visible");
    expect(style("html").overflow).toBe("visible");
  });

  it("hides all but the active slide once the player is flagged", () => {
    const { doc, style } = styled(true);
    expect(style("#intro").visibility).toBe("hidden");
    expect(style("#intro").position).toBe("absolute");
    doc.getElementById("intro")!.classList.add("active");
    expect(style("#intro").visibility).toBe("visible");
    expect(style("[data-hidden] .reveal").opacity).toBe("0");
  });

  it("stops mobile browsers from enlarging slide text", () => {
    // iOS inflates small text on narrow screens, which overflows the zoomed slides.
    expect(CSS).toMatch(/html\s*\{[^}]*-webkit-text-size-adjust:\s*100%/);
    expect(CSS).toMatch(/html\s*\{[^}]*[^-]text-size-adjust:\s*100%/);
  });

  it("zooms the stage down to narrow windows", () => {
    const zooms = [...CSS.matchAll(/@media screen and \(max-width: ([\d.]+)px\) \{[^{]*\{ zoom: ([\d.]+); \} \}/g)];
    expect(zooms.length).toBeGreaterThan(10);
    for (const [, width, zoom] of zooms) {
      // The step applies to windows narrower than `width`; the zoomed stage still fits the
      // narrowest of them.
      const narrowest = Number(width) + 0.02 - 96;
      expect(1920 * Number(zoom)).toBeLessThanOrEqual(narrowest);
    }
    // Phones (≈375–430 CSS px wide) get a step.
    expect(zooms.some(([, width]) => Number(width) < 430)).toBe(true);
  });
});
