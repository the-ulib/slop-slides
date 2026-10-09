import { describe, expect, it } from "vitest";

import {
  INK_STYLE,
  inkBounds,
  isDot,
  LASER_GLOW,
  LASER_SIZE,
  paintTrails,
  pruneTrails,
  SLIDE_SIZE,
  strokePath,
  TOOL_KEYS,
  toFraction,
  toPixels,
  TRAIL_FADE_MS,
  TRAIL_HOLD_MS,
  trailFade,
  trailSegments,
  visibleArea,
  zoomBox,
  type Stroke,
  type TrailPoint,
} from "./ink";
import { fakeCanvas2D } from "../test/fakeCanvas";

describe("strokePath", () => {
  it("draws a straight line between two points", () => {
    expect(
      strokePath([
        [10, 20],
        [30, 40],
      ]),
    ).toBe("M10 20L30 40");
  });

  it("curves through far-apart points, bending at each one and meeting halfway between them", () => {
    expect(
      strokePath([
        [0, 0],
        [100, 0],
        [100, 100],
        [0, 100],
      ]),
    ).toBe("M0 0L50 0Q100 0 100 50Q100 100 0 100");
  });

  it("rounds the halfway points to a tenth of a pixel", () => {
    expect(
      strokePath([
        [0, 0],
        [0.15, 0],
        [0.15, 1],
        [1, 1],
      ]),
    ).toBe("M0 0L0.1 0Q0.15 0 0.2 0.5Q0.15 1 1 1");
  });

  it("draws nothing without points", () => {
    expect(strokePath([])).toBe("");
  });
});

describe("toPixels", () => {
  it("scales fractions of the screen to pixels", () => {
    expect(
      toPixels(
        [
          [0.5, 0.25],
          [1, 1],
        ],
        1920,
        1080,
      ),
    ).toEqual([
      [960, 270],
      [1920, 1080],
    ]);
  });

  it("rounds to a tenth of a pixel", () => {
    expect(toPixels([[0.3333, 0.6667]], 1000, 100)).toEqual([[333.3, 66.7]]);
  });
});

describe("isDot", () => {
  it("is true for a tap, also when the pointer reported the same spot again", () => {
    expect(isDot([[5, 5]])).toBe(true);
    expect(
      isDot([
        [5, 5],
        [5, 5],
      ]),
    ).toBe(true);
  });

  it("is false once the pointer moved, and for no points", () => {
    expect(
      isDot([
        [5, 5],
        [5, 6],
      ]),
    ).toBe(false);
    expect(isDot([])).toBe(false);
  });
});

describe("toFraction", () => {
  it("maps a pointer position to fractions of the rect", () => {
    expect(toFraction(600, 300, new DOMRect(100, 100, 1000, 400))).toEqual([0.5, 0.5]);
    expect(toFraction(100, 500, new DOMRect(100, 100, 1000, 400))).toEqual([0, 1]);
  });

  it("rounds to keep stored strokes small", () => {
    expect(toFraction(1, 2, new DOMRect(0, 0, 3, 3))).toEqual([0.3333, 0.6667]);
  });

  it("survives an unmeasured rect", () => {
    expect(toFraction(5, 7, new DOMRect(0, 0, 0, 0))).toEqual([5, 7]);
  });
});

describe("zoomBox", () => {
  it("lays the layer out where the zoom puts it, at the zoomed size", () => {
    expect(zoomBox(0, 0, 1)).toEqual({ left: "0px", top: "0px", width: "100%", height: "100%" });
    expect(zoomBox(-100, 25.5, 2.5)).toEqual({ left: "-100px", top: "25.5px", width: "250%", height: "250%" });
  });
});

describe("TOOL_KEYS", () => {
  it("binds the drawing tools to their initials", () => {
    expect(TOOL_KEYS).toEqual({ l: "laser", p: "pen", h: "highlighter", e: "eraser" });
  });
});

