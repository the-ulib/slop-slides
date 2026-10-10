import type { NarrationLanguage, NarrationManifest, SlideNarration } from "./narration";
import { narrationMarkers } from "./narrationMarkers";
import { slideSpeechSettings } from "./narration";
export interface SpeechSource { providerId?: string; text: string; language: NarrationLanguage; presenterId: string; pace: number; narrationFormatVersion?: number; presenterRevision?: string }
export interface SpeechTake { id: string; key: string; engineVersion: string; modelRevision: string; source: SpeechSource; samples: number; sampleRate: number; sha256: string }
export interface SpeechHistoryTake extends SpeechTake { createdAt: number }
export interface SpeechJob { id: string; kind: string; deckId: string | null; sourceRevision: number | null; stage: string; completed: number; total: number; detail: string }
export interface SpeechProvider {
  id: string; label: string; contractVersion: number; processing: "local" | "cloud" | "test";
  engineVersion: string; modelRevision: string; ready: boolean; available: boolean; unavailableReason: string | null;
  voices: { id: string; name: string; revision?: string; ready?: boolean; modelRevision?: string; engineVersion?: string; referenceLanguage?: string }[]; languages: string[];
  pace: { min: number; max: number; default: number; choices: number[] }; supportsCloning: boolean; cloneReady?: boolean; cloneSetup?: SpeechProvider["setup"];
  narrationControls?: { formatVersion: number; maxPauseMs: number; maxMarkers: number; supportsTone: boolean }; narrationGuidance?: string;
  setup: { totalBytes: number; detail: string; importTitle: string | null } | null; voiceHint: string | null;
}
export interface VoiceProfile { id: string; revision: string; name: string; referenceLanguage: "en" | "de"; ready: boolean }
export interface SavedPresenter extends VoiceProfile { providerId: string }
export interface DefaultPresenter { providerId: string; presenterId: string }
export interface SpeechStatus { providers: SpeechProvider[]; job: SpeechJob | null; presenters?: SavedPresenter[]; defaultPresenter?: DefaultPresenter | null }
export interface SpeechEvent { job: SpeechJob; error: string | null }
export interface SpeechResult { generated: number; reused: number; superseded: number }
export function matchesTake(take: SpeechTake, script: SlideNarration, manifest: NarrationManifest): boolean {
  const settings = slideSpeechSettings(manifest, script);
  const markers = narrationMarkers(script.text);
  if (markers.error || (take.source.narrationFormatVersion ?? 0) !== (markers.pauses.length ? 1 : 0)) return false;
  return (take.source.providerId ?? "qwen-local") === settings.speechProviderId && take.source.text === script.text.trim().replace(/\r\n/g, "\n") && take.source.language === (script.languageOverride ?? manifest.defaultLanguage) && take.source.presenterId === settings.presenterId && take.source.pace === settings.pace;
}
export function currentTake(take: SpeechTake, script: SlideNarration, manifest: NarrationManifest, providers: SpeechProvider[]): boolean {
  const provider = providers.find((p) => p.id === slideSpeechSettings(manifest, script).speechProviderId);
  const voice = provider?.voices.find((v) => v.id === take.source.presenterId);
  return matchesTake(take, script, manifest) && (!provider || (take.engineVersion === (voice?.engineVersion ?? provider.engineVersion) && take.modelRevision === (voice?.modelRevision ?? provider.modelRevision) && (!voice || take.source.presenterRevision === voice.revision)));
}
