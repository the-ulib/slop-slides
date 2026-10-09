import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
import { api } from "../lib/api";
import { emptyNarration, emptyScript } from "../lib/narration";
import type { SpeechHistoryTake } from "../lib/speech";
import { useSpeech } from "../speechStore";
import { RecordingHistory } from "./RecordingHistory";
import { testSpeechProvider } from "../test/speech";
const old: SpeechHistoryTake = { id: "older", createdAt: 10000, key: "key", engineVersion: "v1", modelRevision: "rev", source: { text: "Original words", language: "de", presenterId: "preset:ryan", pace: 1.1 }, samples: 240000, sampleRate: 24000, sha256: "hash" };
const accepted = { ...old, id: "latest", source: { ...old.source, text: "Latest words", language: "en" as const } };
const script = { ...emptyScript(), text: "Latest words", acceptedTakeId: "latest" };
const manifest = { ...emptyNarration().manifest, slides: { intro: script } };
beforeEach(() => {
  useSpeech.setState({ preview: null, job: null, status: { providers: [testSpeechProvider()], job: null } });
  vi.spyOn(api, "speechHistory").mockResolvedValue([accepted, old]);
});
it("previews an older take without selecting it, and explains script restoration", async () => {
  const select = vi.spyOn(useSpeech.getState(), "selectTake").mockResolvedValue();
  await act(async () => render(<RecordingHistory deckId="talk" slide="intro" script={script} manifest={manifest} accepted={accepted} editable />));
  expect((screen.getByText("Recording history (2)").closest("details") as HTMLDetailsElement).open).toBe(false);
  fireEvent.click(screen.getByText("Recording history (2)"));
  fireEvent.change(screen.getByLabelText("Recording history"), { target: { value: "older" } });
  fireEvent.click(screen.getByText("Listen to this recording"));
  expect(useSpeech.getState().preview?.take.id).toBe("older"); expect(select).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Recording script").textContent).toBe("Original words");
  fireEvent.click(screen.getByText("Use recording & script"));
  expect(select).toHaveBeenCalledWith("talk", "intro", "older");
});
it("clears a temporary preview on slide navigation and ignores late history", async () => {
  let resolve!: (takes: SpeechHistoryTake[]) => void;
  vi.mocked(api.speechHistory).mockReturnValueOnce(new Promise((r) => { resolve = r; })).mockResolvedValueOnce([]);
  const view = render(<RecordingHistory deckId="talk" slide="intro" script={script} manifest={manifest} editable />);
  act(() => useSpeech.setState({ preview: { deckId: "talk", slide: "intro", take: old } }));
  await act(async () => view.rerender(<RecordingHistory deckId="talk" slide="outro" script={script} manifest={manifest} editable />));
  await act(async () => resolve([old]));
  expect(useSpeech.getState().preview).toBeNull();
  expect(screen.queryByLabelText("Recording history")).toBeNull();
});
it("keeps available accepted recordings visible when loading history fails", async () => {
  vi.mocked(api.speechHistory).mockRejectedValue(new Error("Cannot read history"));
  await act(async () => render(<RecordingHistory deckId="talk" slide="intro" script={script} manifest={manifest} accepted={accepted} editable />));
  expect(screen.getByRole("alert").textContent).toContain("Cannot read history");
  expect(screen.getByText("Selected for video")).toBeTruthy();
});
