import { describe, expect, it } from "vitest";

import type { Deck } from "./api";
import { applyOrder, railItems, sectionKey, startsSection } from "./sections";

const deck = (sections: [string, number][], ids = ["a", "b", "c"]): Deck => ({
  id: "talk",
  title: "Talk",
  path: "/decks/talk",
  slides: ids.map((id) => ({ id, hash: id, hidden: false, locked: false, moved: false })),
  sections: sections.map(([title, before], index) => ({ index, title, before })),
  shellHash: "s",
});

const keys = (d: Deck) => railItems(d).map((item) => item.key);

describe("railItems", () => {
  it("is just the slides when there are no sections", () => {
    const items = railItems(deck([]));
    expect(items.map((i) => i.kind)).toEqual(["slide", "slide", "slide"]);
    expect(items.map((i) => (i.kind === "slide" ? i.index : -1))).toEqual([0, 1, 2]);
  });

  it("puts each section before the slide it starts", () => {
    expect(keys(deck([["One", 0], ["Two", 2]]))).toEqual(["section:0", "a", "b", "section:1", "c"]);
  });

  it("keeps a trailing section after the last slide", () => {
    expect(keys(deck([["End", 3]]))).toEqual(["a", "b", "c", "section:0"]);
  });

  it("keeps consecutive sections in document order", () => {
    expect(keys(deck([["One", 1], ["Two", 1]]))).toEqual(["a", "section:0", "section:1", "b", "c"]);
  });

  it("lists sections without slides", () => {
    expect(keys(deck([["Only", 0]], []))).toEqual(["section:0"]);
  });
});

describe("applyOrder", () => {
  it("moves a slide across a section boundary", () => {
    const d = deck([["Two", 1]]);
    const next = applyOrder(d, ["a", "b", "section:0", "c"]);
    expect(next.slides.map((s) => s.id)).toEqual(["a", "b", "c"]);
    expect(next.sections).toEqual([{ index: 0, title: "Two", before: 2 }]);
  });

  it("moves a section marker with its title and renumbers sections", () => {
    const d = deck([["One", 0], ["Two", 2]]);
    const next = applyOrder(d, ["a", "section:1", "b", "c", "section:0"]);
    expect(next.sections).toEqual([
      { index: 0, title: "Two", before: 1 },
      { index: 1, title: "One", before: 3 },
    ]);
    expect(keys(next)).toEqual(["a", "section:0", "b", "c", "section:1"]);
  });

  it("keeps everything else on the deck and ignores unknown keys", () => {
    const d = deck([["One", 0]]);
    const next = applyOrder(d, ["b", "nope", "section:0", "a", "c"]);
    expect(next).toMatchObject({ id: "talk", shellHash: "s" });
    expect(next.slides.map((s) => s.id)).toEqual(["b", "a", "c"]);
    expect(next.sections).toEqual([{ index: 0, title: "One", before: 1 }]);
  });
});

describe("startsSection / sectionKey", () => {
  it("knows which slides start a section", () => {
    const d = deck([["One", 0], ["Two", 2]]);
    expect([0, 1, 2].map((i) => startsSection(d, i))).toEqual([true, false, true]);
  });

  it("formats keys like the backend", () => {
    expect(sectionKey(3)).toBe("section:3");
  });
});
