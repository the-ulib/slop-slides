import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";

import {
  api,
  errorMessage,
  type AgentEvent,
  type AgentEventEnvelope,
  type Deck,
  type DeckChanged,
  type LintIssue,
} from "./lib/api";
import { latestContext, mergeContext, type ContextUsage } from "./lib/context";
import { inkBounds, SLIDE_SIZE, type Stroke } from "./lib/ink";
import {
  defaultModel,
  pickContextWindow,
  pickEffort,
  requestEffort,
  type Provider,
  type ProviderInfo,
} from "./lib/models";

import { useNarration } from "./narrationStore";
import { editedManifest, narrationDraftPrompt } from "./lib/narration";

export type ChatPart =
  | { kind: "text"; text: string }
  | {
      kind: "tool";
      id: string;
      name: string;
      input: Record<string, unknown>;
      status: "running" | "done" | "error";
    };

export interface UserMessage {
  id: string;
  role: "user";
  text: string;
  slide: string | null;
  attachments: string[];
  /** The user drew on the slide before sending; absent in chats saved before sketches. */
  sketch?: Sketch | null;
  /** Deck-relative path of a screenshot of the slide sent along (for tidying hand edits). */
  screenshot?: string | null;
  /** Set when the message ran a command instead of prompting the agent. */
  command?: "compact";
  createdAt: number;
}

/** What the agent is told about a drawing on the current slide. */
export interface Sketch {
  /** Deck-relative path of the slide screenshot with the drawing; null if it failed. */
  image: string | null;
  /** The marked area in slide pixels. */
  bounds: { left: number; top: number; right: number; bottom: number };
}

export interface AssistantMessage {
  id: string;
  role: "assistant";
  parts: ChatPart[];
  status: "streaming" | "done" | "interrupted" | "error";
  thinking: boolean;
  error: string | null;
  costUsd: number | null;
  durationMs: number | null;
  /** The provider that wrote this reply; absent in chats saved before it was recorded. */
  provider?: Provider;
  /** How full the provider's context was after this reply, as far as it was reported. */
  context?: ContextUsage | null;
  /** The agent is summarizing the conversation. */
  compacting?: boolean;
  /** The conversation was summarized during this reply. */
  compacted?: boolean;
  createdAt: number;
}

export type ChatMessage = UserMessage | AssistantMessage;

export interface ModelSelection {
  provider: Provider;
  model: string;
  /** Shown until the provider list arrives. */
  label: string;
  effort: string;
  /** Claude only; `null` when the model has a single context window. */
  contextWindow: string | null;
}

const SELECTION_KEY = "slopslide.selection";
const REVIEW_VISIBLE_KEY = "slopslide.reviewVisible";
const FAVORITES_KEY = "slopslide.favoriteModels";

const DEFAULT_SELECTION: ModelSelection = {
  provider: "claude",
  model: "claude-opus-5-5",
  label: "Claude Opus 5.5",
  effort: "medium",
  contextWindow: "1m",
};

function loadSelection(): ModelSelection {
  try {
    const saved = JSON.parse(localStorage.getItem(SELECTION_KEY) ?? "null") as Partial<ModelSelection> | null;
    if (saved?.provider && saved.model && saved.label && saved.effort) {
      return { ...saved, contextWindow: saved.contextWindow ?? null } as ModelSelection;
    }
  } catch {
    // Fall through to the default.
  }
  return DEFAULT_SELECTION;
}

function saveSelection(selection: ModelSelection) {
  localStorage.setItem(SELECTION_KEY, JSON.stringify(selection));
  useApp.setState({ selection });
}

/** Moves a selection that is no longer offered onto an installed provider's default model. */
function reconcileSelection(selection: ModelSelection, providers: ProviderInfo[]): ModelSelection {
  const current = providers.find((p) => p.id === selection.provider);
  const model = current?.models.find((m) => m.id === selection.model);
  if (model) {
    return {
      ...selection,
      label: model.label,
      effort: pickEffort(model, selection.effort),
      contextWindow: pickContextWindow(model, selection.contextWindow),
    };
  }
  // A provider whose models could not be listed keeps the saved choice rather than losing it.
  if (current?.installed && current.error) return selection;
  const fallback = [current, ...providers].find((p) => p?.installed && p.models.length > 0);
  const next = fallback && defaultModel(fallback);
  if (!fallback || !next) return selection;
  return {
    provider: fallback.id,
    model: next.id,
    label: next.label,
    effort: pickEffort(next, selection.effort),
    contextWindow: pickContextWindow(next, null),
  };
}

