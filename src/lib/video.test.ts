import { expect, it } from "vitest";
import { videoSlideAt, type VideoTimeline } from "./video";
const t = { fps: 30, slides: [{ startFrame: 0, endFrame: 31 }, { startFrame: 31, endFrame: 61 }, { startFrame: 61, endFrame: 90 }] } as VideoTimeline;
it("uses the exported cumulative frame boundaries, including exact seeks and the final frame", () => {
  expect(videoSlideAt(t, -1)).toBe(0);
  expect(videoSlideAt(t, 30 / 30)).toBe(0);
  expect(videoSlideAt(t, 31 / 30)).toBe(1);
  expect(videoSlideAt(t, 61 / 30)).toBe(2);
  expect(videoSlideAt(t, 3)).toBe(2);
});
