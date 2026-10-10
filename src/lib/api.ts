import { invoke } from "@tauri-apps/api/core";

import type { SpeechStatus, SpeechTake, SpeechResult } from "./speech";
import type { NarrationDocument, NarrationManifest } from "./narration";
import type { Approval, ApprovalDecision, PermissionMode } from "./permissions";
import type { Stroke } from "./ink";
import type { Provider, ProviderInfo } from "./models";

export interface DeckSummary {
  id: string;
  title: string;
  slideCount: number;
  firstSlide: string | null;
  updatedMs: number;
}

export interface Slide {
  /** The slide's `id` attribute in deck.html. */
  id: string;
  /** Changes when the slide's markup changes. */
  hash: string;
  /** Has `data-hidden`: skipped when presenting, shown muted in the editor. */
  hidden: boolean;
  /** Has `data-locked`: neither the user nor the agent can change it until it is unlocked. */
  locked: boolean;
  /** Has elements moved by hand (`data-moved`) that the agent has not tidied up yet. */
  moved: boolean;
}

/** A named group of slides, started by a marker between slides in deck.html. */
export interface Section {
  /** Position among the deck's section markers, in document order. */
  index: number;
  title: string;
  /** Number of slides before the marker: the section starts at the slide with this index. */
  before: number;
}

export interface Deck {
  id: string;
  title: string;
  path: string;
  slides: Slide[];
  sections: Section[];
  /** Changes when anything outside the slides (styles, fonts) changes. */
  shellHash: string;
  /** Review marks the user drew, by slide id, stored in deck.html. */
  review?: Record<string, Stroke[]>;
  /** Id of the template the deck's design comes from (its `slopslide-template` meta). */
  template?: string | null;
}

/** A deck of example layouts in one style; each slide is a layout for new or changed slides. */
export interface TemplateSummary {
  id: string;
  title: string;
  /** Ships with the app; otherwise it is the user's, in `~/.slopslides/templates/<id>`. */
  builtin: boolean;
  /** Folder of a user template. */
  path: string | null;
  /** Slide ids, one per layout, in order. */
  slides: string[];
}

export interface CreatedSlide {
  deck: Deck;
  slide: string;
}

export interface UpdatedSlide {
  deck: Deck;
  /** The slide's markup before the update; saving it back undoes the update. */
  previous: string;
}

export interface LintIssue {
  /** Rule id, e.g. `unclosed-tag` (see src-tauri/src/lint.rs). */
  rule: string;
  severity: "error" | "warning";
  message: string;
  /** 1-based line in deck.html. */
  line: number;
  slide: string | null;
}

export type AgentEvent =
  | { type: "approvalRequested"; approval: Approval }
  | { type: "approvalResolved"; id: string }
  | { type: "approvalReview"; id: string; status: string; detail: string | null }
  | { type: "started"; sessionId: string | null }
  | { type: "thinking" }
  | { type: "textStart" }
  | { type: "textDelta"; text: string }
  | { type: "toolUse"; id: string; name: string; input: Record<string, unknown> }
  | { type: "toolResult"; id: string; isError: boolean }
  | {
      type: "result";
      isError: boolean;
      text: string | null;
      costUsd: number | null;
      durationMs: number | null;
    }
  | { type: "error"; message: string }
  /** A null field is not known from this event; keep what was known before. */
  | { type: "usage"; contextTokens: number | null; contextWindow: number | null }
  | { type: "compacting" }
  | { type: "compacted" }
  | { type: "finished"; interrupted: boolean };

export interface AgentEventEnvelope {
  deckId: string;
  event: AgentEvent;
}

export interface DeckChanged {
  deckId: string;
  paths: string[];
}

