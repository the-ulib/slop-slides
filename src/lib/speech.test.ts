import { expect, it } from "vitest";
import { editedManifest, emptyNarration, emptyScript } from "./narration";
import { currentTake, matchesTake, type SpeechTake } from "./speech";
import { testSpeechProvider, alternateSpeechProvider } from "../test/speech";

const take: SpeechTake = { id: "saved", key: "key", engineVersion: "v1", modelRevision: "rev", source: { text: "Hello", language: "en", presenterId: "preset:ryan", pace: 1.1 }, samples: 24000, sampleRate: 24000, sha256: "hash" };
it("changing another slide's voice/provider/pace keeps inherited audio current", () => {
  const manifest = editedManifest(emptyNarration().manifest, {
    a: { text: "Hello", acceptedTakeId: take.id },
    b: { text: "Bye", speechProviderIdOverride: "fixture-tone", presenterIdOverride: "tone:440", paceOverride: 1.5 },
  }, null);
  expect(currentTake(take, manifest.slides.a!, manifest, [testSpeechProvider(), alternateSpeechProvider()])).toBe(true);
  expect(matchesTake(take, manifest.slides.b!, manifest)).toBe(false);
});
it("recorded overrides keep audio current when defaults change, while resetting inherits them", () => {
  const manifest = { ...emptyNarration().manifest, speechProviderId: "fixture-tone", presenterId: "tone:440", pace: 1.5 };
  const script = { ...emptyScript(), text: " Hello ", speechProviderIdOverride: "qwen-local", presenterIdOverride: "preset:ryan", presenterNameSnapshotOverride: "Renamed", paceOverride: 1.1, languageOverride: "en" as const };
  expect(currentTake(take, script, manifest, [testSpeechProvider(), alternateSpeechProvider()])).toBe(true);
  expect(matchesTake(take, { ...script, paceOverride: null }, manifest)).toBe(false);
  expect(matchesTake(take, { ...script, text: "Changed words" }, manifest)).toBe(false);
});
it("invalidates literal pre-marker audio but keeps old plain recordings current", () => {
  const text = "Hello [pause:800ms] world.";
  const script = { ...emptyScript(), text };
  const marked = { ...take, source: { ...take.source, text } };
  const manifest = emptyNarration().manifest;
  expect(matchesTake(marked, script, manifest)).toBe(false);
  expect(matchesTake({ ...marked, source: { ...marked.source, narrationFormatVersion: 1 } }, script, manifest)).toBe(true);
  expect(matchesTake(take, { ...script, text: "Hello" }, manifest)).toBe(true);
});
