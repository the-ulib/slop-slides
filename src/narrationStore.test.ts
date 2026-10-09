import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./lib/api";
import { editedManifest, emptyNarration, emptyScript, type NarrationDocument } from "./lib/narration";
import { useNarration } from "./narrationStore";
vi.mock("./lib/api", () => ({ api: { loadNarration: vi.fn(), saveNarration: vi.fn() }, errorMessage: (e: unknown) => String(e) }));
let disk: NarrationDocument;
beforeEach(async () => {
  vi.useFakeTimers();
  vi.mocked(api.loadNarration).mockReset();
  vi.mocked(api.saveNarration).mockReset();
  disk = emptyNarration();
  vi.mocked(api.loadNarration).mockImplementation(async () => structuredClone(disk));
  vi.mocked(api.saveNarration).mockImplementation(async (_, manifest, base) => {
    if (base !== disk.version) throw new Error("Changed on disk");
    disk = { manifest: { ...structuredClone(manifest), revision: disk.manifest.revision + 1 }, version: `v${disk.manifest.revision + 1}` };
    return structuredClone(disk);
  });
  await useNarration.getState().load("talk");
});
afterEach(async () => { await useNarration.getState().load(null); vi.useRealTimers(); });
const current = () => {
  const s = useNarration.getState();
  return editedManifest(s.document!.manifest, s.edits, s.languageEdit, s.settingsEdits);
};
describe("narration persistence", () => {
  it("saves presenter and pace and restores them when reopening", async () => {
    useNarration.getState().setPresenter("preset:aiden", "Aiden");
    useNarration.getState().setPace(1.2);
    await useNarration.getState().save();
    await useNarration.getState().load("talk");
    expect(current().presenterId).toBe("preset:aiden");
    expect(current().presenterNameSnapshot).toBe("Aiden");
    expect(current().pace).toBe(1.2);
    expect(useNarration.getState().settingsEdits).toEqual({});
  });
  it("merges a local pace edit with an external presenter change", async () => {
    useNarration.getState().setPace(1.2);
    disk = { ...emptyNarration(), version: "external" };
    disk.manifest.presenterId = "preset:aiden";
    disk.manifest.presenterNameSnapshot = "Aiden";
    await expect(useNarration.getState().save()).resolves.toBe(false);
    await useNarration.getState().resolve(true);
    expect(disk.manifest.pace).toBe(1.2);
    expect(disk.manifest.presenterId).toBe("preset:aiden");
  });
  it("debounces edits by stable ID, preserves other scripts and survives reopening", async () => {
    useNarration.getState().edit("intro", { text: "Hallo", languageOverride: "de" });
    useNarration.getState().edit("outro", { text: "Bye" });
    useNarration.getState().setLanguage("de");
    expect(api.saveNarration).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(api.saveNarration).toHaveBeenCalledTimes(1);
    await useNarration.getState().load("talk");
    expect(current().slides.intro!.text).toBe("Hallo");
    expect(current().slides.outro!.text).toBe("Bye");
    expect(current().defaultLanguage).toBe("de");
    expect(useNarration.getState().edits).toEqual({});
  });
  it("flush waits for edits typed during a save instead of dropping them", async () => {
    let finish!: (d: NarrationDocument) => void;
    vi.mocked(api.saveNarration).mockImplementationOnce((_, manifest) => new Promise((resolve) => {
      finish = resolve;
      disk = { manifest, version: "first" };
    }));
    useNarration.getState().edit("intro", { text: "First" });
    const saving = useNarration.getState().save();
    await Promise.resolve();
    useNarration.getState().edit("intro", { text: "Second" });
    finish(structuredClone(disk));
    await expect(saving).resolves.toBe(true);
    expect(api.saveNarration).toHaveBeenCalledTimes(2);
    expect(disk.manifest.slides.intro!.text).toBe("Second");
    expect(useNarration.getState().edits).toEqual({});
  });
  it("keeps local patches on conflict and merges only those patches onto the file version", async () => {
    useNarration.getState().edit("intro", { text: "My edit" });
    disk = { ...emptyNarration(), version: "external" };
    disk.manifest.slides = { intro: { ...emptyScript(), text: "Agent edit", tailMs: 800 }, outro: { ...emptyScript(), text: "External outro" } };
    await expect(useNarration.getState().save()).resolves.toBe(false);
    expect(current().slides.intro!.text).toBe("My edit");
    expect(useNarration.getState().conflict?.version).toBe("external");
    await useNarration.getState().resolve(true);
    expect(disk.manifest.slides.intro!.text).toBe("My edit");
    expect(disk.manifest.slides.intro!.tailMs).toBe(800);
    expect(disk.manifest.slides.outro!.text).toBe("External outro");
  });
  it("can explicitly discard local edits and adopt the incoming scripts", async () => {
    useNarration.getState().edit("intro", { text: "Mine" });
    disk = { ...emptyNarration(), version: "external" };
    disk.manifest.slides.intro = { ...emptyScript(), text: "Theirs" };
    await useNarration.getState().refresh();
    await useNarration.getState().resolve(false);
    expect(current().slides.intro!.text).toBe("Theirs");
    expect(api.saveNarration).not.toHaveBeenCalled();
  });
  it("adopts agent changes when clean and retains orphans", async () => {
    disk = { ...emptyNarration(), version: "agent" };
    disk.manifest.slides.removed = { ...emptyScript(), text: "Recovered" };
    await useNarration.getState().refresh();
    expect(current().slides.removed!.text).toBe("Recovered");
    expect(useNarration.getState().conflict).toBeNull();
  });
  it("does not adopt a stale refresh response after a newer save completes", async () => {
    let finish!: (d: NarrationDocument) => void;
    vi.mocked(api.loadNarration).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const refreshing = useNarration.getState().refresh();
    await Promise.resolve();
    const stale = { ...emptyNarration(), version: "stale" };
    useNarration.getState().edit("intro", { text: "New save" });
    await useNarration.getState().save();
    finish(stale);
    await refreshing;
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(useNarration.getState().document?.version).toBe(disk.version);
    expect(current().slides.intro?.text).toBe("New save");
  });
  it("does not let a late load overwrite a newly opened deck", async () => {
    let finish!: (d: NarrationDocument) => void;
    vi.mocked(api.loadNarration).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const first = useNarration.getState().load("old");
    await useNarration.getState().load("new");
    finish({ ...emptyNarration(), version: "old" });
    await first;
    expect(useNarration.getState().deckId).toBe("new");
    expect(useNarration.getState().document?.version).toBe("missing");
  });
  it("blocks editing corrupt files and keeps unsaved text after an I/O failure", async () => {
    vi.mocked(api.loadNarration).mockRejectedValueOnce(new Error("Unsupported schema"));
    await useNarration.getState().load("bad");
    useNarration.getState().edit("intro", { text: "Do not overwrite" });
    expect(useNarration.getState().document).toBeNull();
    expect(useNarration.getState().edits).toEqual({});
    await expect(useNarration.getState().save()).resolves.toBe(true);
    await useNarration.getState().load("talk");
    useNarration.getState().edit("intro", { text: "Keep me" });
    vi.mocked(api.saveNarration).mockRejectedValueOnce(new Error("Disk full"));
    await expect(useNarration.getState().save()).resolves.toBe(false);
    expect(current().slides.intro!.text).toBe("Keep me");
    expect(useNarration.getState().error).toContain("Disk full");
    await expect(useNarration.getState().save()).resolves.toBe(true);
  });
  it("rejects provisional IDs while the deck is being normalized", () => {
    useNarration.getState().edit("#2", { text: "Unstable" });
    expect(useNarration.getState().edits).toEqual({});
  });
});
