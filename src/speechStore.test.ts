import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
const handlers = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, fn: (event: { payload: unknown }) => void) => { handlers.set(name, fn); return () => {}; }) }));
import { api } from "./lib/api";
import { useSpeech } from "./speechStore";
import { useNarration } from "./narrationStore";
import { emptyNarration } from "./lib/narration";
import { testSpeechProvider } from "./test/speech";
import type { SpeechTake } from "./lib/speech";
const status = { providers: [testSpeechProvider()], job: null };
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: unknown) => void; const promise = new Promise<T>((r, j) => { resolve = r; reject = j; }); return { promise, resolve, reject }; }
beforeAll(async () => { vi.spyOn(api, "speechStatus").mockResolvedValue(status); await useSpeech.getState().initialize(); });
beforeEach(() => {
  useSpeech.setState({ status, job: null, error: null, message: null, deckId: "talk", takes: {}, cancelling: false });
  useNarration.setState({ deckId: "talk", document: emptyNarration(), edits: {}, languageEdit: null, settingsEdits: {}, error: null, conflict: null, saving: false });
  vi.spyOn(api, "speechTakes").mockResolvedValue({}); vi.spyOn(api, "loadNarration").mockResolvedValue(emptyNarration());
});
describe("local speech jobs", () => {
  it("saves pending edits before generation and reloads accepted recordings", async () => {
    const save = vi.spyOn(useNarration.getState(), "save").mockResolvedValue(true);
    const generate = vi.spyOn(api, "generateSpeech").mockResolvedValue({ generated: 1, reused: 0, superseded: 0 });
    await useSpeech.getState().generate("talk", "intro");
    expect(save).toHaveBeenCalled(); expect(generate).toHaveBeenCalledWith(expect.any(String), "talk", "intro");
    expect(save.mock.invocationCallOrder[0]).toBeLessThan(generate.mock.invocationCallOrder[0]!);
    expect(api.speechTakes).toHaveBeenCalledWith("talk"); expect(useSpeech.getState().job).toBeNull(); save.mockRestore();
  });
  it("does not generate over a save conflict", async () => {
    const save = vi.spyOn(useNarration.getState(), "save").mockResolvedValue(false);
    const generate = vi.spyOn(api, "generateSpeech").mockClear(); await useSpeech.getState().generate("talk", "intro");
    expect(generate).not.toHaveBeenCalled(); expect(useSpeech.getState().error).toContain("resolve"); save.mockRestore();
  });
  it("cancels only the active job and preserves previous recordings on failure", async () => {
    const pending = deferred<never>(); vi.spyOn(api, "generateSpeech").mockReturnValue(pending.promise);
    const take = { id: "old" } as SpeechTake; useSpeech.setState({ takes: { intro: take } });
    const run = useSpeech.getState().generate("talk", "intro"); await Promise.resolve(); await Promise.resolve();
    const job = useSpeech.getState().job!; const cancel = vi.spyOn(api, "cancelSpeech").mockResolvedValue();
    handlers.get("speech-event")!({ payload: { job: { ...job, id: "other", detail: "Wrong" }, error: null } });
    expect(useSpeech.getState().job?.detail).not.toBe("Wrong");
    await useSpeech.getState().cancel(); expect(cancel).toHaveBeenCalledWith(job.id); expect(useSpeech.getState().cancelling).toBe(true);
    // A rejected invocation leaves the accepted take intact.
    vi.mocked(api.speechTakes).mockResolvedValue({ intro: take });
    handlers.get("speech-event")!({ payload: { job: { ...job, stage: "failed" }, error: "Cancelled" } });
    expect(useSpeech.getState().takes.intro).toBe(take);
    pending.reject(new Error("Cancelled")); await run;
    expect(useSpeech.getState().takes.intro).toBe(take);
    expect(useSpeech.getState().job).toBeNull();
    expect(useSpeech.getState().cancelling).toBe(false);
  });
  it("ignores a stale take load after changing decks", async () => {
    const pending = deferred<Record<string, SpeechTake>>(); vi.spyOn(api, "speechTakes").mockReturnValueOnce(pending.promise).mockResolvedValueOnce({});
    const old = useSpeech.getState().loadTakes("talk"); await useSpeech.getState().loadTakes("next"); pending.resolve({ intro: { id: "old" } as SpeechTake }); await old;
    expect(useSpeech.getState().deckId).toBe("next"); expect(useSpeech.getState().takes).toEqual({});
  });
});
