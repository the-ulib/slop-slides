import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const ask = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
/** Handlers registered through `listen`, by event name. */
const listeners = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return () => {};
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: (...args: unknown[]) => ask(...args) }));

import type { AgentEvent, Deck } from "./lib/api";
import type { ProviderInfo } from "./lib/models";
import type { AssistantMessage, ChatMessage, UserMessage } from "./store";
import { DECK_HTML, deckFor } from "./test/fixtures";

async function freshStore() {
  return (await freshModule()).useApp;
}

async function freshModule() {
  vi.resetModules();
  return import("./store");
}

type Handler = (args: Record<string, unknown>) => unknown;

/** Answers `invoke(command, args)` from a table; unknown commands resolve to undefined. */
function backend(handlers: Record<string, Handler>) {
  invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => handlers[command]?.(args ?? {}));
}

const calls = (command: string) =>
  invoke.mock.calls.filter(([c]) => c === command).map(([, args]) => args as Record<string, unknown>);

beforeEach(() => {
  invoke.mockReset().mockResolvedValue(undefined);
  ask.mockReset();
  listeners.clear();
});

describe("slide selection", () => {
  it("select changes the slide and bumps revealRev, even for the same slide", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML) });
    const rev = useApp.getState().revealRev;
    useApp.getState().select("outro");
    expect(useApp.getState().selected).toBe("outro");
    expect(useApp.getState().revealRev).toBe(rev + 1);
    // Clicking the selected thumbnail again must still reveal it in the HTML view.
    useApp.getState().select("outro");
    expect(useApp.getState().revealRev).toBe(rev + 2);
  });

  it("selectRelative moves within bounds and bumps revealRev", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    useApp.getState().selectRelative(1);
    expect(useApp.getState().selected).toBe("#2");
    useApp.getState().selectRelative(5);
    expect(useApp.getState().selected).toBe("outro");
    useApp.getState().selectRelative(-10);
    expect(useApp.getState().selected).toBe("intro");
    expect(useApp.getState().revealRev).toBe(3);
  });

  it("selectRelative does nothing without slides", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: { ...deckFor(""), slides: [] }, selected: null });
    useApp.getState().selectRelative(1);
    expect(useApp.getState().selected).toBeNull();
    expect(useApp.getState().revealRev).toBe(0);
  });

  it("setDeck keeps the selection when the slide survives, else picks the first", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "outro" });
    useApp.getState().setDeck(deckFor(DECK_HTML, "2"));
    expect(useApp.getState().selected).toBe("outro");
    useApp.getState().setDeck(deckFor(DECK_HTML.replace(` id="outro"`, ` id="end"`)));
    expect(useApp.getState().selected).toBe("intro");
  });
});

describe("chat panel", () => {
  it("is open by default", async () => {
    const useApp = await freshStore();
    expect(useApp.getState().chatOpen).toBe(true);
  });

  it("restores a collapsed chat from localStorage", async () => {
    localStorage.setItem("slopslide.chatOpen", "false");
    const useApp = await freshStore();
    expect(useApp.getState().chatOpen).toBe(false);
  });

  it("setChatOpen switches and persists", async () => {
    const useApp = await freshStore();
    useApp.getState().setChatOpen(false);
    expect(useApp.getState().chatOpen).toBe(false);
    expect(localStorage.getItem("slopslide.chatOpen")).toBe("false");
    useApp.getState().setChatOpen(true);
    expect(useApp.getState().chatOpen).toBe(true);
    expect(localStorage.getItem("slopslide.chatOpen")).toBe("true");
  });
});

describe("stage view", () => {
  it("defaults to slides", async () => {
    const useApp = await freshStore();
    expect(useApp.getState().view).toBe("slides");
  });

  it("restores the last view from localStorage", async () => {
    localStorage.setItem("slopslide.view", "code");
    const useApp = await freshStore();
    expect(useApp.getState().view).toBe("code");
  });

  it("ignores unknown stored values", async () => {
    localStorage.setItem("slopslide.view", "bogus");
    const useApp = await freshStore();
    expect(useApp.getState().view).toBe("slides");
  });

  it("setView switches and persists", async () => {
    const useApp = await freshStore();
    useApp.getState().setView("code");
    expect(useApp.getState().view).toBe("code");
    expect(localStorage.getItem("slopslide.view")).toBe("code");
    useApp.getState().setView("slides");
    expect(localStorage.getItem("slopslide.view")).toBe("slides");
  });
});

describe("closing a deck with unsaved HTML edits", () => {
  it("closes immediately when there are no edits", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), codeDirty: false, sketches: { intro: [{ tool: "pen", color: "#fff", points: [[0, 0]] }] } });
    await useApp.getState().closeDeck();
    expect(ask).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledWith("close_deck");
    expect(useApp.getState().deck).toBeNull();
    expect(useApp.getState().sketches).toEqual({});
  });

  it("stays open when the user keeps their edits", async () => {
    const useApp = await freshStore();
    ask.mockResolvedValue(false);
    useApp.setState({ deck: deckFor(DECK_HTML), codeDirty: true });
    await useApp.getState().closeDeck();
    expect(ask).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalledWith("close_deck");
    expect(useApp.getState().deck).not.toBeNull();
    expect(useApp.getState().codeDirty).toBe(true);
  });

  it("closes and clears the dirty flag when the user discards", async () => {
    const useApp = await freshStore();
    ask.mockResolvedValue(true);
    useApp.setState({ deck: deckFor(DECK_HTML), codeDirty: true });
    await useApp.getState().closeDeck();
    expect(invoke).toHaveBeenCalledWith("close_deck");
    expect(useApp.getState().deck).toBeNull();
    expect(useApp.getState().codeDirty).toBe(false);
  });

  it("falls back to window.confirm when the native dialog is unavailable", async () => {
    const useApp = await freshStore();
    ask.mockRejectedValue(new Error("no dialog plugin"));
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    useApp.setState({ deck: deckFor(DECK_HTML), codeDirty: true });
    await useApp.getState().closeDeck();
    expect(confirm).toHaveBeenCalledOnce();
    expect(useApp.getState().deck).not.toBeNull();
    confirm.mockRestore();
  });
});

const DECK = deckFor(DECK_HTML);

const userMessage = (text: string): UserMessage => ({
  id: `u-${text}`,
  role: "user",
  text,
  slide: null,
  attachments: [],
  createdAt: 1,
});

const assistantMessage = (patch: Partial<AssistantMessage> = {}): AssistantMessage => ({
  id: "a-1",
  role: "assistant",
  parts: [],
  status: "done",
  thinking: false,
  error: null,
  costUsd: null,
  durationMs: null,
  createdAt: 2,
  ...patch,
});

