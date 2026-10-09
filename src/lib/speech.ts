import type { NarrationLanguage, NarrationManifest, SlideNarration } from "./narration";
export interface SpeechSource { text: string; language: NarrationLanguage; presenterId: string; pace: number }
export interface SpeechTake { id: string; key: string; engineVersion: string; modelRevision: string; source: SpeechSource; samples: number; sampleRate: number; sha256: string }
export interface SpeechJob { id: string; kind: string; deckId: string | null; sourceRevision: number | null; stage: string; completed: number; total: number; detail: string }
export interface SpeechStatus { installed: boolean; runtimeAvailable: boolean; totalBytes: number; engineVersion: string; job: SpeechJob | null }
export interface SpeechEvent { job: SpeechJob; error: string | null }
export interface SpeechResult { generated: number; reused: number; superseded: number }
export const STOCK_VOICES = [
  ["ryan", "Ryan"], ["aiden", "Aiden"], ["vivian", "Vivian"], ["serena", "Serena"], ["uncle_fu", "Uncle Fu"], ["dylan", "Dylan"], ["eric", "Eric"], ["ono_anna", "Ono Anna"], ["sohee", "Sohee"],
] as const;
export function matchesTake(take: SpeechTake, script: SlideNarration, manifest: NarrationManifest): boolean {
  return take.source.text === script.text.trim().replace(/\r\n/g, "\n") && take.source.language === (script.languageOverride ?? manifest.defaultLanguage) && take.source.presenterId === manifest.presenterId && take.source.pace === (manifest.pace ?? 1.1);
}
