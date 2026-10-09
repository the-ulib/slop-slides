import { act, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setFullscreen = vi.fn(async () => {});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFullscreen }) }));

import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { LASER_SIZE, TRAIL_FADE_MS, TRAIL_HOLD_MS } from "../lib/ink";
import { fakeCanvas2D } from "../test/fakeCanvas";
import { Presenter } from "./Presenter";

beforeEach(() => {
  setFullscreen.mockClear();
  useApp.setState({ deck: deckFor(DECK_HTML), selected: "outro", presenting: true, revealRev: 0 });
});

const frame = () => screen.getByTitle("Presentation") as HTMLIFrameElement;

function fromFrame(data: unknown, source: MessageEventSource | null = frame().contentWindow) {
  act(() => void window.dispatchEvent(new MessageEvent("message", { data, source })));
}

describe("Presenter", () => {
  it("plays the whole deck from the selected slide", () => {
    render(<Presenter />);
    expect(frame().getAttribute("src")).toBe("/__deck/talk/deck.html?v=shell-1&show#outro");
    expect(frame().getAttribute("sandbox")).toBe("allow-scripts");
  });

  it("encodes positional slide ids in the hash", () => {
    useApp.setState({ selected: "#2" });
    render(<Presenter />);
    expect(frame().getAttribute("src")).toBe("/__deck/talk/deck.html?v=shell-1&show#%232");
  });

  it("starts at the beginning without a selection", () => {
    useApp.setState({ selected: null });
    render(<Presenter />);
    expect(frame().getAttribute("src")).toBe("/__deck/talk/deck.html?v=shell-1&show");
  });

  it("does not restart when the editor selection follows the show", () => {
    render(<Presenter />);
    fromFrame({ type: "slop:slide", id: "intro" });
    expect(useApp.getState().selected).toBe("intro");
    expect(frame().getAttribute("src")).toContain("#outro");
  });

  it("goes full screen for the show and back afterwards", () => {
    const { unmount } = render(<Presenter />);
    expect(setFullscreen).toHaveBeenCalledWith(true);
    unmount();
    expect(setFullscreen).toHaveBeenLastCalledWith(false);
  });

  it("Escape ends the show, from the app or from inside the slide", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(useApp.getState().presenting).toBe(false);
    useApp.setState({ presenting: true });
    fromFrame({ type: "slop:key", key: "Escape" });
    expect(useApp.getState().presenting).toBe(false);
  });

  it("hands other keys to the player", () => {
    render(<Presenter />);
    const focus = vi.spyOn(frame(), "focus");
    const post = vi.spyOn(frame().contentWindow!, "postMessage");
    fireEvent.keyDown(document.body, { key: "ArrowRight" });
    expect(focus).toHaveBeenCalled();
    expect(post).toHaveBeenCalledWith({ type: "slop:go", key: "ArrowRight" }, "*");
    expect(useApp.getState().presenting).toBe(true);
  });

  it("has previous and next buttons that drive the player", () => {
    render(<Presenter />);
    const post = vi.spyOn(frame().contentWindow!, "postMessage");
    fireEvent.click(screen.getByLabelText("Next slide"));
    expect(post).toHaveBeenLastCalledWith({ type: "slop:go", key: "ArrowRight" }, "*");
    fireEvent.click(screen.getByLabelText("Previous slide"));
    expect(post).toHaveBeenLastCalledWith({ type: "slop:go", key: "ArrowLeft" }, "*");
  });

  it("ignores messages that are not from the show", () => {
    render(<Presenter />);
    fromFrame({ type: "slop:key", key: "Escape" }, window);
    fromFrame({ type: "slop:slide", id: "intro" }, null);
    fromFrame({ type: "slop:slide", id: null });
    fromFrame("not an object");
    expect(useApp.getState().presenting).toBe(true);
    expect(useApp.getState().selected).toBe("outro");
  });

  it("keeps the ink on the slide as it is zoomed and panned, ignoring nonsense", () => {
    render(<Presenter />);
    const zoom = () => {
      const { left, top, width, height, transform } = screen.getByTestId("annotation-zoom").style;
      return { left, top, width, height, transform };
    };
    expect(zoom()).toEqual({ left: "0px", top: "0px", width: "100%", height: "100%", transform: "" });
    fromFrame({ type: "slop:zoom", x: -100, y: -50, k: 2 });
    // Laid out at the zoomed size, not CSS-scaled, so the ink is not stretched blurry.
    expect(zoom()).toEqual({ left: "-100px", top: "-50px", width: "200%", height: "200%", transform: "" });
    fromFrame({ type: "slop:zoom", x: "a", y: 0, k: 1 });
    fromFrame({ type: "slop:zoom", x: 0, y: 0, k: 1 }, window);
    expect(zoom()).toEqual({ left: "-100px", top: "-50px", width: "200%", height: "200%", transform: "" });
  });

  it("0 fits the slide to the screen again, from the app too", () => {
    render(<Presenter />);
    const post = vi.spyOn(frame().contentWindow!, "postMessage");
    fireEvent.keyDown(document.body, { key: "0" });
    expect(post).toHaveBeenCalledWith({ type: "slop:camera", home: true }, "*");
    expect(post).not.toHaveBeenCalledWith(expect.objectContaining({ type: "slop:go" }), "*");
  });

  it("offers to fit the slide again once it is zoomed or panned", () => {
    render(<Presenter />);
    const recenter = () => screen.queryByTitle(/fit to the screen/);
    expect(recenter()).toBeNull();
    fromFrame({ type: "slop:zoom", x: 10, y: 0, k: 1 });
    expect(recenter()!.textContent).toBe("100%");
    fromFrame({ type: "slop:zoom", x: -100, y: -50, k: 2 });
    expect(recenter()!.textContent).toBe("200%");
    const post = vi.spyOn(frame().contentWindow!, "postMessage");
    fireEvent.click(recenter()!);
    expect(post).toHaveBeenCalledWith({ type: "slop:camera", home: true }, "*");
    fromFrame({ type: "slop:zoom", x: 0, y: 0, k: 1 });
    expect(recenter()).toBeNull();
  });

  it("stops listening after the show", () => {
    const { unmount } = render(<Presenter />);
    unmount();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(useApp.getState().presenting).toBe(true);
  });

  it("renders nothing without a deck", () => {
    useApp.setState({ deck: null });
    render(<Presenter />);
    expect(screen.queryByTitle("Presentation")).toBeNull();
    expect(screen.queryByRole("toolbar")).toBeNull();
  });
});

