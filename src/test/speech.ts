import type { SpeechProvider } from "../lib/speech";
export const testSpeechProvider = (): SpeechProvider => ({
  id: "qwen-local", label: "Local Qwen", contractVersion: 1, processing: "local", engineVersion: "v1", modelRevision: "rev", ready: true, available: true, unavailableReason: null,
  voices: [{ id: "preset:ryan", name: "Ryan" }, { id: "preset:aiden", name: "Aiden" }], languages: ["en", "de"],
  pace: { min: 0.9, max: 1.25, default: 1.1, choices: [0.9, 1, 1.1, 1.2] }, supportsCloning: false,
  setup: { totalBytes: 2498383610, detail: "Offline after setup.", importTitle: "Choose voice pack" }, voiceHint: null,
});
export const alternateSpeechProvider = (): SpeechProvider => ({ ...testSpeechProvider(), id: "fixture-tone", label: "Test tone (not speech)", processing: "test", ready: true, setup: null, voices: [{ id: "tone:440", name: "440 Hz test tone" }], pace: { min: 0.5, max: 2, default: 1, choices: [0.5, 1, 1.5, 2] } });
