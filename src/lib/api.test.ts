import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { emptyNarration } from "./narration";
import { api, errorMessage } from "./api";

beforeEach(() => {
  invoke.mockReset().mockResolvedValue("result");
});

// Command names and argument keys must match the #[tauri::command]s in src-tauri/src/lib.rs.
const CASES = [
  ["prepareVideo", () => api.prepareVideo("talk", "job"), "prepare_video", { id: "talk", jobId: "job" }],
  ["exportVideo", () => api.exportVideo("job", "/tmp/talk.mp4"), "export_video", { jobId: "job", dest: "/tmp/talk.mp4" }],
  ["cancelVideo", () => api.cancelVideo("job"), "cancel_video", { jobId: "job" }],
  ["releaseVideo", () => api.releaseVideo("job"), "release_video", { jobId: "job" }],
  ["speechStatus", () => api.speechStatus(), "speech_status", undefined],
  ["speechHistory", () => api.speechHistory("talk", "intro"), "speech_history", { id: "talk", slide: "intro" }],
  ["selectSpeechTake", () => api.selectSpeechTake("talk", "intro", "take", "version"), "select_speech_take", { id: "talk", slide: "intro", takeId: "take", base: "version" }],
  ["speechTakes", () => api.speechTakes("talk"), "speech_takes", { id: "talk" }],
  ["installSpeechPack", () => api.installSpeechPack("job", null, "qwen-local"), "install_speech_pack", { jobId: "job", source: null, providerId: "qwen-local" }],
  ["removeSpeechPack", () => api.removeSpeechPack("qwen-local"), "remove_speech_pack", { providerId: "qwen-local" }],
  ["generateSpeech", () => api.generateSpeech("job", "talk", "intro"), "generate_speech", { jobId: "job", id: "talk", slide: "intro", fresh: false }],
  ["cancelSpeech", () => api.cancelSpeech("job"), "cancel_speech", { jobId: "job" }],
  ["listDecks", () => api.listDecks(), "list_decks", undefined],
  ["createDeck", () => api.createDeck("Talk"), "create_deck", { title: "Talk", template: null }],
  ["openDeck", () => api.openDeck("talk"), "open_deck", { id: "talk" }],
  ["closeDeck", () => api.closeDeck(), "close_deck", undefined],
  ["loadNarration", () => api.loadNarration("talk"), "load_narration", { id: "talk" }],
  ["saveNarration", () => api.saveNarration("talk", emptyNarration().manifest, "missing"), "save_narration", { id: "talk", manifest: emptyNarration().manifest, base: "missing" }],
  ["loadDeck", () => api.loadDeck("talk"), "load_deck", { id: "talk" }],
  ["renameDeck", () => api.renameDeck("talk", "New"), "rename_deck", { id: "talk", title: "New" }],
  [
    "saveReview",
    () => api.saveReview("talk", { intro: [{ tool: "pen", color: "#fff", points: [[0.5, 0.5]] }] }),
    "save_review",
    { id: "talk", review: { intro: [{ tool: "pen", color: "#fff", points: [[0.5, 0.5]] }] } },
  ],
  ["deleteDeck", () => api.deleteDeck("talk"), "delete_deck", { id: "talk" }],
  ["reorderSlides", () => api.reorderSlides("talk", ["b", "a"]), "reorder_slides", { id: "talk", slides: ["b", "a"] }],
  ["addSlide", () => api.addSlide("talk", null), "add_slide", { id: "talk", after: null }],
  ["duplicateSlide", () => api.duplicateSlide("talk", "a"), "duplicate_slide", { id: "talk", slide: "a" }],
  [
    "setSlideHidden",
    () => api.setSlideHidden("talk", "a", true),
    "set_slide_hidden",
    { id: "talk", slide: "a", hidden: true },
  ],
  [
    "setSlideLocked",
    () => api.setSlideLocked("talk", "a", true),
    "set_slide_locked",
    { id: "talk", slide: "a", locked: true },
  ],
  [
    "addSection",
    () => api.addSection("talk", "b", "Part two"),
    "add_section",
    { id: "talk", before: "b", title: "Part two" },
  ],
  [
    "renameSection",
    () => api.renameSection("talk", 1, "Wrap up"),
    "rename_section",
    { id: "talk", index: 1, title: "Wrap up" },
  ],
  ["deleteSection", () => api.deleteSection("talk", 1), "delete_section", { id: "talk", index: 1 }],
  ["deleteSlide", () => api.deleteSlide("talk", "a"), "delete_slide", { id: "talk", slide: "a" }],
  [
    "updateSlide",
    () => api.updateSlide("talk", "a", "<section>", "h1"),
    "update_slide",
    { id: "talk", slide: "a", markup: "<section>", base: "h1" },
  ],
  [
    "saveDeckSource",
    () => api.saveDeckSource("talk", "<html>", "<old>"),
    "save_deck_source",
    { id: "talk", source: "<html>", base: "<old>" },
  ],
  ["importAssets", () => api.importAssets("talk", ["/a.png"]), "import_assets", { id: "talk", paths: ["/a.png"] }],
  ["saveAsset", () => api.saveAsset("talk", "shot.png", "aGk="), "save_asset", { id: "talk", name: "shot.png", data: "aGk=" }],
  ["exportDeck", () => api.exportDeck("talk", "/out.html"), "export_deck", { id: "talk", dest: "/out.html" }],
  ["lintDeck", () => api.lintDeck("talk"), "lint_deck", { id: "talk" }],
  ["createImageExportDir", () => api.createImageExportDir("talk", "/out"), "create_image_export_dir", { id: "talk", parent: "/out" }],
  [
    "exportSlideImage",
    () => api.exportSlideImage("/out/Talk", 2, 9, { x: 0, y: 10, width: 640, height: 360 }, { width: 640, height: 400 }),
    "export_slide_image",
    { dir: "/out/Talk", index: 2, total: 9, rect: { x: 0, y: 10, width: 640, height: 360 }, viewport: { width: 640, height: 400 } },
  ],
  [
    "captureSketch",
    () => api.captureSketch("talk", { x: 1, y: 2, width: 300, height: 168.75 }, { width: 1480, height: 920 }),
    "capture_sketch",
    { id: "talk", rect: { x: 1, y: 2, width: 300, height: 168.75 }, viewport: { width: 1480, height: 920 } },
  ],
  ["loadChat", () => api.loadChat("talk"), "load_chat", { id: "talk" }],
  ["saveChat", () => api.saveChat("talk", [1]), "save_chat", { id: "talk", chat: [1] }],
  ["resetChat", () => api.resetChat("talk"), "reset_chat", { id: "talk" }],
  [
    "sendMessage",
    () => api.sendMessage("talk", "Hi", { provider: "claude", model: "claude-opus-5-5", effort: "high", contextWindow: "1m" }),
    "send_message",
    { args: { deckId: "talk", prompt: "Hi", provider: "claude", model: "claude-opus-5-5", effort: "high", contextWindow: "1m", compact: false } },
  ],

  ["codexPermissionModes", () => api.codexPermissionModes("talk"), "codex_permission_modes", { id: "talk" }],
  ["respondApproval", () => api.respondApproval("talk", "request-1", "decline"), "respond_approval", { deckId: "talk", id: "request-1", decision: "decline" }],
  ["interruptAgent", () => api.interruptAgent("talk"), "interrupt_agent", { id: "talk" }],
  ["agentRunning", () => api.agentRunning("talk"), "agent_running", { id: "talk" }],
  ["listProviders", () => api.listProviders(), "list_providers", undefined],
  ["listTemplates", () => api.listTemplates(), "list_templates", undefined],
  ["stageTemplate", () => api.stageTemplate("talk", "swiss"), "stage_template", { id: "talk", template: "swiss" }],
  ["applyTemplate", () => api.applyTemplate("talk", "swiss"), "apply_template", { id: "talk", template: "swiss" }],
  [
    "addTemplateSlide",
    () => api.addTemplateSlide("talk", "swiss", "quote", "intro"),
    "add_template_slide",
    { id: "talk", template: "swiss", slide: "quote", after: "intro" },
  ],
  ["createTemplate", () => api.createTemplate("talk", "Mine"), "create_template", { id: "talk", name: "Mine" }],
] as const;