function loadFavorites(): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(FAVORITES_KEY) ?? "[]");
    return Array.isArray(saved) ? saved.filter((k): k is string => typeof k === "string") : [];
  } catch {
    return [];
  }
}

export type StageView = "slides" | "code";

interface AppState {
  deck: Deck | null;
  /** Id of the selected slide. */
  selected: string | null;
  /** Bumped whenever the user picks a slide, so views can reveal it even if unchanged. */
  revealRev: number;
  /** Whether the stage shows the rendered slide or deck.html's source. */
  view: StageView;
  /** The HTML view holds edits that are not saved to deck.html yet. */
  codeDirty: boolean;
  /** The chat panel is shown; the user can collapse it to give the stage more room. */
  chatOpen: boolean;
  sidebarTab: "chat" | "narration";
  narrationReviewSlide: string | null;
  reviewNarration: () => void;
  setSidebarTab: (tab: "chat" | "narration") => void;
  draftNarration: (scope: "slide" | "deck", audience: string, minutes: string) => Promise<void>;
  /** Bumped when attached assets change, reloading every slide preview. */
  assetsRev: number;
  messages: ChatMessage[];
  running: boolean;
  selection: ModelSelection;
  /** `provider:model` keys starred in the model picker. */
  favoriteModels: string[];
  presenting: boolean;
  /** Installed agent CLIs and their models; `undefined` while loading. */
  providers: ProviderInfo[] | undefined;
  error: string | null;
  /** Lint result for the saved deck.html; null until the first check finishes. */
  lint: LintIssue[] | null;
  /** Text to put in the chat composer, with a counter so the same text can be sent twice. */
  composerFill: { text: string; rev: number } | null;
  /**
   * Ink drawn on slides in the editor, by slide id. Saved in deck.html as review marks, and
   * sent along with the next message about the slide while it has changed since last sent.
   */
  sketches: Record<string, Stroke[]>;
  /** The marks each slide had when they were last sent (or skipped), so they go out once. */
  sketchesSent: Record<string, Stroke[]>;
  /** Review marks are shown on the stage (and sent to the agent). */
  reviewVisible: boolean;
  /** Slides being saved as images into `dir`, one at a time; null when not exporting. */
  imageExport: { dir: string; slides: string[] } | null;
  /** The stage lets the user edit text and move elements on the slide. */
  editing: boolean;
  /** Bumped when a slide edit could not be saved, so the stage reloads the slide from disk. */
  editReload: number;
  /** Slide edits that can be undone, newest last. */
  slideUndo: SlideUndo[];
  /** Undone slide edits that can be redone, newest last. */
  slideRedo: SlideUndo[];

  openDeck: (id: string) => Promise<void>;
  createDeck: (title: string) => Promise<void>;
  closeDeck: () => Promise<void>;
  setDeck: (deck: Deck) => void;
  select: (slide: string | null) => void;
  selectRelative: (delta: number) => void;
  setView: (view: StageView) => void;
  setCodeDirty: (dirty: boolean) => void;
  setChatOpen: (open: boolean) => void;
  setModel: (provider: Provider, model: string) => void;
  setEffort: (effort: string) => void;
  setContextWindow: (contextWindow: string) => void;
  refreshProviders: () => Promise<void>;
  toggleFavoriteModel: (key: string) => void;
  setPresenting: (presenting: boolean) => void;
  setError: (error: string | null) => void;
  refreshLint: () => Promise<void>;
  fillComposer: (text: string) => void;
  setSketches: (update: (all: Record<string, Stroke[]>) => Record<string, Stroke[]>) => void;
  clearSketch: (slide: string) => void;
  /** Keeps the slide's current marks out of the next message. */
  skipSketch: (slide: string) => void;
  setReviewVisible: (visible: boolean) => void;
  startImageExport: (dir: string) => void;
  endImageExport: () => void;
  /** Enters or leaves edit mode, keeping the edits made so far. */
  setEditing: (editing: boolean) => void;
  /** Saves `markup` as the new version of `slide`, after any edits still being saved. */
  saveSlideEdit: (slide: string, markup: string) => Promise<void>;
  undoSlideEdit: () => Promise<void>;
  redoSlideEdit: () => Promise<void>;
  /** Undoes every edit made since entering edit mode, then leaves it. */
  discardSlideEdits: () => Promise<void>;
  /** Asks the agent to rebuild the current slide's layout around the user's hand edits. */
  /** `overflow` lists elements the slide editor found running past the slide or cut off. */
  tidyLayout: (overflow?: string[]) => Promise<void>;
  send: (
    text: string,
    options: { includeSlide: boolean; attachments: string[]; screenshot?: boolean },
  ) => Promise<void>;
  /** Has the agent summarize the conversation so far, freeing up its context. */
  compact: () => Promise<void>;
  interrupt: () => void;
  resetChat: () => Promise<void>;
}

