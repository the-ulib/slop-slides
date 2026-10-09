import { beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(async () => () => {}), save: vi.fn(), reveal: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: mocks.reveal }));
import { useVideo } from "./videoStore";
import { useApp } from "./store";
import { useNarration } from "./narrationStore";
import type { VideoTimeline } from "./lib/video";
const timeline = { id: "job", deckId: "talk", totalFrames: 90, slides: [] } as unknown as VideoTimeline;
beforeEach(() => {
  vi.clearAllMocks(); mocks.invoke.mockResolvedValue(timeline); mocks.save.mockResolvedValue("/tmp/talk.mp4"); mocks.reveal.mockResolvedValue(undefined);
  useVideo.setState({ deckId: null, jobId: null, busy: false, timeline: null, error: null, output: null });
  useApp.setState({ codeDirty: false }); useNarration.setState({ deckId: null });
});
it("prepares a frozen job then exports that same job through the save dialog", async () => {
  await useVideo.getState().open("talk", true);
  const jobId = useVideo.getState().jobId;
  expect(mocks.invoke).toHaveBeenCalledWith("prepare_video", { id: "talk", jobId });
  expect(mocks.invoke).toHaveBeenCalledWith("export_video", { jobId, dest: "/tmp/talk.mp4" });
  expect(useVideo.getState().output).toBe("/tmp/talk.mp4");
});
it("does not start encoding when the save picker is cancelled", async () => {
  mocks.save.mockResolvedValue(null);
  await useVideo.getState().open("talk", true);
  expect(mocks.invoke.mock.calls.some(([name]) => name === "export_video")).toBe(false);
});
it("closing while preparation runs rejects its late result and cleans the frozen job", async () => {
  let resolve!: (t: VideoTimeline) => void;
  mocks.invoke.mockImplementation((name) => name === "prepare_video" ? new Promise<VideoTimeline>((r) => { resolve = r; }) : Promise.resolve());
  const pending = useVideo.getState().open("talk");
  await vi.waitFor(() => expect(resolve).toBeDefined());
  const jobId = useVideo.getState().jobId;
  useVideo.getState().close(); resolve(timeline); await pending;
  expect(useVideo.getState().timeline).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledWith("cancel_video", { jobId });
  expect(mocks.invoke).toHaveBeenCalledWith("release_video", { jobId });
});
it("unsaved HTML blocks preparation and backend errors remain actionable", async () => {
  useApp.setState({ codeDirty: true }); await useVideo.getState().open("talk");
  expect(useVideo.getState().error).toContain("Save your HTML"); expect(mocks.invoke).not.toHaveBeenCalledWith("prepare_video", expect.anything());
  useApp.setState({ codeDirty: false }); mocks.invoke.mockRejectedValue("Slide 2: choose a silent duration");
  await useVideo.getState().open("talk"); expect(useVideo.getState().error).toContain("Slide 2");
});