describe("opening and creating decks", () => {
  it("openDeck loads the deck, its chat and whether the agent is running", async () => {
    const useApp = await freshStore();
    const chat: ChatMessage[] = [userMessage("hi"), assistantMessage()];
    backend({ open_deck: () => DECK, load_chat: () => chat, agent_running: () => true });
    useApp.setState({ assetsRev: 4, presenting: true, selected: "stale", sketches: { intro: [{ tool: "pen", color: "#fff", points: [[0, 0]] }] } });
    await useApp.getState().openDeck("talk");
    expect(calls("open_deck")).toEqual([{ id: "talk" }]);
    expect(calls("load_chat")).toEqual([{ id: "talk" }]);
    const state = useApp.getState();
    expect(state.deck).toEqual(DECK);
    expect(state.selected).toBe("intro");
    expect(state.messages).toEqual(chat);
    expect(state.running).toBe(true);
    expect(state.assetsRev).toBe(0);
    expect(state.presenting).toBe(false);
    expect(state.sketches).toEqual({});
  });

  it("settles a transcript that was saved mid-turn", async () => {
    const useApp = await freshStore();
    const streaming = assistantMessage({ status: "streaming", thinking: true, compacting: true });
    backend({ open_deck: () => DECK, load_chat: () => [userMessage("hi"), streaming], agent_running: () => false });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().messages[1]).toEqual({ ...streaming, status: "interrupted", thinking: false, compacting: false });
  });

  it("starts with an empty transcript when the chat file is missing or not a list", async () => {
    const useApp = await freshStore();
    backend({ open_deck: () => DECK, load_chat: () => null, agent_running: () => false });
    useApp.setState({ messages: [userMessage("old deck")] });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().messages).toEqual([]);
    backend({ open_deck: () => DECK, load_chat: () => ({ bogus: true }), agent_running: () => false });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().messages).toEqual([]);
  });

  it("selects nothing in an empty deck", async () => {
    const useApp = await freshStore();
    backend({ open_deck: () => ({ ...DECK, slides: [] }), load_chat: () => null, agent_running: () => false });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().selected).toBeNull();
  });

  it("openDeck reports failures instead of throwing", async () => {
    const useApp = await freshStore();
    backend({
      open_deck: () => {
        throw "deck not found: talk";
      },
    });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().error).toBe("deck not found: talk");
    expect(useApp.getState().deck).toBeNull();
  });

  it("createDeck creates and opens the new deck", async () => {
    const useApp = await freshStore();
    backend({ create_deck: () => DECK, load_chat: () => null, agent_running: () => false });
    await useApp.getState().createDeck("Talk");
    expect(calls("create_deck")).toEqual([{ title: "Talk" }]);
    expect(useApp.getState().deck).toEqual(DECK);
  });

  it("createDeck reports failures", async () => {
    const useApp = await freshStore();
    backend({
      create_deck: () => {
        throw new Error("disk full");
      },
    });
    await useApp.getState().createDeck("Talk");
    expect(useApp.getState().error).toBe("disk full");
  });

  it("closeDeck resets the editor state", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, selected: "intro", messages: [userMessage("x")], running: true, presenting: true });
    await useApp.getState().closeDeck();
    expect(useApp.getState()).toMatchObject({ deck: null, selected: null, messages: [], running: false, presenting: false });
  });
});

describe("slide image export", () => {
  it("remembers the folder and which slides to save", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK });
    useApp.getState().startImageExport("/out/Talk");
    expect(useApp.getState().imageExport).toEqual({ dir: "/out/Talk", slides: ["intro", "#2", "outro"] });
    useApp.getState().endImageExport();
    expect(useApp.getState().imageExport).toBeNull();
  });

  it("does not start without slides", async () => {
    const useApp = await freshStore();
    useApp.getState().startImageExport("/out/Talk");
    useApp.setState({ deck: { ...DECK, slides: [] } });
    useApp.getState().startImageExport("/out/Talk");
    expect(useApp.getState().imageExport).toBeNull();
  });

  it("is abandoned when the deck closes or another opens", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, imageExport: { dir: "/out", slides: ["intro"] } });
    await useApp.getState().closeDeck();
    expect(useApp.getState().imageExport).toBeNull();
    backend({ open_deck: () => DECK, load_chat: () => [], agent_running: () => false });
    useApp.setState({ imageExport: { dir: "/out", slides: ["intro"] } });
    await useApp.getState().openDeck("talk");
    expect(useApp.getState().imageExport).toBeNull();
  });
});

describe("model choice", () => {
  const PROVIDERS: ProviderInfo[] = [
    {
      id: "claude",
      installed: true,
      path: "/bin/claude",
      models: [
        { id: "claude-opus-5-5", label: "Claude Opus 5.5", isDefault: true, efforts: ["low", "medium", "high", "max"], defaultEffort: "medium", contextWindows: ["200k", "1m"], defaultContextWindow: "1m" },
        { id: "claude-sonnet-5", label: "Claude Sonnet 5", isDefault: false, efforts: ["low", "medium", "high", "max"], defaultEffort: "medium", contextWindows: ["200k", "1m"], defaultContextWindow: "200k" },
      ],
      error: null,
    },
    {
      id: "codex",
      installed: true,
      path: "/bin/codex",
      models: [{ id: "gpt-6-astra", label: "GPT-6-Astra", isDefault: true, efforts: ["low", "high"], defaultEffort: "high", contextWindows: [], defaultContextWindow: null }],
      error: null,
    },
  ];

  it("defaults to Claude Opus 5.5 at medium effort", async () => {
    const useApp = await freshStore();
    expect(useApp.getState().selection).toEqual({
      provider: "claude",
      model: "claude-opus-5-5",
      label: "Claude Opus 5.5",
      effort: "medium",
      contextWindow: "1m",
    });
  });

  it("persists and restores the chosen model, keeping a supported effort", async () => {
    let useApp = await freshStore();
    useApp.setState({ providers: PROVIDERS });
    useApp.getState().setModel("codex", "gpt-6-astra");
    // Codex's model has no "medium", so its own default applies.
    expect(useApp.getState().selection).toEqual({ provider: "codex", model: "gpt-6-astra", label: "GPT-6-Astra", effort: "high", contextWindow: null });
    useApp.getState().setEffort("low");
    useApp = await freshStore();
    expect(useApp.getState().selection).toMatchObject({ provider: "codex", model: "gpt-6-astra", effort: "low" });
  });

  it("ignores models that are not offered", async () => {
    const useApp = await freshStore();
    useApp.setState({ providers: PROVIDERS });
    useApp.getState().setModel("codex", "nope");
    expect(useApp.getState().selection.provider).toBe("claude");
  });

  it("moves a selection whose provider is not installed onto an installed default", async () => {
    localStorage.setItem(
      "slopslide.selection",
      JSON.stringify({ provider: "codex", model: "gone", label: "Gone", effort: "max" }),
    );
    const useApp = await freshStore();
    const providers = PROVIDERS.map((p) => (p.id === "codex" ? { ...p, installed: false, models: [] } : p));
    backend({ list_providers: () => providers });
    await useApp.getState().refreshProviders();
    expect(useApp.getState().providers).toEqual(providers);
    expect(useApp.getState().selection).toEqual({ provider: "claude", model: "claude-opus-5-5", label: "Claude Opus 5.5", effort: "max", contextWindow: "1m" });
  });

  it("keeps the saved model when its provider could not list models", async () => {
    const saved = { provider: "codex", model: "gpt-6-astra", label: "GPT-6-Astra", effort: "high" };
    localStorage.setItem("slopslide.selection", JSON.stringify(saved));
    const useApp = await freshStore();
    const failing = PROVIDERS.map((p) => (p.id === "codex" ? { ...p, models: [], error: "not signed in" } : p));
    backend({ list_providers: () => failing });
    await useApp.getState().refreshProviders();
    expect(useApp.getState().selection).toEqual({ ...saved, contextWindow: null });
  });

  it("starts each model on its default context window and persists a change", async () => {
    let useApp = await freshStore();
    useApp.setState({ providers: PROVIDERS });
    useApp.getState().setModel("claude", "claude-sonnet-5");
    expect(useApp.getState().selection.contextWindow).toBe("200k");
    useApp.getState().setContextWindow("1m");
    useApp = await freshStore();
    expect(useApp.getState().selection).toMatchObject({ model: "claude-sonnet-5", contextWindow: "1m" });
    useApp.setState({ providers: PROVIDERS });
    useApp.getState().setModel("claude", "claude-opus-5-5");
    expect(useApp.getState().selection.contextWindow).toBe("1m");
  });

  it("reconciles a saved context window with the model's options", async () => {
    localStorage.setItem(
      "slopslide.selection",
      JSON.stringify({ provider: "claude", model: "claude-sonnet-5", label: "Claude Sonnet 5", effort: "high" }),
    );
    const useApp = await freshStore();
    backend({ list_providers: () => PROVIDERS });
    await useApp.getState().refreshProviders();
    expect(useApp.getState().selection.contextWindow).toBe("200k");
  });

  it("toggles and persists favorite models", async () => {
    let useApp = await freshStore();
    useApp.getState().toggleFavoriteModel("codex:gpt-6-astra");
    useApp = await freshStore();
    expect(useApp.getState().favoriteModels).toEqual(["codex:gpt-6-astra"]);
    useApp.getState().toggleFavoriteModel("codex:gpt-6-astra");
    expect(useApp.getState().favoriteModels).toEqual([]);
  });
});