interface SlideUndo {
  slide: string;
  /** Markup to restore. */
  markup: string;
  /** The slide's hash right after the edit; undo is refused once the slide changed again. */
  after: string;
}

const UNDO_KEPT = 50;

export const TIDY_PROMPT =
  "Tidy up the layout of this slide. I edited it by hand: keep my text and keep things where I moved them, at the size and angle I gave them, but rebuild the layout cleanly and fix any overflow or clipping.";

export const tidyPrompt = (overflow: string[] = []) =>
  overflow.length ? `${TIDY_PROMPT}\n\nThe editor found overflow:\n${overflow.map((o) => `- ${o}`).join("\n")}` : TIDY_PROMPT;

const newId = () => crypto.randomUUID();
let lintRun = 0;
/** Slide edits are saved one after another, each based on the previous one's result. */
let editQueue: Promise<void> = Promise.resolve();

export const useApp = create<AppState>((set, get) => ({
  deck: null,
  selected: null,
  revealRev: 0,
  view: localStorage.getItem("slopslide.view") === "code" ? "code" : "slides",
  codeDirty: false,
  chatOpen: localStorage.getItem("slopslide.chatOpen") !== "false",
  sidebarTab: localStorage.getItem("slopslide.sidebarTab") === "narration" ? "narration" : "chat",
  narrationReviewSlide: null,
  assetsRev: 0,
  messages: [],
  running: false,
  selection: loadSelection(),
  favoriteModels: loadFavorites(),
  presenting: false,
  providers: undefined,
  error: null,
  lint: null,
  composerFill: null,
  sketches: {},
  sketchesSent: {},
  reviewVisible: localStorage.getItem(REVIEW_VISIBLE_KEY) !== "false",
  imageExport: null,
  editing: false,
  editReload: 0,
  slideUndo: [],
  slideRedo: [],

  openDeck: async (id) => {
    try {
      if (!(await useNarration.getState().save())) { get().setSidebarTab("narration"); return; }
      const deck = await api.openDeck(id);
      await loadDeckState(deck);
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  createDeck: async (title) => {
    try {
      if (!(await useNarration.getState().save())) { get().setSidebarTab("narration"); return; }
      const deck = await api.createDeck(title);
      await loadDeckState(deck);
    } catch (error) {
      set({ error: errorMessage(error) });
    }
  },

  closeDeck: async () => {
    if (get().codeDirty && !(await confirmDiscardEdits())) return;
    if (!(await useNarration.getState().save())) { get().setSidebarTab("narration"); return; }
    await flushReviewSave();
    await api.closeDeck();
    await useNarration.getState().load(null);
    set({ codeDirty: false, deck: null, selected: null, messages: [], running: false, presenting: false, lint: null, composerFill: null, sketches: {}, sketchesSent: {}, imageExport: null, editing: false, slideUndo: [], slideRedo: [] });
  },

  setDeck: (deck) => {
    const { selected } = get();
    const stillThere = deck.slides.some((s) => s.id === selected);
    set({ deck, selected: stillThere ? selected : (deck.slides[0]?.id ?? null) });
  },

  select: (slide) => set((s) => ({ selected: slide, revealRev: s.revealRev + 1 })),

  selectRelative: (delta) => {
    const { deck, selected } = get();
    if (!deck || deck.slides.length === 0) return;
    const index = deck.slides.findIndex((s) => s.id === selected);
    const next = Math.min(deck.slides.length - 1, Math.max(0, index + delta));
    set((s) => ({ selected: deck.slides[next]?.id ?? null, revealRev: s.revealRev + 1 }));
  },

  setView: (view) => {
    localStorage.setItem("slopslide.view", view);
    set({ view });
  },

  setCodeDirty: (codeDirty) => set({ codeDirty }),

  setChatOpen: (chatOpen) => {
    localStorage.setItem("slopslide.chatOpen", String(chatOpen));
    set({ chatOpen });
  },

  setSidebarTab: (sidebarTab) => {
    localStorage.setItem("slopslide.sidebarTab", sidebarTab);
    get().setChatOpen(true);
    set({ sidebarTab });
  },

  reviewNarration: () => {
    const { deck, narrationReviewSlide } = get();
    if (narrationReviewSlide && deck?.slides.some((s) => s.id === narrationReviewSlide)) get().select(narrationReviewSlide);
    get().setSidebarTab("narration");
  },

  draftNarration: async (scope, audience, minutes) => {
    const { deck, selected, running } = get();
    if (!deck || running || (scope === "slide" && (!selected || selected.startsWith("#")))) return;
    if (!(await useNarration.getState().save())) { get().setSidebarTab("narration"); return; }
    if (get().deck?.id !== deck.id || get().running) return;
    const { document, edits, languageEdit } = useNarration.getState();
    if (!document || useNarration.getState().deckId !== deck.id || useNarration.getState().error) return;
    const manifest = editedManifest(document.manifest, edits, languageEdit);
    set({ narrationReviewSlide: scope === "slide" ? selected : deck.slides.find((s) => !s.hidden && !s.id.startsWith("#"))?.id ?? null });
    get().setSidebarTab("chat");
    await get().send(narrationDraftPrompt(deck, selected, scope, manifest.defaultLanguage, audience, minutes), { includeSlide: scope === "slide", attachments: [] });
  },

  setModel: (provider, id) => {
    const model = get()
      .providers?.find((p) => p.id === provider)
      ?.models.find((m) => m.id === id);
    if (!model) return;
    const effort = pickEffort(model, get().selection.effort);
    // Each model starts on its own default window: 1M costs more on models where it is optional.
    const contextWindow = pickContextWindow(model, null);
    saveSelection({ provider, model: id, label: model.label, effort, contextWindow });
  },

  setEffort: (effort) => saveSelection({ ...get().selection, effort }),

  setContextWindow: (contextWindow) => saveSelection({ ...get().selection, contextWindow }),

  refreshProviders: async () => {
    set({ providers: undefined });
    const result = await api.listProviders().catch(() => null);
    const providers = Array.isArray(result) ? result : [];
    set({ providers });
    if (providers.length > 0) saveSelection(reconcileSelection(get().selection, providers));
  },

  toggleFavoriteModel: (key) => {
    const { favoriteModels } = get();
    const next = favoriteModels.includes(key)
      ? favoriteModels.filter((k) => k !== key)
      : [...favoriteModels, key];
    localStorage.setItem(FAVORITES_KEY, JSON.stringify(next));
    set({ favoriteModels: next });
  },

  setPresenting: (presenting) => set({ presenting }),

  setError: (error) => set({ error }),

  refreshLint: async () => {
    const { deck } = get();
    if (!deck) return;
    const run = ++lintRun;
    try {
      const issues = await api.lintDeck(deck.id);
      // Ignore answers that a newer check (or another deck) has overtaken.
      if (run === lintRun && get().deck?.id === deck.id) set({ lint: Array.isArray(issues) ? issues : null });
    } catch {
      if (run === lintRun) set({ lint: null });
    }
  },

  fillComposer: (text) => set((s) => ({ sidebarTab: "chat", chatOpen: true, composerFill: { text, rev: (s.composerFill?.rev ?? 0) + 1 } })),

  setSketches: (update) => {
    set((s) => ({ sketches: update(s.sketches) }));
    scheduleReviewSave();
  },

  clearSketch: (slide) => {
    set((s) => {
      const { [slide]: _, ...rest } = s.sketches;
      return { sketches: rest };
    });
    scheduleReviewSave();
  },

  skipSketch: (slide) => set((s) => ({ sketchesSent: { ...s.sketchesSent, [slide]: s.sketches[slide] ?? [] } })),

  setReviewVisible: (reviewVisible) => {
    localStorage.setItem(REVIEW_VISIBLE_KEY, String(reviewVisible));
    set({ reviewVisible });
  },

  startImageExport: (dir) => {
    const { deck } = get();
    if (deck && deck.slides.length > 0) set({ imageExport: { dir, slides: deck.slides.map((s) => s.id) } });
  },

  endImageExport: () => set({ imageExport: null }),

  setEditing: (editing) => {
    if (editing === get().editing) return;
    // Each edit session starts with fresh history; leaving keeps the edits.
    set({ editing, slideUndo: [], slideRedo: [] });
  },

  saveSlideEdit: (slide, markup) => {
    const save = async () => {
      const { deck } = get();
      const base = deck?.slides.find((s) => s.id === slide)?.hash;
      if (!deck || !base) return;
      try {
        const { deck: next, previous } = await api.updateSlide(deck.id, slide, markup, base);
        if (get().deck?.id !== deck.id) return;
        const after = next.slides.find((s) => s.id === slide)?.hash ?? "";
        get().setDeck(next);
        set((s) => ({ slideUndo: [...s.slideUndo, { slide, markup: previous, after }].slice(-UNDO_KEPT), slideRedo: [] }));
      } catch (error) {
        set((s) => ({ error: errorMessage(error), editReload: s.editReload + 1 }));
      }
    };
    editQueue = editQueue.then(save);
    return editQueue;
  },

  undoSlideEdit: () => {
    editQueue = editQueue.then(() => stepSlideHistory("slideUndo", "slideRedo")).then(() => undefined);
    return editQueue;
  },

  redoSlideEdit: () => {
    editQueue = editQueue.then(() => stepSlideHistory("slideRedo", "slideUndo")).then(() => undefined);
    return editQueue;
  },

  discardSlideEdits: () => {
    const discard = async () => {
      while (get().slideUndo.length > 0) {
        if (!(await stepSlideHistory("slideUndo", "slideRedo"))) break;
      }
      set({ editing: false, slideUndo: [], slideRedo: [] });
    };
    editQueue = editQueue.then(discard);
    return editQueue;
  },

  tidyLayout: async (overflow = []) => {
    // Let any edit still being saved land first, so the agent sees the final version.
    await editQueue;
    await get().send(tidyPrompt(overflow), { includeSlide: true, attachments: [], screenshot: true });
  },

  send: async (text, { includeSlide, attachments, screenshot = false }) => {
    const { deck, selected, running, selection } = get();
    if (!deck || running) return;
    // Typed as a message, the command still compacts rather than reaching the agent as text.
    if (text.trim() === COMPACT_COMMAND) return get().compact();
    const slide = includeSlide ? selected : null;
    const strokes = slide ? (get().sketches[slide] ?? []) : [];
    // Marks go out once, while they are on show; they stay on the slide as a review.
    const unsent = !!slide && get().reviewVisible && strokes !== get().sketchesSent[slide];
    const bounds = unsent ? inkBounds(strokes) : null;
    let user: UserMessage = {
      id: newId(),
      role: "user",
      text,
      slide,
      attachments,
      sketch: bounds ? { image: null, bounds } : null,
      createdAt: Date.now(),
    };
    const assistant = newReply(selection.provider);
    set((s) => ({ messages: [...s.messages, user, assistant], running: true }));
    if (slide && (user.sketch || screenshot)) {
      // Screenshot the slide with the ink on it.
      const image = await captureSlide(deck.id);
      const captured: UserMessage = {
        ...user,
        ...(user.sketch && { sketch: { ...user.sketch, image } }),
        ...(screenshot && { screenshot: image }),
      };
      user = captured;
      set((s) => ({ messages: s.messages.map((m) => (m.id === captured.id ? captured : m)) }));
      if (user.sketch) set((s) => ({ sketchesSent: { ...s.sketchesSent, [slide]: strokes } }));
    }
    await startTurn(deck.id, assistant.id, buildPrompt(deck, user), false);
  },

  compact: async () => {
    const { deck, running, selection } = get();
    if (!deck || running) return;
    const user: UserMessage = {
      id: newId(),
      role: "user",
      text: COMPACT_COMMAND,
      slide: null,
      attachments: [],
      command: "compact",
      createdAt: Date.now(),
    };
    const assistant = newReply(selection.provider);
    set((s) => ({ messages: [...s.messages, user, assistant], running: true }));
    await startTurn(deck.id, assistant.id, COMPACT_COMMAND, true);
  },

  interrupt: () => {
    const { deck } = get();
    if (deck) void api.interruptAgent(deck.id);
  },

  resetChat: async () => {
    const { deck } = get();
    if (!deck) return;
    await api.resetChat(deck.id);
    set({ messages: [], running: false });
  },
}));

export const COMPACT_COMMAND = "/compact";

function newReply(provider: Provider): AssistantMessage {
  return {
    id: newId(),
    role: "assistant",
    parts: [],
    status: "streaming",
    thinking: true,
    error: null,
    costUsd: null,
    durationMs: null,
    provider,
    createdAt: Date.now(),
  };
}

/** Hands `prompt` to the selected agent; a failure to start lands on reply `replyId`. */
async function startTurn(deckId: string, replyId: string, prompt: string, compact: boolean) {
  const { selection, providers } = useApp.getState();
  try {
    if (!(await useNarration.getState().save())) {
      useApp.getState().setSidebarTab("narration");
      throw new Error("Save or resolve your narration edits before starting the agent.");
    }
    if (useApp.getState().deck?.id !== deckId) return;
    const { provider, model, contextWindow } = selection;
    const info = providers?.find((p) => p.id === provider)?.models.find((m) => m.id === model);
    const effort = requestEffort(info, selection.effort);
    await api.sendMessage(deckId, prompt, { provider, model, effort, contextWindow }, compact);
  } catch (error) {
    updateAssistant(replyId, (m) => ({
      ...m,
      status: "error",
      thinking: false,
      error: errorMessage(error),
    }));
    useApp.setState({ running: false });
    persistChat();
  }
}

/**
 * Restores the newest entry of one slide edit history (undo or redo) and records how to
 * reverse that in the other. False when there was nothing to restore or it was refused.
 */
async function stepSlideHistory(from: "slideUndo" | "slideRedo", to: "slideUndo" | "slideRedo"): Promise<boolean> {
  const { deck, [from]: history } = useApp.getState();
  const last = history.at(-1);
  if (!deck || !last) return false;
  useApp.setState({ [from]: history.slice(0, -1) });
  const current = deck.slides.find((s) => s.id === last.slide)?.hash;
  if (current !== last.after) {
    useApp.setState({ error: `The slide changed since that edit, so it cannot be ${from === "slideUndo" ? "undone" : "redone"}.` });
    return false;
  }
  try {
    const { deck: next, previous } = await api.updateSlide(deck.id, last.slide, last.markup, current);
    if (useApp.getState().deck?.id !== deck.id) return false;
    const after = next.slides.find((s) => s.id === last.slide)?.hash ?? "";
    useApp.getState().setDeck(next);
    useApp.getState().select(last.slide);
    useApp.setState((s) => ({ [to]: [...s[to], { slide: last.slide, markup: previous, after }].slice(-UNDO_KEPT) }));
    return true;
  } catch (error) {
    useApp.setState({ error: errorMessage(error) });
    return false;
  }
}

async function confirmDiscardEdits(): Promise<boolean> {
  const message = "You have unsaved changes to deck.html. Discard them?";
  try {
    return await ask(message, { title: "Unsaved changes", kind: "warning", okLabel: "Discard" });
  } catch {
    return window.confirm(message);
  }
}

async function loadDeckState(deck: Deck) {
  const [chat, running] = await Promise.all([api.loadChat(deck.id), api.agentRunning(deck.id), useNarration.getState().load(deck.id)]);
  savedReview = reviewKey(deck.review ?? {});
  const messages = Array.isArray(chat) ? (chat as ChatMessage[]) : [];
  useApp.setState({
    deck,
    narrationReviewSlide: null,
    selected: deck.slides[0]?.id ?? null,
    assetsRev: 0,
    messages: messages.map(settleInterrupted),
    running,
    presenting: false,
    lint: null,
    composerFill: null,
    sketches: deck.review ?? {},
    sketchesSent: {},
    imageExport: null,
    editing: false,
    slideUndo: [],
    slideRedo: [],
  });
}

const REVIEW_SAVE_DELAY = 400;
let reviewSave: { deckId: string; timer: ReturnType<typeof setTimeout> } | null = null;
/** Review saves still on their way to disk. */
let reviewSaving = 0;
/** The review marks as deck.html holds them, as far as the app knows (see `reviewKey`). */
let savedReview = reviewKey({});

/** Compares review marks independent of slide order; slides without marks don't count. */
function reviewKey(review: Record<string, Stroke[]>): string {
  return JSON.stringify(
    Object.entries(review)
      .filter(([, strokes]) => strokes.length > 0)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}

/**
 * Takes on review marks changed outside the app's own saves (the agent clearing them, an
 * edit in the HTML view), unless the user's latest marks are still being saved.
 */
function adoptReview(deck: Deck) {
  const review = deck.review ?? {};
  const key = reviewKey(review);
  if (reviewSave || reviewSaving > 0 || key === savedReview) return;
  savedReview = key;
  if (key !== reviewKey(useApp.getState().sketches)) useApp.setState({ sketches: review, sketchesSent: {} });
}

/** Saves the review marks to deck.html once drawing pauses. */
function scheduleReviewSave() {
  const deck = useApp.getState().deck;
  if (!deck) return;
  if (reviewSave && reviewSave.deckId !== deck.id) void flushReviewSave();
  if (reviewSave) clearTimeout(reviewSave.timer);
  reviewSave = { deckId: deck.id, timer: setTimeout(() => void flushReviewSave(), REVIEW_SAVE_DELAY) };
}

/** Saves review marks still waiting for drawing to pause, right away. */
export async function flushReviewSave() {
  if (!reviewSave) return;
  const { deckId, timer } = reviewSave;
  clearTimeout(timer);
  reviewSave = null;
  const { deck, sketches } = useApp.getState();
  if (deck?.id !== deckId) return;
  const review = Object.fromEntries(Object.entries(sketches).filter(([, strokes]) => strokes.length > 0));
  // Drawing and undoing back to where the file is leaves nothing to save.
  if (reviewKey(review) === savedReview) return;
  reviewSaving++;
  try {
    await api.saveReview(deckId, review);
    savedReview = reviewKey(review);
  } catch (error) {
    useApp.setState({ error: `Could not save the review marks: ${errorMessage(error)}` });
  } finally {
    reviewSaving--;
  }
}

/** Marks the slide on the stage to screenshot when sending a sketch. */
export const SKETCH_TARGET_ATTR = "data-sketch-target";

/** Screenshot of the slide on the stage, ink included; null when it cannot be taken. */
async function captureSlide(deckId: string): Promise<string | null> {
  const target = document.querySelector(`[${SKETCH_TARGET_ATTR}]`);
  if (!target) return null;
  const { x, y, width, height } = target.getBoundingClientRect();
  try {
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    return await api.captureSketch(deckId, { x, y, width, height }, viewport);
  } catch (error) {
    // The marked area still tells the agent where to look.
    console.warn("slide screenshot failed:", errorMessage(error));
    return null;
  }
}

/** Chat instructions asking the agent to fix `issues` and verify with its lint tool. */
export function lintFixPrompt(issues: LintIssue[]): string {
  const lines = issues.map(
    (i) => `- line ${i.line} ${i.severity} [${i.rule}]${i.slide ? ` (slide \`${i.slide}\`)` : ""}: ${i.message}`,
  );
  return [
    "deck.html does not pass the HTML lint. Fix these issues without changing how the slides look:",
    "",
    ...lines,
    "",
    "Then run the lint_deck tool to verify, and repeat until it reports no issues.",
  ].join("\n");
}

/** A transcript saved mid-turn (app quit) cannot resume streaming. */
function settleInterrupted(message: ChatMessage): ChatMessage {
  if (message.role !== "assistant" || message.status !== "streaming") return message;
  return { ...message, status: "interrupted", thinking: false, compacting: false };
}

function buildPrompt(deck: Deck, message: UserMessage): string {
  const context: string[] = [];
  if (message.slide) {
    const index = deck.slides.findIndex((s) => s.id === message.slide);
    context.push(
      `Current slide: <section id="${message.slide}"> in deck.html (slide ${index + 1} of ${deck.slides.length})`,
    );
  } else if (deck.slides.length === 0) {
    context.push("The deck has no slides yet.");
  }
  if (message.attachments.length > 0) {
    context.push(`Attached files: ${message.attachments.join(", ")}`);
  }
  if (message.sketch) {
    const { image, bounds } = message.sketch;
    if (image) {
      context.push(`Sketch: ${image} (screenshot of the current slide with the user's marks drawn on top)`);
    }
    context.push(
      `Marked area: x ${bounds.left}–${bounds.right}, y ${bounds.top}–${bounds.bottom} of the ${SLIDE_SIZE.width}×${SLIDE_SIZE.height} slide`,
    );
  }
  if (message.screenshot) {
    context.push(`Screenshot: ${message.screenshot} (the current slide as it looks now, with the user's hand edits)`);
  }
  if (context.length === 0) return message.text;
  return `[context]\n${context.join("\n")}\n[/context]\n\n${message.text}`;
}

function updateAssistant(id: string, update: (m: AssistantMessage) => AssistantMessage) {
  useApp.setState((s) => ({
    messages: s.messages.map((m) => (m.id === id && m.role === "assistant" ? update(m) : m)),
  }));
}

function updateLastAssistant(update: (m: AssistantMessage) => AssistantMessage) {
  const last = useApp.getState().messages.findLast((m) => m.role === "assistant");
  if (last) updateAssistant(last.id, update);
}

function replyProvider(message: AssistantMessage): Provider {
  return message.provider ?? useApp.getState().selection.provider;
}

function persistChat() {
  const { deck, messages } = useApp.getState();
  if (deck) void api.saveChat(deck.id, messages);
}

function applyAgentEvent(event: AgentEvent) {
  switch (event.type) {
    case "started":
      return;
    case "thinking":
      return updateLastAssistant((m) => ({ ...m, thinking: true }));
    case "textStart":
      return updateLastAssistant((m) => ({
        ...m,
        thinking: false,
        parts: [...m.parts, { kind: "text", text: "" }],
      }));
    case "textDelta":
      return updateLastAssistant((m) => {
        const parts = [...m.parts];
        const last = parts.at(-1);
        if (last?.kind === "text") parts[parts.length - 1] = { ...last, text: last.text + event.text };
        else parts.push({ kind: "text", text: event.text });
        return { ...m, thinking: false, parts };
      });
    case "toolUse":
      return updateLastAssistant((m) => ({
        ...m,
        thinking: false,
        parts: [
          ...m.parts.filter((p) => p.kind !== "text" || p.text.trim() !== ""),
          { kind: "tool", id: event.id, name: event.name, input: event.input, status: "running" },
        ],
      }));
    case "toolResult":
      return updateLastAssistant((m) => ({
        ...m,
        parts: m.parts.map((p) =>
          p.kind === "tool" && p.id === event.id
            ? { ...p, status: event.isError ? "error" : "done" }
            : p,
        ),
      }));
    case "result":
      return updateLastAssistant((m) => {
        const hasText = m.parts.some((p) => p.kind === "text" && p.text.trim() !== "");
        const parts: ChatPart[] =
          !hasText && !event.isError && event.text
            ? [...m.parts, { kind: "text", text: event.text }]
            : m.parts;
        return {
          ...m,
          parts,
          costUsd: event.costUsd,
          durationMs: event.durationMs,
          error: event.isError ? (event.text ?? "The agent reported an error.") : m.error,
        };
      });
    case "error":
      return updateLastAssistant((m) => ({ ...m, error: event.message }));
    case "usage":
      return updateLastAssistant((m) => {
        const provider = replyProvider(m);
        return { ...m, context: mergeContext(latestContext(useApp.getState().messages, provider), provider, event) };
      });
    case "compacting":
      return updateLastAssistant((m) => ({ ...m, thinking: false, compacting: true }));
    case "compacted":
      return updateLastAssistant((m) => {
        const provider = replyProvider(m);
        const window = latestContext(useApp.getState().messages, provider)?.window ?? null;
        // The new size is only known once the agent reports usage again.
        return { ...m, compacting: false, compacted: true, context: { provider, tokens: null, window } };
      });
    case "finished":
      updateLastAssistant((m) => ({
        ...m,
        thinking: false,
        compacting: false,
        status: event.interrupted ? "interrupted" : m.error ? "error" : "done",
        parts: m.parts.map((p) =>
          p.kind === "tool" && p.status === "running" ? { ...p, status: "done" } : p,
        ),
      }));
      useApp.setState({ running: false });
      persistChat();
      void useNarration.getState().refresh();
      return;
  }
}

let reloadTimer: ReturnType<typeof setTimeout> | undefined;

function applyDeckChanged(paths: string[]) {
  if (paths.includes("narration.json")) void useNarration.getState().refresh();
  if (paths.some((p) => p.startsWith("assets/"))) {
    useApp.setState((s) => ({ assetsRev: s.assetsRev + 1 }));
  }
  if (!paths.includes("deck.html")) return;
  // Edits arrive in bursts while the agent works; reload once they settle.
  clearTimeout(reloadTimer);
  reloadTimer = setTimeout(async () => {
    const { deck } = useApp.getState();
    if (!deck) return;
    try {
      const next = await api.loadDeck(deck.id);
      if (useApp.getState().deck?.id !== deck.id) return;
      const before = new Map(deck.slides.map((s) => [s.id, s.hash]));
      const changed = next.slides.find((s) => before.get(s.id) !== s.hash);
      useApp.getState().setDeck(next);
      adoptReview(next);
      // Follow the agent to the slide it is working on.
      if (changed && useApp.getState().running) useApp.setState({ selected: changed.id });
    } catch {
      // deck.html is mid-write; the next change event retries.
    }
  }, 120);
}

export async function initEventBridge() {
  // Listing Codex models starts its app server; don't hold up the rest of the bridge.
  void useApp.getState().refreshProviders();
  await listen<AgentEventEnvelope>("agent-event", ({ payload }) => {
    if (payload.deckId === useApp.getState().deck?.id) applyAgentEvent(payload.event);
  });
  await listen<DeckChanged>("deck-changed", ({ payload }) => {
    if (payload.deckId === useApp.getState().deck?.id) applyDeckChanged(payload.paths);
  });
}
