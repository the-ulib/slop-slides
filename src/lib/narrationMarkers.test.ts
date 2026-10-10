import { expect, it } from "vitest";
import cases from "../../src-tauri/speech-connector/data/narration-cases.json";
import { insertPause, narrationMarkers } from "./narrationMarkers";
it("uses the same accepted and rejected marker syntax as the native renderer", () => {
  for (const test of cases) {
    const result = narrationMarkers(test.text);
    expect(!!result.error, test.text).toBe("error" in test && test.error === true);
    if (!result.error) expect(result.pauses.map((p) => p.ms), test.text).toEqual("pauses" in test ? test.pauses : []);
  }
  expect(narrationMarkers(`Hello ${"[pause:1ms]".repeat(101)}`).error).toBeTruthy();
  expect(narrationMarkers(`Hello ${"[pause:60000ms]".repeat(11)}`).error).toBeTruthy();
  expect(narrationMarkers("[pause:800ms]").error).toContain("spoken text");
});
it("counts only spoken words and adds pause duration to the estimate", () => {
  const result = narrationMarkers("First thought. [pause:800ms] Second thought.");
  expect(result.words).toBe(4); expect(result.pauseMs).toBe(800);
  expect(result.pauses).toEqual([{ start: 15, end: 28, ms: 800 }]);
});
it("inserts at the cursor or replaces a selection without merging words", () => {
  expect(insertPause("First.Second", 6, 6)).toEqual({ text: "First. [pause:800ms] Second", cursor: 21 });
  expect(insertPause("First replace last", 6, 13, 1500).text).toBe("First [pause:1500ms] last");
  expect(insertPause("", 0, 0).text).toBe("[pause:800ms]");
});
