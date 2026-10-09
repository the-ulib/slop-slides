import { invoke } from "@tauri-apps/api/core";

import type { NarrationDocument, NarrationManifest } from "./narration";
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
  listDecks: () => invoke<DeckSummary[]>("list_decks"),
  createDeck: (title: string) => invoke<Deck>("create_deck", { title }),
  openDeck: (id: string) => invoke<Deck>("open_deck", { id }),
  closeDeck: () => invoke<void>("close_deck"),
  loadDeck: (id: string) => invoke<Deck>("load_deck", { id }),
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
    selection: { provider: Provider; model: string; effort: string; contextWindow: string | null },
    /** Summarize the conversation so far instead of sending `prompt`. */
    compact = false,
  ) => invoke<void>("send_message", { args: { deckId, prompt, ...selection, compact } }),
  interruptAgent: (id: string) => invoke<void>("interrupt_agent", { id }),
  agentRunning: (id: string) => invoke<boolean>("agent_running", { id }),
  listProviders: () => invoke<ProviderInfo[]>("list_providers"),
};

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
