import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "./lib/api";
import { useNarration } from "./narrationStore";
import type { SpeechEvent, SpeechJob, SpeechStatus, SpeechTake } from "./lib/speech";
interface SpeechState {
  status: SpeechStatus | null; job: SpeechJob | null; error: string | null; message: string | null;
  deckId: string | null; takes: Record<string, SpeechTake>; cancelling: boolean;
  initialize: () => Promise<void>; refresh: () => Promise<void>; loadTakes: (id: string | null) => Promise<void>;
  install: (source?: string) => Promise<void>; remove: () => Promise<void>;
  generate: (id: string, slide: string | null) => Promise<void>; cancel: () => Promise<void>;
}
let listening: Promise<void> | undefined;
let takeRequest = 0;
const terminal = (job: SpeechJob) => ["complete", "failed"].includes(job.stage);
function newJob(kind: string, deckId: string | null): SpeechJob {
  return { id: crypto.randomUUID(), kind, deckId, sourceRevision: null, stage: "starting", completed: 0, total: 0, detail: kind === "setup" ? "Setting up local speech…" : "Preparing narration…" };
}
export const useSpeech = create<SpeechState>((set, get) => ({
  status: null, job: null, error: null, message: null, deckId: null, takes: {}, cancelling: false,
  initialize: async () => {
    listening ??= listen<SpeechEvent>("speech-event", ({ payload }) => {
      if (get().job?.id !== payload.job.id) return;
      if (terminal(payload.job)) {
        set({ job: null, cancelling: false, error: payload.error, message: payload.error ? null : payload.job.detail });
        void get().refresh();
        const id = get().deckId; if (id) void get().loadTakes(id);
        if (payload.job.deckId === useNarration.getState().deckId) void useNarration.getState().refresh();
      } else set({ job: payload.job });
    }).then(() => {});
    await listening; await get().refresh();
  },
  refresh: async () => {
    try { const status = await api.speechStatus(); if (status) set({ status, ...(status.job && !terminal(status.job) && !get().job ? { job: status.job } : {}) }); }
    catch (e) { set({ error: errorMessage(e) }); }
  },
  loadTakes: async (deckId) => {
    const run = ++takeRequest;
    if (get().deckId !== deckId) set({ deckId, takes: {} });
    if (!deckId) return;
    try { const takes = await api.speechTakes(deckId); if (run === takeRequest) set({ takes: takes ?? {} }); }
    catch (e) { if (run === takeRequest) set({ error: errorMessage(e) }); }
  },
  install: async (source) => {
    if (get().job) return;
    const job = newJob("setup", null); set({ job, error: null, message: null, cancelling: false });
    try { await api.installSpeechPack(job.id, source ?? null); set({ message: "Local speech is ready." }); }
    catch (e) { set({ error: errorMessage(e) }); }
    finally { if (get().job?.id === job.id) set({ job: null, cancelling: false }); await get().refresh(); }
  },
  remove: async () => {
    if (get().job) return;
    const job = newJob("remove", null); set({ job, error: null, message: null });
    try { await api.removeSpeechPack(); }
    catch (e) { set({ error: errorMessage(e) }); }
    finally { if (get().job?.id === job.id) set({ job: null }); await get().refresh(); }
  },
  generate: async (id, slide) => {
    if (get().job) return;
    const job = newJob("generation", id); set({ job, error: null, message: null, cancelling: false });
    try {
      if (useNarration.getState().deckId !== id || !(await useNarration.getState().save())) throw new Error("Save or resolve your narration edits before generating audio.");
      const result = await api.generateSpeech(job.id, id, slide);
      set({ message: result?.superseded ? "The script changed during generation. Retry to use its latest version; the recording is cached." : "Audio ready. Press Play below the slide." });
      if (useNarration.getState().deckId === id) await useNarration.getState().refresh();
      if (get().deckId === id) await get().loadTakes(id);
    } catch (e) { set({ error: errorMessage(e) }); }
    finally { if (get().job?.id === job.id) set({ job: null, cancelling: false }); }
  },
  cancel: async () => {
    const job = get().job; if (!job) return;
    set({ cancelling: true });
    try { await api.cancelSpeech(job.id); } catch (e) { set({ error: errorMessage(e), cancelling: false }); }
  },
}));
