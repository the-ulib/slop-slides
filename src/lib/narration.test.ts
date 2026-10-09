import { describe, expect, it } from "vitest";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { emptyNarration, narrationDraftPrompt, slideReviewHash } from "./narration";
describe("narration draft requests", () => {
  it("uses stable visible IDs and leaves deleted scripts and slide HTML alone", () => {
    const deck = deckFor(DECK_HTML);
    deck.slides.find((s) => s.id === "outro")!.hidden = true;
    const prompt = narrationDraftPrompt(deck, "intro", "deck", "de", "engineers", "2");
    expect(prompt).toContain('exact slide IDs: ["intro"]');
    expect(prompt).toContain("German");
    expect(prompt).toContain("engineers");
    expect(prompt).toContain("2 minutes");
    expect(prompt).toContain("Do not change deck.html");
    expect(prompt).toContain("including removed slides");
  });
  it("allows explicit selected hidden-slide drafting, without index matching", () => {
    const deck = deckFor(DECK_HTML);
    deck.slides.find((s) => s.id === "outro")!.hidden = true;
    expect(narrationDraftPrompt(deck, "outro", "slide", "en", "", "")).toContain('exact slide IDs: ["outro"]');
  });
  it("tracks visual edits separately from script text and never relies on position", () => {
    const deck = deckFor(DECK_HTML);
    const before = slideReviewHash(deck, "intro");
    deck.slides.reverse();
    expect(slideReviewHash(deck, "intro")).toBe(before);
    deck.shellHash = "new-css";
    expect(slideReviewHash(deck, "intro")).not.toBe(before);
    expect(slideReviewHash(deck, "deleted")).toBeNull();
    expect(emptyNarration().manifest.schemaVersion).toBe(3);
  });
});