describe("sending a message", () => {
  const prompt = () => (calls("send_message")[0]!.args as { prompt: string }).prompt;

  it("does nothing without a deck or while the agent runs", async () => {
    const useApp = await freshStore();
    await useApp.getState().send("hi", { includeSlide: true, attachments: [] });
    useApp.setState({ deck: DECK, running: true });
    await useApp.getState().send("hi", { includeSlide: true, attachments: [] });
    expect(calls("send_message")).toEqual([]);
    expect(useApp.getState().messages).toEqual([]);
  });

  it("adds the user message and a streaming reply, and marks the agent running", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, selected: "outro" });
    await useApp.getState().send("Make it blue", { includeSlide: true, attachments: ["assets/a.png"] });
    const [user, reply] = useApp.getState().messages as [UserMessage, AssistantMessage];
    expect(user).toMatchObject({ role: "user", text: "Make it blue", slide: "outro", attachments: ["assets/a.png"] });
    expect(reply).toMatchObject({ role: "assistant", status: "streaming", thinking: true, parts: [] });
    expect(user.id).not.toBe(reply.id);
    expect(useApp.getState().running).toBe(true);
  });

  it("tells the agent which slide is selected", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, selected: "#2" });
    await useApp.getState().send("Make it blue", { includeSlide: true, attachments: [] });
    expect(prompt()).toBe(
      `[context]\nCurrent slide: <section id="#2"> in deck.html (slide 2 of 3)\n[/context]\n\nMake it blue`,
    );
  });

  it("sends the bare text when there is no context to add", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, selected: "intro" });
    await useApp.getState().send("Whole deck please", { includeSlide: false, attachments: [] });
    expect(prompt()).toBe("Whole deck please");
    expect(useApp.getState().messages[0]).toMatchObject({ slide: null });
  });

  it("mentions an empty deck and attached files", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: { ...DECK, slides: [] }, selected: null });
    await useApp.getState().send("Start", { includeSlide: true, attachments: ["assets/a.png", "assets/b.csv"] });
    expect(prompt()).toBe(
      "[context]\nThe deck has no slides yet.\nAttached files: assets/a.png, assets/b.csv\n[/context]\n\nStart",
    );
  });

  it("passes the selected provider, model, effort, and context window", async () => {
    const useApp = await freshStore();
    useApp.setState({
      deck: DECK,
      selection: { provider: "codex", model: "gpt-6-astra", label: "GPT-6-Astra", effort: "high", contextWindow: null },
    });
    await useApp.getState().send("a", { includeSlide: false, attachments: [] });
    expect(calls("send_message")[0]!.args).toEqual({
      deckId: "talk",
      prompt: "a",
      provider: "codex",
      model: "gpt-6-astra",
      effort: "high",
      contextWindow: null,
      compact: false,
    });
  });

  it("sends no effort for a model that takes none", async () => {
    const useApp = await freshStore();
    useApp.setState({
      deck: DECK,
      providers: [
        {
          id: "copilot",
          installed: true,
          path: "/bin/copilot",
          models: [
            { id: "auto", label: "Auto", isDefault: false, efforts: [], defaultEffort: null, contextWindows: [], defaultContextWindow: null },
          ],
          error: null,
        },
      ],
      selection: { provider: "copilot", model: "auto", label: "Auto", effort: "medium", contextWindow: null },
    });
    await useApp.getState().send("a", { includeSlide: false, attachments: [] });
    expect(calls("send_message")[0]!.args).toMatchObject({ provider: "copilot", model: "auto", effort: "" });
  });

  it("shows a failed send on the reply and saves the transcript", async () => {
    const useApp = await freshStore();
    backend({
      send_message: () => {
        throw "Claude Code was not found.";
      },
    });
    useApp.setState({ deck: DECK });
    await useApp.getState().send("hi", { includeSlide: false, attachments: [] });
    const reply = useApp.getState().messages[1] as AssistantMessage;
    expect(reply).toMatchObject({ status: "error", thinking: false, error: "Claude Code was not found." });
    expect(useApp.getState().running).toBe(false);
    expect(calls("save_chat")).toEqual([{ id: "talk", chat: useApp.getState().messages }]);
  });

  describe("with a sketch on the slide", () => {
    // Pen strokes from (0.25, 0.5) to (0.5, 0.25) of the slide: x 480–960, y 270–540, padded by 4px.
    const mark = {
      tool: "pen" as const,
      color: "#ef4444",
      points: [
        [0.25, 0.5],
        [0.5, 0.25],
      ] as [number, number][],
    };
    let target: HTMLElement;

    beforeEach(() => {
      target = document.createElement("div");
      target.setAttribute("data-sketch-target", "");
      target.getBoundingClientRect = () => new DOMRect(40, 60, 800, 450);
      document.body.appendChild(target);
    });
    afterEach(() => target.remove());

    it("screenshots the slide with the ink, which stays on the slide as a review", async () => {
      const useApp = await freshStore();
      let inkWhenCaptured: unknown;
      backend({
        capture_sketch: () => {
          inkWhenCaptured = useApp.getState().sketches.outro;
          return ".slopslide/sketches/1-ab.png";
        },
      });
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark], intro: [mark] } });
      await useApp.getState().send("Move this up", { includeSlide: true, attachments: [] });
      expect(calls("capture_sketch")).toEqual([
        {
          id: "talk",
          rect: { x: 40, y: 60, width: 800, height: 450 },
          viewport: { width: window.innerWidth, height: window.innerHeight },
        },
      ]);
      expect(inkWhenCaptured).toEqual([mark]);
      expect(useApp.getState().sketches).toEqual({ outro: [mark], intro: [mark] });
      expect(prompt()).toBe(
        [
          "[context]",
          'Current slide: <section id="outro"> in deck.html (slide 3 of 3)',
          "Sketch: .slopslide/sketches/1-ab.png (screenshot of the current slide with the user's marks drawn on top)",
          "Marked area: x 476–964, y 266–544 of the 1920×1080 slide",
          "[/context]",
          "",
          "Move this up",
        ].join("\n"),
      );
      expect(useApp.getState().messages[0]).toMatchObject({
        sketch: { image: ".slopslide/sketches/1-ab.png", bounds: { left: 476, top: 266, right: 964, bottom: 544 } },
      });
    });

    it("still describes the marked area when the screenshot fails", async () => {
      const useApp = await freshStore();
      vi.spyOn(console, "warn").mockImplementation(() => {});
      backend({
        capture_sketch: () => {
          throw "slide screenshots are not supported on this platform yet";
        },
      });
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] } });
      await useApp.getState().send("Fix", { includeSlide: true, attachments: [] });
      expect(prompt()).not.toContain("Sketch:");
      expect(prompt()).toContain("Marked area: x 476–964, y 266–544 of the 1920×1080 slide");
      expect(useApp.getState().messages[0]).toMatchObject({ sketch: { image: null } });
      expect(useApp.getState().sketches).toEqual({ outro: [mark] });
      expect(calls("send_message")).toHaveLength(1);
      vi.restoreAllMocks();
    });

    it("skips the screenshot when the slide is not on screen", async () => {
      const useApp = await freshStore();
      target.remove();
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] } });
      await useApp.getState().send("Fix", { includeSlide: true, attachments: [] });
      expect(calls("capture_sketch")).toEqual([]);
      expect(prompt()).toContain("Marked area:");
    });

    it("leaves the sketch alone when the message is not about the slide", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] } });
      await useApp.getState().send("Whole deck", { includeSlide: false, attachments: [] });
      expect(calls("capture_sketch")).toEqual([]);
      expect(prompt()).toBe("Whole deck");
      expect(useApp.getState().messages[0]).toMatchObject({ sketch: null });
      expect(useApp.getState().sketches).toEqual({ outro: [mark] });
    });

    it("sends the marks once, and again after they change", async () => {
      const useApp = await freshStore();
      backend({ capture_sketch: () => ".slopslide/sketches/1.png" });
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] } });
      const send = async (text: string) => {
        await useApp.getState().send(text, { includeSlide: true, attachments: [] });
        useApp.setState({ running: false });
      };
      await send("Fix");
      await send("And the title");
      const prompts = () => calls("send_message").map((c) => (c.args as { prompt: string }).prompt);
      expect(calls("capture_sketch")).toHaveLength(1);
      expect(prompts()[1]).not.toContain("Marked area");
      useApp.getState().setSketches((all) => ({ ...all, outro: [mark, mark] }));
      await send("Also this");
      expect(calls("capture_sketch")).toHaveLength(2);
      expect(prompts()[2]).toContain("Marked area");
    });

    it("skipSketch keeps the current marks out of the next message", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] } });
      useApp.getState().skipSketch("outro");
      await useApp.getState().send("Fix", { includeSlide: true, attachments: [] });
      expect(calls("capture_sketch")).toEqual([]);
      expect(prompt()).not.toContain("Marked area");
      expect(useApp.getState().sketches).toEqual({ outro: [mark] });
    });

    it("does not send hidden review marks", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "outro", sketches: { outro: [mark] }, reviewVisible: false });
      await useApp.getState().send("Fix", { includeSlide: true, attachments: [] });
      expect(calls("capture_sketch")).toEqual([]);
      expect(useApp.getState().messages[0]).toMatchObject({ sketch: null });
    });

    it("ignores another slide's sketch and emptied sketches", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "outro", sketches: { intro: [mark], outro: [] } });
      await useApp.getState().send("Fix", { includeSlide: true, attachments: [] });
      expect(calls("capture_sketch")).toEqual([]);
      expect(prompt()).not.toContain("Marked area");
    });
  });

  it("clearSketch and setSketches edit ink per slide", async () => {
    const useApp = await freshStore();
    const ink = [{ tool: "pen" as const, color: "#fff", points: [[0, 0]] as [number, number][] }];
    useApp.getState().setSketches((all) => ({ ...all, a: ink, b: ink }));
    useApp.getState().clearSketch("a");
    useApp.getState().clearSketch("missing");
    expect(useApp.getState().sketches).toEqual({ b: ink });
  });

  describe("review marks", () => {
    const ink = [{ tool: "pen" as const, color: "#fff", points: [[0.5, 0.5]] as [number, number][] }];

    afterEach(() => {
      vi.useRealTimers();
    });

    it("are saved to deck.html once drawing pauses, without empty slides", async () => {
      vi.useFakeTimers();
      const useApp = await freshStore();
      useApp.setState({ deck: DECK });
      useApp.getState().setSketches((all) => ({ ...all, intro: ink }));
      useApp.getState().setSketches((all) => ({ ...all, outro: ink }));
      useApp.getState().clearSketch("outro");
      useApp.getState().setSketches((all) => ({ ...all, "#2": [] }));
      await vi.advanceTimersByTimeAsync(300);
      expect(calls("save_review")).toEqual([]);
      await vi.advanceTimersByTimeAsync(200);
      expect(calls("save_review")).toEqual([{ id: "talk", review: { intro: ink } }]);
    });

    it("are saved right away when the deck closes", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK });
      useApp.getState().setSketches(() => ({ intro: ink }));
      await useApp.getState().closeDeck();
      expect(calls("save_review")).toEqual([{ id: "talk", review: { intro: ink } }]);
      const order = invoke.mock.calls.map(([c]) => c);
      expect(order.indexOf("save_review")).toBeLessThan(order.indexOf("close_deck"));
    });

    it("are not saved when they end up as the file has them", async () => {
      const { useApp, flushReviewSave } = await freshModule();
      useApp.setState({ deck: DECK });
      useApp.getState().setSketches(() => ({ intro: ink }));
      useApp.getState().clearSketch("intro");
      await flushReviewSave();
      expect(calls("save_review")).toEqual([]);
    });

    it("report a failed save", async () => {
      const { useApp, flushReviewSave } = await freshModule();
      backend({
        save_review: () => {
          throw "disk full";
        },
      });
      useApp.setState({ deck: DECK });
      useApp.getState().setSketches(() => ({ intro: ink }));
      await flushReviewSave();
      expect(useApp.getState().error).toBe("Could not save the review marks: disk full");
    });

    it("are loaded with the deck", async () => {
      const useApp = await freshStore();
      backend({ open_deck: () => ({ ...DECK, review: { intro: ink } }), load_chat: () => [], agent_running: () => false });
      await useApp.getState().openDeck("talk");
      expect(useApp.getState().sketches).toEqual({ intro: ink });
    });

    it("visibility is remembered", async () => {
      localStorage.removeItem("slopslide.reviewVisible");
      let useApp = await freshStore();
      expect(useApp.getState().reviewVisible).toBe(true);
      useApp.getState().setReviewVisible(false);
      useApp = await freshStore();
      expect(useApp.getState().reviewVisible).toBe(false);
      localStorage.removeItem("slopslide.reviewVisible");
    });
  });

  it("interrupt asks the backend to stop this deck's agent", async () => {
    const useApp = await freshStore();
    useApp.getState().interrupt();
    expect(calls("interrupt_agent")).toEqual([]);
    useApp.setState({ deck: DECK });
    useApp.getState().interrupt();
    expect(calls("interrupt_agent")).toEqual([{ id: "talk" }]);
  });

  it("records which provider writes the reply", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: DECK, selection: { ...useApp.getState().selection, provider: "copilot" } });
    await useApp.getState().send("hi", { includeSlide: false, attachments: [] });
    expect(useApp.getState().messages[1]).toMatchObject({ role: "assistant", provider: "copilot" });
    expect(calls("send_message")[0]!.args).toMatchObject({ compact: false });
  });

  describe("compacting", () => {
    it("asks the agent to compact, without slide context, and shows it as a command", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "intro" });
      await useApp.getState().compact();
      expect(calls("send_message")[0]!.args).toMatchObject({ prompt: "/compact", compact: true, provider: "claude" });
      const [user, reply] = useApp.getState().messages as [UserMessage, AssistantMessage];
      expect(user).toMatchObject({ text: "/compact", command: "compact", slide: null, attachments: [] });
      expect(reply).toMatchObject({ status: "streaming", provider: "claude" });
      expect(useApp.getState().running).toBe(true);
    });

    it("does nothing without a deck or while the agent runs", async () => {
      const useApp = await freshStore();
      await useApp.getState().compact();
      useApp.setState({ deck: DECK, running: true });
      await useApp.getState().compact();
      expect(calls("send_message")).toEqual([]);
      expect(useApp.getState().messages).toEqual([]);
    });

    it("treats a typed /compact as the command", async () => {
      const useApp = await freshStore();
      useApp.setState({ deck: DECK, selected: "intro" });
      await useApp.getState().send("  /compact ", { includeSlide: true, attachments: [] });
      expect(calls("send_message")[0]!.args).toMatchObject({ prompt: "/compact", compact: true });
      expect(useApp.getState().messages[0]).toMatchObject({ command: "compact", slide: null });
    });

    it("shows a refused compaction on the reply", async () => {
      const useApp = await freshStore();
      backend({
        send_message: () => {
          throw "The agent is still working on this deck.";
        },
      });
      useApp.setState({ deck: DECK });
      await useApp.getState().compact();
      expect(useApp.getState().messages[1]).toMatchObject({ status: "error", error: "The agent is still working on this deck." });
      expect(useApp.getState().running).toBe(false);
    });
  });

  it("resetChat clears the transcript", async () => {
    const useApp = await freshStore();
    await useApp.getState().resetChat();
    expect(calls("reset_chat")).toEqual([]);
    useApp.setState({ deck: DECK, messages: [userMessage("x")], running: true });
    await useApp.getState().resetChat();
    expect(calls("reset_chat")).toEqual([{ id: "talk" }]);
    expect(useApp.getState().messages).toEqual([]);
    expect(useApp.getState().running).toBe(false);
  });
});