describe("Presenter tools", () => {
  const layer = () => screen.getByTestId("annotation-layer");
  const tool = (name: RegExp) => screen.getByRole("button", { name });
  const pressed = (name: RegExp) => tool(name).getAttribute("aria-pressed") === "true";
  const strokes = () => layer().querySelectorAll("[data-stroke]");

  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 1000, 500));
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function draw(points: [number, number][]) {
    const [first, ...rest] = points;
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1, clientX: first![0], clientY: first![1], pointerId: 1 });
    for (const [x, y] of rest) fireEvent.pointerMove(layer(), { buttons: 1, clientX: x, clientY: y, pointerId: 1 });
    fireEvent.pointerUp(layer(), { pointerId: 1 });
  }

  it("starts with the slide clickable and no tool", () => {
    render(<Presenter />);
    expect(pressed(/^Pointer/)).toBe(true);
    expect(layer().style.pointerEvents).toBe("none");
  });

  it("keyboard shortcuts pick tools, and the same key puts the tool away", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "p" });
    expect(pressed(/^Pen/)).toBe(true);
    expect(layer().style.pointerEvents).toBe("auto");
    fireEvent.keyDown(document.body, { key: "H" });
    expect(pressed(/^Highlighter/)).toBe(true);
    fireEvent.keyDown(document.body, { key: "e" });
    expect(pressed(/^Eraser/)).toBe(true);
    fireEvent.keyDown(document.body, { key: "l" });
    expect(pressed(/^Laser/)).toBe(true);
    fireEvent.keyDown(document.body, { key: "l" });
    expect(pressed(/^Pointer/)).toBe(true);
  });

  it("shortcuts also work while the slide has focus", () => {
    render(<Presenter />);
    fromFrame({ type: "slop:key", key: "p", mod: false });
    expect(pressed(/^Pen/)).toBe(true);
  });

  it("tool shortcuts are not passed on to the player", () => {
    render(<Presenter />);
    const post = vi.spyOn(frame().contentWindow!, "postMessage");
    fireEvent.keyDown(document.body, { key: "p" });
    expect(post).not.toHaveBeenCalled();
  });

  it("Escape puts the tool away first, then ends the show", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "p" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(pressed(/^Pointer/)).toBe(true);
    expect(useApp.getState().presenting).toBe(true);
    fromFrame({ type: "slop:key", key: "Escape" });
    expect(useApp.getState().presenting).toBe(false);
  });

  it("toolbar buttons pick tools and end the show", () => {
    render(<Presenter />);
    fireEvent.click(tool(/^Highlighter/));
    expect(pressed(/^Highlighter/)).toBe(true);
    fireEvent.click(tool(/^End show/));
    expect(useApp.getState().presenting).toBe(false);
  });

  it("the pen draws strokes in fractions of the screen", () => {
    render(<Presenter />);
    fireEvent.click(tool(/^Pen/));
    draw([
      [100, 100],
      [200, 150],
      [300, 250],
    ]);
    expect(strokes()).toHaveLength(1);
    const path = strokes()[0]!;
    expect(path.getAttribute("d")).toBe("M100 100L150 125Q200 150 300 250");
    expect(path.getAttribute("stroke")).toBe("#ef4444");
    expect(path.getAttribute("stroke-width")).toBe("4");
  });

  /** A move that also carries the samples the browser coalesced into it, the event's own last. */
  function moveThrough(samples: [number, number][]) {
    const [x, y] = samples[samples.length - 1]!;
    const event = createEvent.pointerMove(layer(), { buttons: 1, clientX: x, clientY: y, pointerId: 1 });
    const coalesced = samples.map(([clientX, clientY]) => ({ clientX, clientY }));
    Object.assign(event, { getCoalescedEvents: () => coalesced });
    fireEvent(layer(), event);
  }

  it("the pen keeps every sample of a fast move, not just one per frame", () => {
    render(<Presenter />);
    fireEvent.click(tool(/^Pen/));
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    moveThrough([
      [200, 100],
      [200, 200],
      [100, 200],
    ]);
    fireEvent.pointerUp(layer(), { pointerId: 1 });
    expect(strokes()[0]!.getAttribute("d")).toBe("M100 100L150 100Q200 100 200 150Q200 200 100 200");
  });

  it("draws ink as vectors at the zoomed size, thickening with the slide", () => {
    const resized: (() => void)[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: () => void) {
          resized.push(callback);
        }
        observe() {}
        disconnect() {}
      },
    );
    render(<Presenter />);
    fireEvent.click(tool(/^Pen/));
    draw([
      [100, 100],
      [200, 150],
    ]);
    fireEvent.keyDown(document.body, { key: "h" });
    draw([[300, 200]]);
    // The layer now measures twice the size, as it would once laid out at 200%.
    vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockReturnValue(new DOMRect(-100, -50, 2000, 1000));
    fromFrame({ type: "slop:zoom", x: -100, y: -50, k: 2 });
    // Redrawn right in the resize callback, before the browser paints, so zooming doesn't flicker.
    resized.forEach((callback) => callback());
    const [pen, dot] = strokes();
    expect(pen!.getAttribute("d")).toBe("M200 200L400 300");
    expect(pen!.getAttribute("stroke-width")).toBe("8");
    expect(dot!.getAttribute("r")).toBe("28");
    expect(layer().querySelector("svg")!.style.transform).toBe("");
  });

  it("the highlighter draws wide, translucent strokes", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "h" });
    draw([
      [0, 0],
      [500, 250],
    ]);
    const path = strokes()[0]!;
    expect(path.getAttribute("stroke")).toBe("#facc15");
    expect(path.getAttribute("stroke-width")).toBe("28");
    expect(path.getAttribute("stroke-opacity")).toBe("0.4");
  });

  it("a tap leaves a dot the size of the tool, not a highlighted screen", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "h" });
    draw([[300, 200]]);
    expect(layer().querySelector("path")).toBeNull();
    const dot = strokes()[0]!;
    expect(dot.tagName).toBe("circle");
    expect(dot.getAttribute("cx")).toBe("300");
    expect(dot.getAttribute("cy")).toBe("200");
    expect(dot.getAttribute("r")).toBe("14");
    expect(dot.getAttribute("fill")).toBe("#facc15");
    expect(dot.getAttribute("fill-opacity")).toBe("0.4");
    fireEvent.keyDown(document.body, { key: "p" });
    draw([[10, 10]]);
    expect(strokes()[1]!.getAttribute("r")).toBe("2");
  });

  it("draws in pixels so strokes keep their width, and follows resizes", () => {
    const resized: (() => void)[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: () => void) {
          resized.push(callback);
        }
        observe() {}
        disconnect() {}
      },
    );
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "p" });
    draw([
      [100, 100],
      [500, 250],
    ]);
    const svg = layer().querySelector("svg")!;
    expect(svg.hasAttribute("viewBox")).toBe(false);
    expect(strokes()[0]!.hasAttribute("vector-effect")).toBe(false);
    vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockReturnValue(new DOMRect(0, 0, 2000, 1000));
    act(() => resized.forEach((callback) => callback()));
    expect(strokes()[0]!.getAttribute("d")).toBe("M200 200L1000 500");
  });

  it("colors apply to the active ink tool", () => {
    render(<Presenter />);
    expect(screen.queryByRole("button", { name: /^Color/ })).toBeNull();
    fireEvent.click(tool(/^Pen/));
    fireEvent.click(tool(/Color #3b82f6/));
    expect(tool(/Color #3b82f6/).getAttribute("aria-pressed")).toBe("true");
    draw([
      [10, 10],
      [20, 20],
    ]);
    expect(strokes()[0]!.getAttribute("stroke")).toBe("#3b82f6");
    fireEvent.click(tool(/^Highlighter/));
    expect(tool(/Color #facc15/).getAttribute("aria-pressed")).toBe("true");
  });

  it("ignores right clicks and moves without a press", () => {
    render(<Presenter />);
    fireEvent.click(tool(/^Pen/));
    fireEvent.pointerDown(layer(), { button: 2, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(layer(), { clientX: 20, clientY: 20 });
    fireEvent.pointerUp(layer());
    expect(strokes()).toHaveLength(0);
  });

  it("undo removes the last stroke, clear removes them all", () => {
    render(<Presenter />);
    expect((tool(/^Undo/) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(tool(/^Pen/));
    draw([[10, 10]]);
    draw([[20, 20]]);
    draw([[30, 30]]);
    fireEvent.keyDown(document.body, { key: "z", metaKey: true });
    expect(strokes()).toHaveLength(2);
    fromFrame({ type: "slop:key", key: "z", mod: true });
    expect(strokes()).toHaveLength(1);
    fireEvent.click(tool(/^Undo/));
    expect(strokes()).toHaveLength(0);
    draw([[10, 10]]);
    draw([[20, 20]]);
    fireEvent.keyDown(document.body, { key: "c" });
    expect(strokes()).toHaveLength(0);
    draw([[10, 10]]);
    fireEvent.click(tool(/^Clear slide/));
    expect(strokes()).toHaveLength(0);
  });

  it("modifier shortcuts other than undo are left alone", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "p", metaKey: true });
    expect(pressed(/^Pointer/)).toBe(true);
  });

  it("the eraser removes the strokes it touches", () => {
    render(<Presenter />);
    fireEvent.click(tool(/^Pen/));
    draw([[10, 10]]);
    draw([[20, 20]]);
    draw([[30, 30]]);
    fireEvent.click(tool(/^Eraser/));
    fireEvent.pointerDown(strokes()[1]!, { button: 0, buttons: 1 });
    expect(strokes()).toHaveLength(2);
    fireEvent.pointerMove(strokes()[0]!, { buttons: 0 });
    expect(strokes()).toHaveLength(2);
    fireEvent.pointerMove(strokes()[0]!, { buttons: 1 });
    expect(strokes()).toHaveLength(1);
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1 });
    expect(strokes()).toHaveLength(1);
  });

  it("keeps ink with the slide it was drawn on", () => {
    render(<Presenter />);
    fromFrame({ type: "slop:slide", id: "intro", index: 0 });
    fireEvent.click(tool(/^Pen/));
    draw([[10, 10]]);
    fromFrame({ type: "slop:slide", id: null, index: 1 });
    expect(strokes()).toHaveLength(0);
    draw([[20, 20]]);
    draw([[30, 30]]);
    fromFrame({ type: "slop:slide", id: "intro", index: 0 });
    expect(strokes()).toHaveLength(1);
    fromFrame({ type: "slop:slide", id: null, index: 1 });
    expect(strokes()).toHaveLength(2);
  });

  it("the laser shows a dot that follows the pointer", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "l" });
    expect(layer().style.cursor).toBe("none");
    expect(screen.queryByTestId("laser")).toBeNull();
    fireEvent.pointerMove(layer(), { clientX: 120, clientY: 80 });
    const dot = screen.getByTestId("laser");
    expect(dot.style.left).toBe("120px");
    expect(dot.style.top).toBe("80px");
    fireEvent.pointerDown(layer(), { button: 0, clientX: 120, clientY: 80 });
    fireEvent.pointerUp(layer());
    expect(strokes()).toHaveLength(0);
    fireEvent.pointerLeave(layer());
    expect(screen.queryByTestId("laser")).toBeNull();
  });

  /** Records what is painted on the laser trail's canvas. */
  function paintedTrail() {
    const painted = { strokes: [] as number[], images: [] as { shadowBlur: number; shadowColor: string }[] };
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function (this: HTMLCanvasElement) {
      const ctx = fakeCanvas2D(this);
      // The visible canvas is in the document; the scratch canvas the trail is drawn on first is not.
      if (this.isConnected) {
        const clear = ctx.clearRect;
        ctx.clearRect = () => {
          clear();
          painted.strokes = ctx.strokes;
          painted.images = ctx.images;
        };
      } else {
        ctx.clearRect = () => {
          ctx.strokes = [];
          painted.strokes = ctx.strokes;
        };
      }
      return ctx as unknown as CanvasRenderingContext2D;
    } as unknown as typeof HTMLCanvasElement.prototype.getContext);
    return painted;
  }

  function dragLaser() {
    fireEvent.keyDown(document.body, { key: "l" });
    // Just moving the laser leaves no trail.
    fireEvent.pointerMove(layer(), { buttons: 0, clientX: 100, clientY: 100 });
    expect(screen.queryByTestId("laser-trail")).toBeNull();
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(layer(), { buttons: 1, clientX: 200, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(layer(), { buttons: 1, clientX: 300, clientY: 150, pointerId: 1 });
    fireEvent.pointerUp(layer(), { pointerId: 1 });
  }

  it("dragging the laser leaves a trail that lingers, then fades away", () => {
    vi.useFakeTimers();
    try {
      const painted = paintedTrail();
      render(<Presenter />);
      dragLaser();
      expect(screen.getByTestId("laser-trail")).toBeTruthy();
      expect(painted.strokes).toEqual([LASER_SIZE, LASER_SIZE]);
      expect(strokes()).toHaveLength(0);
      act(() => void vi.advanceTimersByTime(TRAIL_HOLD_MS - 100));
      expect(painted.strokes).toEqual([LASER_SIZE, LASER_SIZE]);
      act(() => void vi.advanceTimersByTime(100 + TRAIL_FADE_MS / 2));
      expect(painted.strokes).toHaveLength(2);
      for (const width of painted.strokes) {
        expect(width).toBeGreaterThan(0);
        expect(width).toBeLessThan(LASER_SIZE);
      }
      act(() => void vi.advanceTimersByTime(TRAIL_FADE_MS));
      expect(screen.queryByTestId("laser-trail")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("the laser trail keeps every sample of a fast move", () => {
    const painted = paintedTrail();
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "l" });
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    moveThrough([
      [200, 100],
      [200, 200],
      [100, 200],
    ]);
    expect(painted.strokes).toEqual([LASER_SIZE, LASER_SIZE, LASER_SIZE]);
  });

  it("a fresh laser trail is as wide as the dot and glows like it", () => {
    const painted = paintedTrail();
    render(<Presenter />);
    dragLaser();
    fireEvent.pointerMove(layer(), { buttons: 0, clientX: 300, clientY: 150 });
    const dot = screen.getByTestId("laser");
    expect(`${painted.strokes[0]}px`).toBe(dot.style.width);
    const colors = (css: string) => css.match(/rgb\([^)]*\)/g)!.map((c) => c.replace(/\s+/g, " "));
    const glow = painted.images.filter((i) => i.shadowBlur > 0).map((i) => i.shadowColor);
    expect(new Set(glow)).toEqual(new Set(colors(dot.style.boxShadow)));
  });

  it("a zoomed-in laser trail only gets a canvas the size of the screen", () => {
    paintedTrail();
    const zoomed = { left: -1000, top: -500, width: window.innerWidth * 4, height: window.innerHeight * 4 };
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ ...zoomed, x: zoomed.left, y: zoomed.top } as DOMRect);
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(zoomed.width);
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(zoomed.height);
    render(<Presenter />);
    dragLaser();
    const canvas = screen.getByTestId("laser-trail") as HTMLCanvasElement;
    expect([canvas.style.left, canvas.style.top]).toEqual(["1000px", "500px"]);
    expect([canvas.style.width, canvas.style.height]).toEqual([`${window.innerWidth}px`, `${window.innerHeight}px`]);
    const scale = window.devicePixelRatio || 1;
    expect([canvas.width, canvas.height]).toEqual([window.innerWidth * scale, window.innerHeight * scale]);
  });

  it("putting the laser away clears its trail", () => {
    render(<Presenter />);
    fireEvent.keyDown(document.body, { key: "l" });
    fireEvent.pointerDown(layer(), { button: 0, buttons: 1, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(layer(), { buttons: 1, clientX: 200, clientY: 100, pointerId: 1 });
    expect(screen.queryByTestId("laser-trail")).not.toBeNull();
    fireEvent.keyDown(document.body, { key: "l" });
    expect(screen.queryByTestId("laser-trail")).toBeNull();
  });

  it("the toolbar shows on hover and briefly after shortcuts", () => {
    vi.useFakeTimers();
    try {
      render(<Presenter />);
      const bar = screen.getByRole("toolbar");
      expect(bar.dataset.visible).toBe("true");
      act(() => void vi.advanceTimersByTime(2000));
      expect(bar.dataset.visible).toBe("false");
      fireEvent.mouseEnter(screen.getByTestId("presenter-toolbar-zone"));
      expect(bar.dataset.visible).toBe("true");
      fireEvent.mouseLeave(screen.getByTestId("presenter-toolbar-zone"));
      expect(bar.dataset.visible).toBe("false");
      fireEvent.keyDown(document.body, { key: "p" });
      expect(bar.dataset.visible).toBe("true");
      act(() => void vi.advanceTimersByTime(2000));
      expect(bar.dataset.visible).toBe("false");
    } finally {
      vi.useRealTimers();
    }
  });

  it("the arrow buttons show at the start, on hover and briefly after shortcuts", () => {
    vi.useFakeTimers();
    try {
      render(<Presenter />);
      const next = screen.getByLabelText("Next slide");
      const zone = screen.getByTestId("nav-zone");
      expect(next.dataset.visible).toBe("true");
      act(() => void vi.advanceTimersByTime(2000));
      expect(next.dataset.visible).toBe("false");
      fireEvent.mouseEnter(zone);
      expect(next.dataset.visible).toBe("true");
      fireEvent.mouseLeave(zone);
      expect(next.dataset.visible).toBe("false");
      fireEvent.keyDown(document.body, { key: "p" });
      expect(next.dataset.visible).toBe("true");
      act(() => void vi.advanceTimersByTime(2000));
      expect(next.dataset.visible).toBe("false");
    } finally {
      vi.useRealTimers();
    }
  });

  it("toolbar buttons do not steal focus from the slide", () => {
    render(<Presenter />);
    const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    tool(/^Pen/).dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });
});
