import { act, fireEvent, render, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));

import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { SlideFrame, useSlideVersion } from "./SlideFrame";

const NoLayout = globalThis.ResizeObserver;
let width = 960;
const observers: { callback: ResizeObserverCallback; disconnected: boolean }[] = [];

/** Reports `width` for every observed element, like a browser after layout. */
class MeasuringObserver {
  private entry: { callback: ResizeObserverCallback; disconnected: boolean };
  constructor(callback: ResizeObserverCallback) {
    this.entry = { callback, disconnected: false };
    observers.push(this.entry);
  }
  observe(target: Element) {
    this.entry.callback([{ target, contentRect: { width } } as unknown as ResizeObserverEntry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {
    this.entry.disconnected = true;
  }
}

beforeEach(() => {
  width = 960;
  observers.length = 0;
  globalThis.ResizeObserver = MeasuringObserver as unknown as typeof ResizeObserver;
});

afterEach(() => {
  globalThis.ResizeObserver = NoLayout;
});

const frames = (container: HTMLElement) => [...container.querySelectorAll("iframe")];

describe("SlideFrame", () => {
  it("renders the slide through the deck's player, scaled to fit", () => {
    const { container } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    const [frame] = frames(container);
    expect(frame!.getAttribute("src")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=v1");
    expect(frame!.title).toBe("intro");
    expect(frame!.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame!.style.width).toBe("1920px");
    expect(frame!.style.height).toBe("1080px");
    expect(frame!.style.transform).toBe("scale(0.5)");
    expect(frame!.style.pointerEvents).toBe("auto");
    expect(frame!.tabIndex).not.toBe(-1);
  });

  it("loads the slide editor (on the final animation frame) in edit mode", () => {
    const { container, rerender } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" editKey="0" />);
    expect(frames(container)[0]!.getAttribute("src")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=v1&static&edit=0");
    expect(frames(container)[0]!.style.pointerEvents).toBe("auto");
    // A new key reloads the slide, e.g. after a refused save.
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v1" editKey="1" />);
    expect(frames(container).map((f) => f.getAttribute("src"))).toContain(
      "/__deck/talk/deck.html?embed&slide=intro&v=v1&static&edit=1",
    );
  });

  const arena = { width: 1200, height: 700 };

  it("lets the editor's preview fill the arena around the centered slide, keeping the slide's scale", () => {
    const { container } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" editKey="0" arena={arena} />);
    const [frame] = frames(container);
    expect(frame!.style.width).toBe("2400px");
    expect(frame!.style.height).toBe("1400px");
    expect(frame!.style.left).toBe("-120px");
    expect(frame!.style.top).toBe("-80px");
    expect(frame!.style.transform).toBe("scale(0.5)");
    expect((container.firstElementChild as HTMLElement).style.overflow).toBe("visible");
  });

  it("never makes the preview smaller than the slide", () => {
    const { container } = render(
      <SlideFrame deckId="talk" slideId="intro" version="v1" editKey="0" arena={{ width: 100, height: 100 }} />,
    );
    const [frame] = frames(container);
    expect(frame!.style.width).toBe("1920px");
    expect(frame!.style.height).toBe("1080px");
    expect(frame!.style.left).toBe("0px");
  });

  it("puts a preview given an arena on the pasteboard, in view and edit mode", () => {
    const { container, rerender } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" arena={arena} />);
    const [viewer] = frames(container);
    expect(viewer!.getAttribute("src")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=v1&pan");
    expect(viewer!.style.width).toBe("2400px");
    expect(viewer!.style.left).toBe("-120px");
    const box = container.firstElementChild as HTMLElement;
    expect(box.style.overflow).toBe("visible");
    // The slide may be panned anywhere, so nothing is painted where it started.
    expect(box.style.background).toBe("transparent");
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v1" editKey="0" arena={arena} />);
    const editor = frames(container).at(-1)!;
    expect(editor.getAttribute("src")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=v1&static&pan&edit=0");
    expect(editor.style.width).toBe("2400px");
  });

  it("keeps a plain preview and thumbnails to the slide, clipped, with no pasteboard", () => {
    const { container, rerender } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    const box = () => container.firstElementChild as HTMLElement;
    expect(frames(container)[0]!.getAttribute("src")).not.toContain("pan");
    expect(box().style.overflow).toBe("hidden");
    expect(box().style.background).toBe("rgb(0, 0, 0)");
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v1" thumbnail arena={arena} />);
    const thumb = frames(container).at(-1)!;
    expect(thumb.getAttribute("src")).not.toContain("pan");
    expect(thumb.style.width).toBe("1920px");
  });

  it("waits for layout before loading anything", () => {
    globalThis.ResizeObserver = NoLayout;
    const { container } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    expect(frames(container)).toEqual([]);
  });

  it("follows size changes", () => {
    const { container } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    width = 1920;
    act(() => observers[0]!.callback([{ contentRect: { width } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(frames(container)[0]!.style.transform).toBe("scale(1)");
  });

  it("stops observing when unmounted", () => {
    const { unmount } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    unmount();
    expect(observers[0]!.disconnected).toBe(true);
  });

  it("thumbnails are static and inert", () => {
    const { container } = render(<SlideFrame deckId="talk" slideId="#2" version="v1" thumbnail />);
    const [frame] = frames(container);
    expect(frame!.getAttribute("src")).toBe("/__deck/talk/deck.html?embed&slide=%232&v=v1&static");
    expect(frame!.tabIndex).toBe(-1);
    expect(frame!.style.pointerEvents).toBe("none");
  });

  it("loads a new version behind the current one and swaps once it has loaded", () => {
    const onFrameReady = vi.fn();
    const { container, rerender } = render(
      <SlideFrame deckId="talk" slideId="intro" version="v1" onFrameReady={onFrameReady} />,
    );
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v2" onFrameReady={onFrameReady} />);
    const [current, next] = frames(container);
    expect(current!.getAttribute("src")).toContain("v=v1");
    expect(next!.getAttribute("src")).toContain("v=v2");
    expect(current!.style.visibility).toBe("visible");
    expect(next!.style.visibility).toBe("hidden");

    // The old frame finishing a reload changes nothing.
    fireEvent.load(current!);
    expect(frames(container)).toHaveLength(2);

    fireEvent.load(next!);
    expect(frames(container)).toEqual([next]);
    expect(next!.style.visibility).toBe("visible");
    expect(onFrameReady).toHaveBeenLastCalledWith(next);
  });

  it("only keeps the newest pending version", () => {
    const { container, rerender } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" />);
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v2" />);
    rerender(<SlideFrame deckId="talk" slideId="intro" version="v3" />);
    expect(frames(container).map((f) => f.getAttribute("src")!.split("v=")[1])).toEqual(["v1", "v3"]);
  });

  it("reports the first load too", () => {
    const onFrameReady = vi.fn();
    const { container } = render(<SlideFrame deckId="talk" slideId="intro" version="v1" onFrameReady={onFrameReady} />);
    fireEvent.load(frames(container)[0]!);
    expect(onFrameReady).toHaveBeenCalledWith(frames(container)[0]);
  });
});

describe("useSlideVersion", () => {
  it("changes with the slide, the deck's shared styles, and attached assets", () => {
    useApp.setState({ deck: deckFor(DECK_HTML), assetsRev: 0 });
    const slide = { id: "intro", hash: "h1", hidden: false, locked: false, moved: false };
    const { result, rerender } = renderHook(({ s }) => useSlideVersion(s), { initialProps: { s: slide } });
    expect(result.current).toBe("shell-1.h1.0");
    rerender({ s: { ...slide, hash: "h2" } });
    expect(result.current).toBe("shell-1.h2.0");
    act(() => useApp.setState({ assetsRev: 3 }));
    expect(result.current).toBe("shell-1.h2.3");
    act(() => useApp.setState({ deck: deckFor(DECK_HTML, "9") }));
    expect(result.current).toBe("shell-9.h2.3");
  });
});
