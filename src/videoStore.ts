import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorMessage } from "./lib/api";
import type { VideoTimeline, VideoProgress } from "./lib/video";
import { useNarration } from "./narrationStore";
import { useApp } from "./store";

interface VideoState {
  deckId: string | null; jobId: string | null; timeline: VideoTimeline | null; busy: boolean;
  progress: VideoProgress | null; error: string | null; output: string | null;
  open: (id: string, exportAfter?: boolean) => Promise<void>;
  export: () => Promise<void>;
  close: () => void;
}
let run = 0;
async function events(jobId: string) {
  return listen<VideoProgress>("video-progress", ({ payload }) => {
    if (useVideo.getState().jobId === jobId && useVideo.getState().busy) useVideo.setState({ progress: payload });
  });
}
export const useVideo = create<VideoState>((set, get) => ({
  deckId: null, jobId: null, timeline: null, busy: false, progress: null, error: null, output: null,
  open: async (id, exportAfter = false) => {
    if (get().busy) return;
    get().close();
    const current = ++run;
    const jobId = crypto.randomUUID();
    set({ deckId: id, jobId, busy: true, error: null, progress: { id: jobId, stage: "preparing", completed: 0, total: 0 } });
    let unlisten: (() => void) | undefined;
    try {
      if (useApp.getState().codeDirty) throw new Error("Save your HTML changes before preparing a video.");
      if (useNarration.getState().deckId === id && !(await useNarration.getState().save())) throw new Error("Resolve the narration save conflict before preparing a video.");
      if (current !== run) return;
      unlisten = await events(jobId);
      if (current !== run) return;
      const timeline = await api.prepareVideo(id, jobId);
      if (current !== run) { await api.releaseVideo(jobId); return; }
      set({ timeline, busy: false, progress: null });
      if (exportAfter) await get().export();
    } catch (e) { if (current === run) set({ error: errorMessage(e), busy: false, progress: null }); }
    finally { unlisten?.(); }
  },
  export: async () => {
    const { timeline, jobId, busy } = get();
    if (!timeline || !jobId || busy) return;
    const current = run;
    let unlisten: (() => void) | undefined;
    try {
      set({ busy: true, error: null });
      const title = useApp.getState().deck?.title ?? "presentation";
      const dest = await save({ title: "Export narrated video", defaultPath: `${title.replace(/[\\/:*?"<>|]+/g, "").trim() || "presentation"}.mp4`, filters: [{ name: "MP4 video", extensions: ["mp4"] }] });
      if (current !== run) return;
      if (!dest) { set({ busy: false }); return; }
      set({ busy: true, output: null, error: null, progress: { id: jobId, stage: "encoding", completed: 0, total: timeline.totalFrames } });
      unlisten = await events(jobId);
      if (current !== run) return;
      await api.exportVideo(jobId, dest);
      if (current !== run) return;
      set({ busy: false, progress: null, output: dest });
      void revealItemInDir(dest).catch(() => {});
    } catch (e) { if (current === run) set({ error: errorMessage(e), busy: false, progress: null }); }
    finally { unlisten?.(); if (current === run) set({ busy: false }); }
  },
  close: () => {
    ++run;
    const { jobId, busy } = get();
    set({ deckId: null, jobId: null, timeline: null, busy: false, progress: null, error: null, output: null });
    if (jobId) void (busy ? api.cancelVideo(jobId) : Promise.resolve()).catch(() => {}).then(() => api.releaseVideo(jobId)).catch(() => {});
  },
}));