describe("api", () => {
  it.each(CASES)("%s invokes %s", async (_, call, command, args) => {
    await expect(call()).resolves.toBe("result");
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke.mock.calls[0]).toEqual(args === undefined ? [command] : [command, args]);
  });

  it("createDeck passes the chosen template", async () => {
    await api.createDeck("Talk", "synthwave");
    expect(invoke).toHaveBeenLastCalledWith("create_deck", { title: "Talk", template: "synthwave" });
  });

  it("sendMessage can ask for a compaction instead of a prompt", async () => {
    invoke.mockResolvedValue(undefined);
    await api.sendMessage("talk", "/compact", { provider: "copilot", model: "gpt-x", effort: "", contextWindow: null }, true);
    expect(invoke).toHaveBeenLastCalledWith("send_message", {
      args: { deckId: "talk", prompt: "/compact", provider: "copilot", model: "gpt-x", effort: "", contextWindow: null, compact: true },
    });
  });

  it("covers every api function", () => {
    expect(CASES.map(([name]) => name).sort()).toEqual(Object.keys(api).sort());
  });

  it("passes backend rejections through", async () => {
    invoke.mockRejectedValue("deck not found: x");
    await expect(api.openDeck("x")).rejects.toBe("deck not found: x");
  });
});

describe("errorMessage", () => {
  it("shows backend string errors as-is", () => {
    expect(errorMessage("deck not found: x")).toBe("deck not found: x");
  });

  it("uses an Error's message", () => {
    expect(errorMessage(new TypeError("boom"))).toBe("boom");
  });

  it("stringifies anything else", () => {
    expect(errorMessage(42)).toBe("42");
    expect(errorMessage(null)).toBe("null");
    expect(errorMessage(undefined)).toBe("undefined");
  });
});