describe("agent events", () => {
  async function bridged(messages: ChatMessage[] = [userMessage("hi"), assistantMessage({ status: "streaming", thinking: true })]) {
    const store = await freshModule();
    backend({ list_providers: () => [] });
    await store.initEventBridge();
    store.useApp.setState({ deck: DECK, messages, running: true });
    const emit = (event: AgentEvent, deckId = "talk") => listeners.get("agent-event")!({ payload: { deckId, event } });
    const reply = () => store.useApp.getState().messages.findLast((m) => m.role === "assistant") as AssistantMessage;
    return { useApp: store.useApp, emit, reply };
  }

  it("initEventBridge loads the installed providers, or none when the check fails", async () => {
    const store = await freshModule();
    const providers = [{ id: "claude", installed: false, path: null, models: [], error: null }];
    backend({ list_providers: () => providers });
    await store.initEventBridge();
    await vi.waitFor(() => expect(store.useApp.getState().providers).toEqual(providers));
    const failing = await freshModule();
    backend({
      list_providers: () => {
        throw new Error("no");
      },
    });
    await failing.initEventBridge();
    await vi.waitFor(() => expect(failing.useApp.getState().providers).toEqual([]));
  });

  it("streams text into the last reply", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "started", sessionId: "s" });
    emit({ type: "textStart" });
    expect(reply()).toMatchObject({ thinking: false, parts: [{ kind: "text", text: "" }] });
    emit({ type: "textDelta", text: "Hel" });
    emit({ type: "textDelta", text: "lo" });
    expect(reply().parts).toEqual([{ kind: "text", text: "Hello" }]);
  });

  it("starts a text part when a delta arrives without textStart", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "toolUse", id: "t1", name: "Read", input: {} });
    emit({ type: "textDelta", text: "after tool" });
    expect(reply().parts.map((p) => p.kind)).toEqual(["tool", "text"]);
  });

  it("shows thinking again between blocks", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "textDelta", text: "x" });
    expect(reply().thinking).toBe(false);
    emit({ type: "thinking" });
    expect(reply().thinking).toBe(true);
  });

  it("tracks tool calls and drops empty text before them", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "textStart" });
    emit({ type: "textDelta", text: "  \n" });
    emit({ type: "toolUse", id: "t1", name: "Edit", input: { file_path: "/d/deck.html" } });
    emit({ type: "toolUse", id: "t2", name: "Read", input: {} });
    expect(reply().parts).toEqual([
      { kind: "tool", id: "t1", name: "Edit", input: { file_path: "/d/deck.html" }, status: "running" },
      { kind: "tool", id: "t2", name: "Read", input: {}, status: "running" },
    ]);
    emit({ type: "toolResult", id: "t2", isError: true });
    emit({ type: "toolResult", id: "t1", isError: false });
    expect(reply().parts.map((p) => p.kind === "tool" && p.status)).toEqual(["done", "error"]);
  });

  it("keeps text that has content when a tool starts", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "textDelta", text: "Editing now." });
    emit({ type: "toolUse", id: "t1", name: "Edit", input: {} });
    expect(reply().parts[0]).toEqual({ kind: "text", text: "Editing now." });
  });

  it("uses the result text when nothing was streamed", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "result", isError: false, text: "All done.", costUsd: 0.12, durationMs: 3400 });
    expect(reply()).toMatchObject({ parts: [{ kind: "text", text: "All done." }], costUsd: 0.12, durationMs: 3400, error: null });
  });

  it("does not repeat the result text after streamed text", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "textDelta", text: "All done." });
    emit({ type: "result", isError: false, text: "All done.", costUsd: null, durationMs: null });
    expect(reply().parts).toEqual([{ kind: "text", text: "All done." }]);
  });

  it("turns an error result into an error message", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "result", isError: true, text: "Credit balance too low", costUsd: null, durationMs: 10 });
    expect(reply()).toMatchObject({ error: "Credit balance too low", parts: [] });
    emit({ type: "result", isError: true, text: null, costUsd: null, durationMs: null });
    expect(reply().error).toBe("The agent reported an error.");
  });

  it("records backend errors", async () => {
    const { emit, reply } = await bridged();
    emit({ type: "error", message: "Claude Code stopped unexpectedly: boom" });
    expect(reply().error).toBe("Claude Code stopped unexpectedly: boom");
  });

  it.each([
    [{ interrupted: false }, null, "done"],
    [{ interrupted: false }, "boom", "error"],
    [{ interrupted: true }, "boom", "interrupted"],
  ] as const)("finished %j with error %j settles as %s", async (finished, error, status) => {
    const { useApp, emit, reply } = await bridged();
    emit({ type: "toolUse", id: "t1", name: "Edit", input: {} });
    if (error) emit({ type: "error", message: error });
    emit({ type: "finished", ...finished });
    expect(reply()).toMatchObject({ status, thinking: false });
    expect(reply().parts[0]).toMatchObject({ status: "done" });
    expect(useApp.getState().running).toBe(false);
    expect(calls("save_chat")).toEqual([{ id: "talk", chat: useApp.getState().messages }]);
  });

  it("only updates the last reply", async () => {
    const first = assistantMessage({ id: "a-0", parts: [{ kind: "text", text: "old" }] });
    const { useApp, emit } = await bridged([first, userMessage("again"), assistantMessage({ id: "a-1", status: "streaming" })]);
    emit({ type: "textDelta", text: "new" });
    expect(useApp.getState().messages[0]).toEqual(first);
  });

  describe("context usage", () => {
    it("records the context size on the reply, keeping what a report leaves out", async () => {
      const { emit, reply } = await bridged([userMessage("hi"), assistantMessage({ status: "streaming", provider: "claude" })]);
      emit({ type: "usage", contextTokens: 20_000, contextWindow: null });
      expect(reply().context).toEqual({ provider: "claude", tokens: 20_000, window: null });
      emit({ type: "usage", contextTokens: null, contextWindow: 200_000 });
      expect(reply().context).toEqual({ provider: "claude", tokens: 20_000, window: 200_000 });
    });

    it("carries the window over from an earlier reply of the same provider", async () => {
      const earlier = assistantMessage({ id: "a-0", provider: "claude", context: { provider: "claude", tokens: 5, window: 1_000_000 } });
      const other = assistantMessage({ id: "a-1", provider: "copilot", context: { provider: "copilot", tokens: 9, window: 128_000 } });
      const { emit, reply } = await bridged([earlier, other, userMessage("hi"), assistantMessage({ id: "a-2", status: "streaming", provider: "claude" })]);
      emit({ type: "usage", contextTokens: 60_000, contextWindow: null });
      expect(reply().context).toEqual({ provider: "claude", tokens: 60_000, window: 1_000_000 });
    });

    it("falls back to the selected provider for replies saved before providers were recorded", async () => {
      const { useApp, emit, reply } = await bridged();
      useApp.setState({ selection: { ...useApp.getState().selection, provider: "copilot" } });
      emit({ type: "usage", contextTokens: 1, contextWindow: 2 });
      expect(reply().context).toEqual({ provider: "copilot", tokens: 1, window: 2 });
    });

    it("shows compaction, forgets the old size, and settles when the turn ends", async () => {
      const earlier = assistantMessage({ id: "a-0", provider: "claude", context: { provider: "claude", tokens: 90_000, window: 200_000 } });
      const { emit, reply } = await bridged([earlier, userMessage("/compact"), assistantMessage({ id: "a-1", status: "streaming", thinking: true, provider: "claude" })]);
      emit({ type: "compacting" });
      expect(reply()).toMatchObject({ compacting: true, thinking: false });
      emit({ type: "compacted" });
      expect(reply()).toMatchObject({ compacting: false, compacted: true, context: { provider: "claude", tokens: null, window: 200_000 } });
      emit({ type: "usage", contextTokens: 12_000, contextWindow: 128_000 });
      expect(reply().context).toEqual({ provider: "claude", tokens: 12_000, window: 128_000 });
      emit({ type: "compacting" });
      emit({ type: "finished", interrupted: true });
      expect(reply()).toMatchObject({ compacting: false, status: "interrupted" });
    });
  });

  it("ignores events for other decks", async () => {
    const { useApp, emit } = await bridged();
    const before = useApp.getState().messages;
    emit({ type: "textDelta", text: "elsewhere" }, "other-deck");
    emit({ type: "finished", interrupted: false }, "other-deck");
    expect(useApp.getState().messages).toBe(before);
    expect(useApp.getState().running).toBe(true);
  });

  it("tolerates events with no reply to update", async () => {
    const { useApp, emit } = await bridged([]);
    emit({ type: "textDelta", text: "x" });
    emit({ type: "finished", interrupted: false });
    expect(useApp.getState().messages).toEqual([]);
    expect(useApp.getState().running).toBe(false);
  });
});