export const api = {
  prepareVideo: (id: string, jobId: string) => invoke<import("./video").VideoTimeline>("prepare_video", { id, jobId }),
  exportVideo: (jobId: string, dest: string) => invoke<void>("export_video", { jobId, dest }),
  cancelVideo: (jobId: string) => invoke<void>("cancel_video", { jobId }),
  releaseVideo: (jobId: string) => invoke<void>("release_video", { jobId }),
  listDecks: () => invoke<DeckSummary[]>("list_decks"),
  /** With a template, the new deck takes its styles (and names it). */
  createDeck: (title: string, template: string | null = null) => invoke<Deck>("create_deck", { title, template }),
  openDeck: (id: string) => invoke<Deck>("open_deck", { id }),
  closeDeck: () => invoke<void>("close_deck"),
  loadDeck: (id: string) => invoke<Deck>("load_deck", { id }),
  speechStatus: () => invoke<SpeechStatus>("speech_status"),
  speechTakes: (id: string) => invoke<Record<string, SpeechTake>>("speech_takes", { id }),
  speechHistory: (id: string, slide: string) => invoke<import("./speech").SpeechHistoryTake[]>("speech_history", { id, slide }),
  selectSpeechTake: (id: string, slide: string, takeId: string, base: string) => invoke<NarrationDocument>("select_speech_take", { id, slide, takeId, base }),
  installSpeechPack: (jobId: string, source: string | null, providerId: string) => invoke<void>("install_speech_pack", { jobId, source, providerId }),
  installCloningPack: (jobId: string, source: string | null, providerId: string) => invoke<void>("install_cloning_pack", { jobId, source, providerId }),
  createVoiceProfile: (jobId: string, providerId: string, request: { name: string; language: string; transcript: string; reference: string; authorized: boolean }) => invoke<import("./speech").VoiceProfile>("create_voice_profile", { jobId, providerId, request }),
  previewVoiceProfile: (jobId: string, providerId: string, token: string, language: string) => invoke<void>("preview_voice_profile", { jobId, providerId, token, language }),
  voiceProfileAction: (providerId: string, action: "save" | "discard" | "rename" | "delete", id: string, value: string | null = null) => invoke<import("./speech").VoiceProfile | null>("voice_profile_action", { providerId, action, id, value }),
  setDefaultPresenter: (choice: import("./speech").DefaultPresenter | null) => invoke<void>("set_default_presenter", { choice }),
  importVoiceRecording: (path: string) => invoke<{ id: string; path: string }>("import_voice_recording", { path }),
  stageVoiceRecording: (bytes: number[]) => invoke<{ id: string; path: string }>("stage_voice_recording", { bytes }),
  releaseVoiceRecording: (id: string) => invoke<void>("release_voice_recording", { id }),
  removeSpeechPack: (providerId: string) => invoke<void>("remove_speech_pack", { providerId }),
  generateSpeech: (jobId: string, id: string, slide: string | null, fresh = false) => invoke<SpeechResult>("generate_speech", { jobId, id, slide, fresh }),
  cancelSpeech: (jobId: string) => invoke<void>("cancel_speech", { jobId }),
  loadNarration: (id: string) => invoke<NarrationDocument>("load_narration", { id }),
  saveNarration: (id: string, manifest: NarrationManifest, base: string) => invoke<NarrationDocument>("save_narration", { id, manifest, base }),
  /** Stores the review marks (by slide id) in deck.html. */
  saveReview: (id: string, review: Record<string, Stroke[]>) => invoke<void>("save_review", { id, review }),
  renameDeck: (id: string, title: string) => invoke<Deck>("rename_deck", { id, title }),
  deleteDeck: (id: string) => invoke<void>("delete_deck", { id }),
  /** `slides` lists every slide id and section key (see `sectionKey`) in the new order. */
  reorderSlides: (id: string, slides: string[]) => invoke<Deck>("reorder_slides", { id, slides }),
  addSlide: (id: string, after: string | null) => invoke<CreatedSlide>("add_slide", { id, after }),
  duplicateSlide: (id: string, slide: string) =>
    invoke<CreatedSlide>("duplicate_slide", { id, slide }),
  setSlideHidden: (id: string, slide: string, hidden: boolean) =>
    invoke<Deck>("set_slide_hidden", { id, slide, hidden }),
  setSlideLocked: (id: string, slide: string, locked: boolean) =>
    invoke<Deck>("set_slide_locked", { id, slide, locked }),
  addSection: (id: string, before: string | null, title: string) =>
    invoke<Deck>("add_section", { id, before, title }),
  renameSection: (id: string, index: number, title: string) =>
    invoke<Deck>("rename_section", { id, index, title }),
  deleteSection: (id: string, index: number) => invoke<Deck>("delete_section", { id, index }),
  deleteSlide: (id: string, slide: string) => invoke<Deck>("delete_slide", { id, slide }),
  /** Replaces one slide's markup; refused when the slide's hash is no longer `base`. */
  updateSlide: (id: string, slide: string, markup: string, base: string) =>
    invoke<UpdatedSlide>("update_slide", { id, slide, markup, base }),
  saveDeckSource: (id: string, source: string, base: string | null) =>
    invoke<Deck>("save_deck_source", { id, source, base }),
  importAssets: (id: string, paths: string[]) => invoke<string[]>("import_assets", { id, paths }),
  /** Saves pasted file contents (base64) into the deck's assets; returns its `assets/…` ref. */
  saveAsset: (id: string, name: string, data: string) =>
    invoke<string>("save_asset", { id, name, data }),
  exportDeck: (id: string, dest: string) => invoke<void>("export_deck", { id, dest }),
  /** Creates a new folder named after the deck inside `parent`; returns its path. */
  createImageExportDir: (id: string, parent: string) =>
    invoke<string>("create_image_export_dir", { id, parent }),
  /** Screenshots `rect` (one slide, CSS pixels) as `<dir>/slide-NN.png`; returns its path. */
  exportSlideImage: (
    dir: string,
    index: number,
    total: number,
    rect: { x: number; y: number; width: number; height: number },
    viewport: { width: number; height: number },
  ) => invoke<string>("export_slide_image", { dir, index, total, rect, viewport }),
  lintDeck: (id: string) => invoke<LintIssue[]>("lint_deck", { id }),
  /**
   * Screenshots `rect` of the window; returns the deck-relative image path. Both are in CSS
   * pixels; the viewport size lets the backend work out the display's scale.
   */
  captureSketch: (
    id: string,
    rect: { x: number; y: number; width: number; height: number },
    viewport: { width: number; height: number },
  ) => invoke<string>("capture_sketch", { id, rect, viewport }),
  loadChat: (id: string) => invoke<unknown>("load_chat", { id }),
  saveChat: (id: string, chat: unknown) => invoke<void>("save_chat", { id, chat }),
  resetChat: (id: string) => invoke<void>("reset_chat", { id }),
  sendMessage: (
    deckId: string,
    prompt: string,
    selection: { provider: Provider; model: string; effort: string; contextWindow: string | null; permissionMode?: PermissionMode },
    /** Summarize the conversation so far instead of sending `prompt`. */
    compact = false,
  ) => invoke<void>("send_message", { args: { deckId, prompt, ...selection, compact } }),
  codexPermissionModes: (id: string) => invoke<PermissionMode[]>("codex_permission_modes", { id }),
  respondApproval: (deckId: string, id: string, decision: ApprovalDecision) =>
    invoke<void>("respond_approval", { deckId, id, decision }),
  interruptAgent: (id: string) => invoke<void>("interrupt_agent", { id }),
  agentRunning: (id: string) => invoke<boolean>("agent_running", { id }),
  listProviders: () => invoke<ProviderInfo[]>("list_providers"),
  /** The user's templates first, then the built-in ones. */
  listTemplates: () => invoke<TemplateSummary[]>("list_templates"),
  /** Copies the template into the deck's internals for the agent; returns its deck-relative path. */
  stageTemplate: (id: string, template: string) => invoke<string>("stage_template", { id, template }),
  /** Gives a deck without slides the template's styles. */
  applyTemplate: (id: string, template: string) => invoke<Deck>("apply_template", { id, template }),
  /** Inserts a copy of the template's slide after `after` (or at the end). */
  addTemplateSlide: (id: string, template: string, slide: string, after: string | null) =>
    invoke<CreatedSlide>("add_template_slide", { id, template, slide, after }),
  /** Saves the deck as a user template with placeholder text in place of its content. */
  createTemplate: (id: string, name: string) => invoke<TemplateSummary>("create_template", { id, name }),
};

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