describe("inkBounds", () => {
  const pen = (points: [number, number][]): Stroke => ({ tool: "pen", color: "#ef4444", points });

  it("is null without ink", () => {
    expect(inkBounds([])).toBeNull();
    expect(inkBounds([pen([])])).toBeNull();
  });

  it("covers every stroke in slide pixels, padded by the ink width", () => {
    const pad = INK_STYLE.pen.width;
    expect(
      inkBounds([
        pen([
          [0.25, 0.5],
          [0.5, 0.25],
        ]),
        pen([[0.75, 0.75]]),
      ]),
    ).toEqual({ left: 480 - pad, top: 270 - pad, right: 1440 + pad, bottom: 810 + pad });
  });

  it("pads highlighter strokes more than pen strokes", () => {
    const point: [number, number][] = [[0.5, 0.5]];
    const marker = inkBounds([{ tool: "highlighter", color: "#facc15", points: point }])!;
    const line = inkBounds([pen(point)])!;
    expect(marker.right - marker.left).toBeGreaterThan(line.right - line.left);
  });

  it("stays on the slide", () => {
    expect(
      inkBounds([
        pen([
          [0, 0],
          [1, 1],
        ]),
      ]),
    ).toEqual({ left: 0, top: 0, right: SLIDE_SIZE.width, bottom: SLIDE_SIZE.height });
  });
});

describe("laser trails", () => {
  const life = TRAIL_HOLD_MS + TRAIL_FADE_MS;
  const pt = (x: number, t: number): TrailPoint => ({ x, y: 0.5, t });

  it("stay fully visible for a while, then fade out gradually", () => {
    expect(trailFade(0)).toBe(1);
    expect(trailFade(TRAIL_HOLD_MS)).toBe(1);
    expect(trailFade(TRAIL_HOLD_MS + TRAIL_FADE_MS / 2)).toBeCloseTo(0.5);
    expect(trailFade(life)).toBe(0);
    expect(trailFade(life + 1000)).toBe(0);
  });

  it("draws a single segment straight, in pixels", () => {
    expect(trailSegments([pt(0, 0), pt(1, 0)], 0, 200, 100)).toEqual([
      { x1: 0, y1: 50, cx: 0, cy: 50, x2: 200, y2: 50, fade: 1 },
    ]);
    expect(trailSegments([pt(0, 0)], 0, 200, 100)).toEqual([]);
  });

  it("cut the smooth curve at the halfway points, each piece faded by the age of the point after it", () => {
    const at = (x: number, y: number, t: number): TrailPoint => ({ x, y, t });
    const trail = [at(0, 0, 0), at(0.5, 0, 0), at(0.5, 1, TRAIL_FADE_MS), at(0, 1, TRAIL_FADE_MS)];
    expect(trailSegments(trail, TRAIL_HOLD_MS + TRAIL_FADE_MS / 2, 200, 100)).toEqual([
      { x1: 0, y1: 0, cx: 0, cy: 0, x2: 50, y2: 0, fade: 0.5 },
      { x1: 50, y1: 0, cx: 100, cy: 0, x2: 100, y2: 50, fade: 1 },
      { x1: 100, y1: 50, cx: 100, cy: 100, x2: 0, y2: 100, fade: 1 },
    ]);
    expect(trailSegments(trail, life, 200, 100)).toEqual([
      { x1: 50, y1: 0, cx: 100, cy: 0, x2: 100, y2: 50, fade: 1 },
      { x1: 100, y1: 50, cx: 100, cy: 100, x2: 0, y2: 100, fade: 1 },
    ]);
  });

  it("drop faded points and trails, keeping every point that shapes a visible piece", () => {
    const trail = [pt(0, 0), pt(0.25, 100), pt(0.5, 200), pt(0.75, 300)];
    const trails = [[pt(0, 0)], trail];
    expect(pruneTrails(trails, 50)).toBe(trails);
    // The piece around the second point is fading out by the third point's age, so the first stays.
    expect(pruneTrails(trails, life + 150)).toEqual([trail]);
    expect(pruneTrails(trails, life + 250)).toEqual([[pt(0.25, 100), pt(0.5, 200), pt(0.75, 300)]]);
    expect(pruneTrails(trails, life + 300)).toEqual([]);
  });

  it("look the same just before and after a point is dropped", () => {
    const trail = [pt(0, 0), pt(0.25, 100), pt(0.5, 200), pt(0.75, 300), pt(1, 400)];
    const now = life + 200;
    const [pruned] = pruneTrails([trail], now);
    expect(pruned).toHaveLength(trail.length - 1);
    expect(trailSegments(pruned!, now, 200, 100)).toEqual(trailSegments(trail, now, 200, 100));
  });
});