describe("deck file changes", () => {
  async function watching(next: () => Deck | Promise<Deck>) {
    vi.useFakeTimers();
    const store = await freshModule();
    backend({ agent_status: () => null, load_deck: () => next() });
    await store.initEventBridge();
    store.useApp.setState({ deck: DECK, selected: "intro", running: false, assetsRev: 0 });
    const changed = (paths: string[], deckId = "talk") => listeners.get("deck-changed")!({ payload: { deckId, paths } });
    return { useApp: store.useApp, changed };
  }

  afterEach(() => {
    vi.useRealTimers();
  });

  it("takes on review marks changed outside the app", async () => {
    const ink = [{ tool: "pen" as const, color: "#fff", points: [[0.5, 0.5]] as [number, number][] }];
    let review: Deck["review"] = { intro: ink };
    const { useApp, changed } = await watching(() => ({ ...DECK, review }));
    useApp.setState({ sketches: {} });
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(200);
    expect(useApp.getState().sketches).toEqual({ intro: ink });
    // The agent cleared the review when asked to.
    review = undefined;
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(200);
    expect(useApp.getState().sketches).toEqual({});
  });

  it("keeps marks the user is still drawing over the file's older ones", async () => {
    const ink = [{ tool: "pen" as const, color: "#fff", points: [[0.5, 0.5]] as [number, number][] }];
    const { useApp, changed } = await watching(() => ({ ...DECK, review: {} }));
    useApp.getState().setSketches(() => ({ intro: ink }));
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(130);
    expect(useApp.getState().sketches).toEqual({ intro: ink });
    // Once saved, the file matches what was saved and nothing is taken on.
    await vi.advanceTimersByTimeAsync(400);
    expect(calls("save_review")).toHaveLength(1);
  });

  it("reloads every preview when attached assets change", async () => {
    const { useApp, changed } = await watching(() => DECK);
    changed(["assets/photo.png"]);
    expect(useApp.getState().assetsRev).toBe(1);
    await vi.advanceTimersByTimeAsync(500);
    expect(calls("load_deck")).toEqual([]);
  });

  it("reloads the deck once a burst of edits settles", async () => {
    const edited = deckFor(DECK_HTML, "2");
    const { useApp, changed } = await watching(() => edited);
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(100);
    changed(["deck.html", "assets/a.png"]);
    await vi.advanceTimersByTimeAsync(100);
    expect(calls("load_deck")).toEqual([]);
    await vi.advanceTimersByTimeAsync(20);
    expect(calls("load_deck")).toEqual([{ id: "talk" }]);
    expect(useApp.getState().deck).toEqual(edited);
    expect(useApp.getState().assetsRev).toBe(1);
  });

  it("follows the agent to the slide it changed", async () => {
    const edited = { ...DECK, slides: DECK.slides.map((s) => (s.id === "outro" ? { ...s, hash: "new" } : s)) };
    const { useApp, changed } = await watching(() => edited);
    useApp.setState({ running: true });
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(120);
    expect(useApp.getState().selected).toBe("outro");
  });

  it("keeps the user's selection when they are editing themselves", async () => {
    const edited = { ...DECK, slides: DECK.slides.map((s) => (s.id === "outro" ? { ...s, hash: "new" } : s)) };
    const { useApp, changed } = await watching(() => edited);
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(120);
    expect(useApp.getState().selected).toBe("intro");
  });

  it("follows the agent to a newly added slide", async () => {
    const added = { ...DECK, slides: [...DECK.slides, { id: "fresh", hash: "f", hidden: false, moved: false }] };
    const { useApp, changed } = await watching(() => added);
    useApp.setState({ running: true });
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(120);
    expect(useApp.getState().selected).toBe("fresh");
  });

  it("ignores a deck.html that cannot be read mid-write", async () => {
    const { useApp, changed } = await watching(() => {
      throw "stream did not contain valid UTF-8";
    });
    changed(["deck.html"]);
    await vi.advanceTimersByTimeAsync(120);
    expect(useApp.getState().deck).toEqual(DECK);
    expect(useApp.getState().error).toBeNull();
  });

  it("does nothing when the deck was closed meanwhile", async () => {
    const { useApp, changed } = await watching(() => DECK);
    changed(["deck.html"]);
    useApp.setState({ deck: null });
    await vi.advanceTimersByTimeAsync(120);
    expect(calls("load_deck")).toEqual([]);
  });

  it("ignores other decks and unrelated files", async () => {
    const { useApp, changed } = await watching(() => DECK);
    changed(["deck.html", "assets/a.png"], "other-deck");
    changed(["notes.md"]);
    await vi.advanceTimersByTimeAsync(500);
    expect(calls("load_deck")).toEqual([]);
    expect(useApp.getState().assetsRev).toBe(0);
  });
});

