import type { Deck } from "./api";

export type NarrationLanguage = "en" | "de";
export interface SlideNarration {
  text: string;
  languageOverride: NarrationLanguage | null;
  leadInMs: number;
  tailMs: number;
  silentDurationMs: number | null;
  acceptedTakeId: string | null;
  reviewedSlideHash: string | null;
}
export interface NarrationManifest {
  schemaVersion: number;
  revision: number;
  speechProviderId?: string;
  presenterId: string;
  presenterNameSnapshot: string;
  defaultLanguage: NarrationLanguage;
  pace?: number;
  slides: Record<string, SlideNarration>;
}
export interface NarrationDocument { manifest: NarrationManifest; version: string }
export const emptyNarration = (): NarrationDocument => ({
  version: "missing",
  manifest: { schemaVersion: 2, revision: 0, speechProviderId: "qwen-local", presenterId: "preset:ryan", presenterNameSnapshot: "Ryan", defaultLanguage: "en", pace: 1.1, slides: {} },
});
export const emptyScript = (): SlideNarration => ({ text: "", languageOverride: null, leadInMs: 250, tailMs: 500, silentDurationMs: null, acceptedTakeId: null, reviewedSlideHash: null });
export function slideReviewHash(deck: Deck, id: string): string | null {
  const slide = deck.slides.find((s) => s.id === id);
  return slide ? `${deck.shellHash}:${slide.hash}` : null;
}
export type NarrationSettings = Partial<Pick<NarrationManifest, "speechProviderId" | "presenterId" | "presenterNameSnapshot" | "pace">>;
export type NarrationEdits = Record<string, Partial<SlideNarration>>;
export function editedManifest(base: NarrationManifest, edits: NarrationEdits, language: NarrationLanguage | null, settings: NarrationSettings = {}): NarrationManifest {
  const slides = { ...base.slides };
  for (const [id, patch] of Object.entries(edits)) slides[id] = { ...(slides[id] ?? emptyScript()), ...patch };
  return { ...base, ...settings, slides, defaultLanguage: language ?? base.defaultLanguage };
}
export function narrationDraftPrompt(deck: Deck, selected: string | null, scope: "slide" | "deck", language: NarrationLanguage, audience: string, minutes: string): string {
  const ids = deck.slides.filter((s) => !s.id.startsWith("#") && (scope === "slide" ? s.id === selected : !s.hidden)).map((s) => s.id);
  return `Draft presentation narration in narration.json for these exact slide IDs: ${JSON.stringify(ids)}. Do not change deck.html, slide IDs, or assets. Follow the narration schema in your system instructions. Use read_narration and write_narration MCP tools with the returned file fingerprint; never overwrite narration via raw file edits. Read existing narration first; preserve all other entries (including removed slides) and settings. Draft natural spoken explanations, not a verbatim reading of slide text. Default language: ${language === "de" ? "German" : "English"}; respect each slide's languageOverride. Mark these new scripts for review (reviewedSlideHash: null). Preserve acceptedTakeId references so previous recordings remain recoverable until new speech succeeds. The write tool increments revision and replaces JSON atomically; re-read and merge on a conflict.\nAudience: ${audience.trim() || "general audience"}.\nTarget duration: ${minutes.trim() ? `${minutes.trim()} minutes for ${scope === "slide" ? "this slide" : "the visible deck"} (approximate, before speech generation)` : "use a concise, comfortable presentation pace"}.\nWhen finished, ask me to review the scripts in the Narration tab.`;
}
