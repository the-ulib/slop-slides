import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../lib/voiceRecording", () => ({ startVoiceRecording: vi.fn() }));
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { startVoiceRecording } from "../lib/voiceRecording";
import { testSpeechProvider } from "../test/speech";
import { useSpeech } from "../speechStore";
import { VoiceSetup } from "./VoiceSetup";
const profile = { id: "profile:123", revision: "rev1", name: "My voice", referenceLanguage: "de" as const, ready: true };
const provider = () => ({ ...testSpeechProvider(), supportsCloning: true, cloneReady: true, cloneSetup: { totalBytes: 2516100912, detail: "Stays local.", importTitle: "Base pack" } });
beforeEach(() => {
  useSpeech.setState({ status: { providers: [provider()], job: null, presenters: [] }, job: null, error: null });
  vi.spyOn(useSpeech.getState(), "refresh").mockResolvedValue();
  vi.spyOn(api, "importVoiceRecording").mockResolvedValue({ id: "recording1", path: "/sample.wav" });
  vi.spyOn(api, "createVoiceProfile").mockResolvedValue(profile);
  vi.spyOn(api, "previewVoiceProfile").mockResolvedValue();
  vi.spyOn(api, "voiceProfileAction").mockResolvedValue(profile);
  vi.spyOn(api, "setDefaultPresenter").mockResolvedValue();
  vi.spyOn(api, "releaseVoiceRecording").mockResolvedValue();
  vi.mocked(open).mockResolvedValue("/sample.wav");
});
async function prepare() {
  fireEvent.click(screen.getByText("Import recording…"));
  await screen.findByText("Recording ready: sample.wav");
  fireEvent.change(screen.getByLabelText("Voice recording transcript"), { target: { value: "Exact spoken words." } });
  fireEvent.click(screen.getByRole("checkbox", { name: /I am the speaker/ }));
}
describe("voice setup", () => {
  it("requires a reference, transcript and consent, then listening before save/default", async () => {
    const onSaved = vi.fn(); render(<VoiceSetup provider={provider()} initialLanguage="de" onClose={vi.fn()} onSaved={onSaved} />);
    expect((screen.getByText("Create voice preview") as HTMLButtonElement).disabled).toBe(true);
    await prepare(); fireEvent.click(screen.getByText("Create voice preview"));
    await screen.findByLabelText("Voice setup preview");
    expect(api.createVoiceProfile).toHaveBeenCalledWith(expect.any(String), "qwen-local", expect.objectContaining({ transcript: "Exact spoken words.", reference: "/sample.wav", language: "de", authorized: true }));
    expect(api.voiceProfileAction).not.toHaveBeenCalled(); expect((screen.getByText("Save presenter") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.ended(screen.getByLabelText("Voice setup preview"));
    fireEvent.click(screen.getByRole("checkbox", { name: "Use as default presenter for new decks" }));
    fireEvent.click(screen.getByText("Save presenter"));
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(profile));
    expect(api.voiceProfileAction).toHaveBeenCalledWith("qwen-local", "save", "rev1", null);
    expect(api.setDefaultPresenter).toHaveBeenCalledWith({ providerId: "qwen-local", presenterId: "profile:123" });
  });
  it("keeps the draft after preview failure and discards it on close, without saving", async () => {
    vi.mocked(api.previewVoiceProfile).mockRejectedValue(new Error("Generation cancelled"));
    const { unmount } = render(<VoiceSetup provider={provider()} initialLanguage="de" onClose={vi.fn()} onSaved={vi.fn()} />);
    await prepare(); fireEvent.click(screen.getByText("Create voice preview"));
    await screen.findByText("Generation cancelled"); expect((screen.getByText("Save presenter") as HTMLButtonElement).disabled).toBe(true);
    unmount(); expect(api.voiceProfileAction).toHaveBeenCalledWith("qwen-local", "discard", "rev1"); expect(api.setDefaultPresenter).not.toHaveBeenCalled();
  });
  it("offers import after microphone denial and releases capture on unmount", async () => {
    vi.mocked(startVoiceRecording).mockRejectedValueOnce(new Error("Permission denied"));
    const { unmount } = render(<VoiceSetup provider={provider()} initialLanguage="en" onClose={vi.fn()} onSaved={vi.fn()} />);
    fireEvent.click(screen.getByText("Record here")); await screen.findByText(/Permission denied.*import a WAV/);
    const cancel = vi.fn(); vi.mocked(startVoiceRecording).mockResolvedValueOnce({ cancel, stop: vi.fn() });
    fireEvent.click(screen.getByText("Record here")); await screen.findByText("Read this passage naturally");
    expect((screen.getByLabelText("Voice recording transcript") as HTMLTextAreaElement).value).toContain("Good presentations");
    unmount(); expect(cancel).toHaveBeenCalled();
  });
  it("does not accept hearing the other language as approval for the primary preview", async () => {
    render(<VoiceSetup provider={provider()} initialLanguage="de" onClose={vi.fn()} onSaved={vi.fn()} />);
    await prepare(); fireEvent.click(screen.getByText("Create voice preview")); await screen.findByLabelText("Voice setup preview");
    fireEvent.change(screen.getByLabelText("Voice preview language"), { target: { value: "en" } });
    fireEvent.click(screen.getByText("Generate preview")); await waitFor(() => expect(api.previewVoiceProfile).toHaveBeenCalledWith(expect.any(String), "qwen-local", "rev1", "en"));
    await act(async () => {}); fireEvent.ended(screen.getByLabelText("Voice setup preview")); expect((screen.getByText("Save presenter") as HTMLButtonElement).disabled).toBe(true);
  });
});