describe("lint", () => {
  const issue = (patch: Record<string, unknown> = {}) => ({
    rule: "unclosed-tag",
    severity: "error",
    message: "<div> is never closed.",
    line: 12,
    slide: "intro",
    ...patch,
  });

  it("refreshLint stores the backend's issues", async () => {
    const useApp = await freshStore();
    backend({ lint_deck: () => [issue()] });
    useApp.setState({ deck: deckFor(DECK_HTML) });
    await useApp.getState().refreshLint();
    expect(calls("lint_deck")).toEqual([{ id: "talk" }]);
    expect(useApp.getState().lint).toEqual([issue()]);
  });

  it("does nothing without a deck", async () => {
    const useApp = await freshStore();
    await useApp.getState().refreshLint();
    expect(calls("lint_deck")).toEqual([]);
    expect(useApp.getState().lint).toBeNull();
  });

  it("keeps only the newest of overlapping checks", async () => {
    const useApp = await freshStore();
    const answers: ((issues: unknown) => void)[] = [];
    invoke.mockImplementation(() => new Promise((resolve) => answers.push(resolve)));
    useApp.setState({ deck: deckFor(DECK_HTML) });
    const first = useApp.getState().refreshLint();
    const second = useApp.getState().refreshLint();
    answers[1]!([]);
    await second;
    answers[0]!([issue()]);
    await first;
    expect(useApp.getState().lint).toEqual([]);
  });

  it("drops results for a deck that is no longer open", async () => {
    const useApp = await freshStore();
    let answer: (issues: unknown) => void = () => {};
    invoke.mockImplementation(() => new Promise((resolve) => (answer = resolve)));
    useApp.setState({ deck: deckFor(DECK_HTML) });
    const pending = useApp.getState().refreshLint();
    useApp.setState({ deck: { ...deckFor(DECK_HTML), id: "other" } });
    answer([issue()]);
    await pending;
    expect(useApp.getState().lint).toBeNull();
  });

  it("clears the status when linting fails", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), lint: [] });
    backend({
      lint_deck: () => {
        throw new Error("deck not found: talk");
      },
    });
    await useApp.getState().refreshLint();
    expect(useApp.getState().lint).toBeNull();
  });

  it("resets lint and composer text when the deck closes", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), lint: [], composerFill: { text: "x", rev: 1 } });
    await useApp.getState().closeDeck();
    expect(useApp.getState().lint).toBeNull();
    expect(useApp.getState().composerFill).toBeNull();
  });

  it("fillComposer bumps its revision even for identical text", async () => {
    const useApp = await freshStore();
    useApp.getState().fillComposer("fix it");
    useApp.getState().fillComposer("fix it");
    expect(useApp.getState().composerFill).toEqual({ text: "fix it", rev: 2 });
  });

  it("lintFixPrompt lists every issue and asks the agent to verify with its tool", async () => {
    const { lintFixPrompt } = await freshModule();
    const prompt = lintFixPrompt([
      issue() as never,
      issue({ severity: "warning", rule: "title", message: "Needs a title.", line: 1, slide: null }) as never,
    ]);
    expect(prompt).toContain("- line 12 error [unclosed-tag] (slide `intro`): <div> is never closed.");
    expect(prompt).toContain("- line 1 warning [title]: Needs a title.");
    expect(prompt).toMatch(/run the lint_deck tool to verify/);
  });
});