describe("paintTrails", () => {
  const pt = (x: number, t: number): TrailPoint => ({ x, y: 0.5, t });

  it("strokes each visible piece as wide as the laser, thinning as it fades", () => {
    const [target, core] = [fakeCanvas2D(), fakeCanvas2D()];
    const trail = [pt(0, 0), pt(0.5, 0), pt(1, TRAIL_FADE_MS)];
    paintTrails(target, core, [trail], TRAIL_HOLD_MS + TRAIL_FADE_MS / 2, 200, 100, 2);
    expect(core.strokes).toEqual([LASER_SIZE / 2, LASER_SIZE]);
  });

  it("glows with every halo of the laser, under one solid copy of the trail", () => {
    const [target, core] = [fakeCanvas2D(), fakeCanvas2D()];
    paintTrails(target, core, [[pt(0, 0), pt(1, 0)]], 0, 200, 100, 2);
    expect(target.images).toEqual([
      ...[...LASER_GLOW].reverse().map((g) => ({
        shadowBlur: (g.blur + g.spread) * 2,
        shadowColor: `rgb(239 68 68 / ${g.opacity})`,
      })),
      { shadowBlur: 0, shadowColor: "transparent" },
    ]);
  });

  it("draws a layer's trails shifted so the canvas starts at the given origin", () => {
    const [target, core] = [fakeCanvas2D(), fakeCanvas2D()];
    paintTrails(target, core, [[pt(0, 0), pt(1, 0)]], 0, 200, 100, 2, { x: 30, y: 10 });
    expect(core.transform).toEqual([2, 0, 0, 2, -60, -20]);
  });

  it("clears what was painted before", () => {
    const [target, core] = [fakeCanvas2D(), fakeCanvas2D()];
    paintTrails(target, core, [[pt(0, 0), pt(1, 0)]], 0, 200, 100, 1);
    paintTrails(target, core, [], 0, 200, 100, 1);
    expect(core.strokes).toEqual([]);
  });
});

describe("visibleArea", () => {
  const viewport = { width: 800, height: 600 };

  it("is the whole layer when it fits on screen", () => {
    expect(visibleArea({ left: 50, top: 20, width: 400, height: 300 }, viewport)).toEqual({ x: 0, y: 0, width: 400, height: 300 });
  });

  it("is only the on-screen part of a zoomed layer, in the layer's pixels", () => {
    // A 4x zoom: the layer is 3200x2400, shifted so its middle is on screen.
    expect(visibleArea({ left: -1200, top: -900, width: 3200, height: 2400 }, viewport)).toEqual({
      x: 1200,
      y: 900,
      width: 800,
      height: 600,
    });
  });

  it("widens to whole pixels", () => {
    expect(visibleArea({ left: -10.5, top: -0.25, width: 2000, height: 2000 }, viewport)).toEqual({
      x: 10,
      y: 0,
      width: 801,
      height: 601,
    });
  });

  it("is empty for a layer off screen", () => {
    expect(visibleArea({ left: 900, top: 0, width: 400, height: 300 }, viewport)).toMatchObject({ width: 0 });
    expect(visibleArea({ left: -500, top: 0, width: 400, height: 300 }, viewport)).toMatchObject({ width: 0 });
  });
});