describe("editing slides on the stage", () => {
  const MOVED = `<section class="slide" id="intro">\n  <h1 data-moved="" style="translate: 4px 0px;">Hello</h1>\n</section>`;
  const ORIGINAL = `<section class="slide" id="intro">\n  <h1>Hello</h1>\n</section>`;
  /** Backend whose `update_slide` swaps in the markup; like the real one, the hash follows the content. */
  function slideBackend() {
    const revs = new Map([[ORIGINAL, 1]]);
    let rev = 1;
    let markup = ORIGINAL;
    backend({
      update_slide: ({ slide, markup: next, base }) => {
        if (base !== `${String(slide)}-${rev}`) throw "The slide changed while you were editing it.";
        const previous = markup;
        markup = String(next);
        if (!revs.has(markup)) revs.set(markup, revs.size + 1);
        rev = revs.get(markup)!;
        const deck = deckFor(DECK_HTML, String(rev));
        return { deck: { ...deck, slides: deck.slides.map((s) => (s.id === slide ? { ...s, moved: markup.includes("data-moved") } : s)) }, previous };
      },
    });
    return { markup: () => markup };
  }

  it("saves an edit against the slide's current hash and remembers how to undo it", async () => {
    const useApp = await freshStore();
    slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    expect(calls("update_slide")).toEqual([{ id: "talk", slide: "intro", markup: MOVED, base: "intro-1" }]);
    const deck = useApp.getState().deck!;
    expect(deck.slides[0]).toMatchObject({ hash: "intro-2", moved: true });
    expect(useApp.getState().slideUndo).toEqual([{ slide: "intro", markup: ORIGINAL, after: "intro-2" }]);
  });

  it("saves edits one after another, each on top of the last", async () => {
    const useApp = await freshStore();
    const disk = slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    const first = useApp.getState().saveSlideEdit("intro", MOVED);
    const second = useApp.getState().saveSlideEdit("intro", MOVED.replace("Hello", "Hi"));
    await Promise.all([first, second]);
    expect(calls("update_slide").map((c) => c.base)).toEqual(["intro-1", "intro-2"]);
    expect(disk.markup()).toContain("Hi");
    expect(useApp.getState().error).toBeNull();
  });

  it("reports a refused save and reloads the slide from disk", async () => {
    const useApp = await freshStore();
    slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML, "9"), selected: "intro", editReload: 0 });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    expect(useApp.getState().error).toContain("changed");
    expect(useApp.getState().editReload).toBe(1);
    expect(useApp.getState().slideUndo).toEqual([]);
  });

  it("ignores edits of slides that are gone", async () => {
    const useApp = await freshStore();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("deleted", MOVED);
    expect(calls("update_slide")).toEqual([]);
  });

  it("undo saves the previous markup back and selects the slide", async () => {
    const useApp = await freshStore();
    const disk = slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    useApp.getState().select("outro");
    await useApp.getState().undoSlideEdit();
    expect(disk.markup()).toBe(ORIGINAL);
    expect(calls("update_slide")[1]).toEqual({ id: "talk", slide: "intro", markup: ORIGINAL, base: "intro-2" });
    expect(useApp.getState().selected).toBe("intro");
    expect(useApp.getState().slideUndo).toEqual([]);
    await useApp.getState().undoSlideEdit();
    expect(calls("update_slide")).toHaveLength(2);
  });

  it("refuses to undo once the slide changed again (say, by the agent)", async () => {
    const useApp = await freshStore();
    slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    useApp.getState().setDeck(deckFor(DECK_HTML, "agent"));
    await useApp.getState().undoSlideEdit();
    expect(calls("update_slide")).toHaveLength(1);
    expect(useApp.getState().error).toContain("cannot be undone");
    expect(useApp.getState().slideUndo).toEqual([]);
  });

  it("redoes an undone edit, and a new edit forgets what could be redone", async () => {
    const useApp = await freshStore();
    const disk = slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    await useApp.getState().undoSlideEdit();
    expect(useApp.getState().slideRedo).toEqual([{ slide: "intro", markup: MOVED, after: "intro-1" }]);
    await useApp.getState().redoSlideEdit();
    expect(disk.markup()).toBe(MOVED);
    expect(calls("update_slide")[2]).toEqual({ id: "talk", slide: "intro", markup: MOVED, base: "intro-1" });
    expect(useApp.getState().slideRedo).toEqual([]);
    expect(useApp.getState().slideUndo).toEqual([{ slide: "intro", markup: ORIGINAL, after: "intro-2" }]);
    await useApp.getState().undoSlideEdit();
    await useApp.getState().saveSlideEdit("intro", MOVED.replace("Hello", "Hi"));
    expect(useApp.getState().slideRedo).toEqual([]);
    await useApp.getState().redoSlideEdit();
    expect(calls("update_slide")).toHaveLength(5);
  });

  it("refuses to redo once the slide changed again", async () => {
    const useApp = await freshStore();
    slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    await useApp.getState().saveSlideEdit("intro", MOVED);
    await useApp.getState().undoSlideEdit();
    useApp.getState().setDeck(deckFor(DECK_HTML, "agent"));
    await useApp.getState().redoSlideEdit();
    expect(calls("update_slide")).toHaveLength(2);
    expect(useApp.getState().error).toContain("cannot be redone");
  });

  it("discard undoes every edit of the session and leaves edit mode", async () => {
    const useApp = await freshStore();
    const disk = slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    useApp.getState().setEditing(true);
    await useApp.getState().saveSlideEdit("intro", MOVED);
    await useApp.getState().saveSlideEdit("intro", MOVED.replace("Hello", "Hi"));
    await useApp.getState().discardSlideEdits();
    expect(disk.markup()).toBe(ORIGINAL);
    expect(useApp.getState()).toMatchObject({ editing: false, slideUndo: [], slideRedo: [], error: null });
  });

  it("discard stops at an edit it cannot undo", async () => {
    const useApp = await freshStore();
    slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    useApp.getState().setEditing(true);
    await useApp.getState().saveSlideEdit("intro", MOVED);
    useApp.getState().setDeck(deckFor(DECK_HTML, "agent"));
    await useApp.getState().discardSlideEdits();
    expect(calls("update_slide")).toHaveLength(1);
    expect(useApp.getState().error).toContain("cannot be undone");
    expect(useApp.getState().editing).toBe(false);
  });

  it("accepting (leaving edit mode) keeps the edits, and each session starts with fresh history", async () => {
    const useApp = await freshStore();
    const disk = slideBackend();
    useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
    useApp.getState().setEditing(true);
    await useApp.getState().saveSlideEdit("intro", MOVED);
    useApp.getState().setEditing(true);
    expect(useApp.getState().slideUndo).toHaveLength(1);
    useApp.getState().setEditing(false);
    expect(disk.markup()).toBe(MOVED);
    expect(useApp.getState()).toMatchObject({ editing: false, slideUndo: [], slideRedo: [] });
    useApp.getState().setEditing(true);
    expect(useApp.getState().slideUndo).toEqual([]);
  });

  it("leaves edit mode and forgets undo history when the deck closes", async () => {
    const useApp = await freshStore();
    const entry = { slide: "intro", markup: ORIGINAL, after: "x" };
    useApp.setState({ deck: DECK, editing: true, slideUndo: [entry], slideRedo: [entry] });
    await useApp.getState().closeDeck();
    expect(useApp.getState()).toMatchObject({ editing: false, slideUndo: [], slideRedo: [] });
  });

  describe("tidying the layout", () => {
    let target: HTMLElement;
    beforeEach(() => {
      target = document.createElement("div");
      target.setAttribute("data-sketch-target", "");
      target.getBoundingClientRect = () => new DOMRect(0, 0, 640, 360);
      document.body.appendChild(target);
    });
    afterEach(() => target.remove());

    it("sends the agent a screenshot of the slide with tidy-up instructions", async () => {
      const { useApp, TIDY_PROMPT } = await freshModule();
      backend({ capture_sketch: () => ".slopslide/sketches/2-cd.png" });
      useApp.setState({ deck: DECK, selected: "intro" });
      await useApp.getState().tidyLayout();
      expect(calls("capture_sketch")).toHaveLength(1);
      expect((calls("send_message")[0]!.args as { prompt: string }).prompt).toBe(
        [
          "[context]",
          'Current slide: <section id="intro"> in deck.html (slide 1 of 3)',
          "Screenshot: .slopslide/sketches/2-cd.png (the current slide as it looks now, with the user's hand edits)",
          "[/context]",
          "",
          TIDY_PROMPT,
        ].join("\n"),
      );
      expect(useApp.getState().messages[0]).toMatchObject({
        text: TIDY_PROMPT,
        slide: "intro",
        sketch: null,
        screenshot: ".slopslide/sketches/2-cd.png",
      });
    });

    it("lists the overflow the editor found in the request", async () => {
      const { useApp, tidyPrompt } = await freshModule();
      backend({ capture_sketch: () => ".slopslide/sketches/4.png" });
      useApp.setState({ deck: DECK, selected: "intro" });
      const overflow = ['<p> "Long" runs past the bottom edge by 80px', "<h1> is cut off by its own box"];
      await useApp.getState().tidyLayout(overflow);
      const prompt = (calls("send_message")[0]!.args as { prompt: string }).prompt;
      expect(prompt.endsWith(tidyPrompt(overflow))).toBe(true);
      expect(prompt).toContain(`The editor found overflow:\n- ${overflow[0]}\n- ${overflow[1]}`);
      expect(tidyPrompt()).toBe(tidyPrompt([]));
    });

    it("keeps any sketch on the slide with the message", async () => {
      const useApp = await freshStore();
      backend({ capture_sketch: () => ".slopslide/sketches/3.png" });
      const ink = [{ tool: "pen" as const, color: "#f00", points: [[0.5, 0.5]] as [number, number][] }];
      useApp.setState({ deck: DECK, selected: "intro", sketches: { intro: ink } });
      await useApp.getState().tidyLayout();
      expect(calls("capture_sketch")).toHaveLength(1);
      expect(useApp.getState().messages[0]).toMatchObject({
        sketch: { image: ".slopslide/sketches/3.png" },
        screenshot: ".slopslide/sketches/3.png",
      });
      expect(useApp.getState().sketches).toEqual({ intro: ink });
    });
  });
});

describe("narration integration", () => {
  it("flushes scripts before drafting, uses the selected provider, and returns to the requested slide for review", async () => {
    const { useApp } = await freshModule();
    const { useNarration } = await import("./narrationStore");
    const { emptyNarration } = await import("./lib/narration");
    let document = emptyNarration();
    backend({
      open_deck: () => DECK,
      load_narration: () => document,
      save_narration: (a) => { document = { manifest: a.manifest as typeof document.manifest, version: "saved" }; return document; },
    });
    await useApp.getState().openDeck("talk");
    useApp.setState({ selection: { provider: "codex", model: "gpt-6-astra", label: "Codex", effort: "high", contextWindow: null } });
    useNarration.getState().edit("intro", { text: "Existing draft" });
    await useApp.getState().draftNarration("slide", "engineers", "1");
    const commands = invoke.mock.calls.map(([c]) => c);
    expect(commands.indexOf("save_narration")).toBeLessThan(commands.indexOf("send_message"));
    const request = calls("send_message")[0]!.args as { prompt: string; provider: string };
    expect(request.provider).toBe("codex");
    expect(request.prompt).toContain('exact slide IDs: ["intro"]');
    expect(request.prompt).toContain("write_narration MCP tools");
    expect(useApp.getState().sidebarTab).toBe("chat");
    useApp.getState().select("outro");
    useApp.getState().reviewNarration();
    expect(useApp.getState().selected).toBe("intro");
    expect(useApp.getState().sidebarTab).toBe("narration");
    await useNarration.getState().load(null);
  });
  it("keeps the deck open when unsaved narration cannot be written", async () => {
    const { useApp } = await freshModule();
    const { useNarration } = await import("./narrationStore");
    const { emptyNarration } = await import("./lib/narration");
    backend({ open_deck: () => DECK, load_narration: () => emptyNarration(), save_narration: () => { throw new Error("Disk full"); } });
    await useApp.getState().openDeck("talk");
    useNarration.getState().edit("intro", { text: "Keep this text" });
    await useApp.getState().closeDeck();
    expect(calls("close_deck")).toEqual([]);
    expect(useApp.getState().deck?.id).toBe("talk");
    expect(useApp.getState().sidebarTab).toBe("narration");
    expect(useNarration.getState().edits.intro?.text).toBe("Keep this text");
    await useApp.getState().draftNarration("deck", "", "");
    expect(calls("send_message")).toEqual([]);
    await useNarration.getState().load(null);
  });
  it("reloads narration independently of HTML and preview assets", async () => {
    const { useApp, initEventBridge } = await freshModule();
    const { useNarration } = await import("./narrationStore");
    const { emptyNarration, emptyScript } = await import("./lib/narration");
    let document = emptyNarration();
    backend({ open_deck: () => DECK, load_narration: () => document });
    await initEventBridge();
    await useApp.getState().openDeck("talk");
    document = { ...emptyNarration(), version: "external" };
    document.manifest.slides.intro = { ...emptyScript(), text: "Agent draft" };
    listeners.get("deck-changed")!({ payload: { deckId: "talk", paths: ["narration.json"] } });
    // queue wait + IPC response + adoption.
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(useNarration.getState().document?.manifest.slides.intro?.text).toBe("Agent draft");
    expect(calls("load_deck")).toEqual([]);
    expect(useApp.getState().assetsRev).toBe(0);
    expect(useApp.getState().selected).toBe("intro");
    await useNarration.getState().load(null);
  });
});
