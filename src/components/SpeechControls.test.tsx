import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
import { useApp } from "../store";
import { useNarration } from "../narrationStore";
import { useSpeech } from "../speechStore";
import { emptyNarration, emptyScript } from "../lib/narration";
import type { SpeechTake } from "../lib/speech";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { testSpeechProvider, alternateSpeechProvider } from "../test/speech";
import { SpeechControls } from "./SpeechControls";
import { SpeechPlayback } from "./SpeechPlayback";
const doc = emptyNarration(); doc.manifest.slides.intro = { ...emptyScript(), text: "Welcome" };
const take: SpeechTake = { id: "recording", key: "key", engineVersion: "v1", modelRevision: "rev", source: { text: "Welcome", language: "en", presenterId: "preset:ryan", pace: 1.1 }, samples: 240000, sampleRate: 24000, sha256: "hash" };
beforeEach(() => {
  useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
  useNarration.setState({ deckId: "talk", document: doc, edits: {}, settingsEdits: {}, languageEdit: null, error: null });
  useSpeech.setState({ status: { providers: [testSpeechProvider()], job: null }, deckId: "talk", takes: {}, job: null, error: null, message: null });
  vi.spyOn(useSpeech.getState(), "initialize").mockResolvedValue(); vi.spyOn(useSpeech.getState(), "loadTakes").mockResolvedValue();
});
it("offers generation and keeps the current presenter and pace in narration", () => {
  const presenter = vi.spyOn(useNarration.getState(), "setPresenter"); const pace = vi.spyOn(useNarration.getState(), "setPace");
  const generate = vi.spyOn(useSpeech.getState(), "generate").mockResolvedValue();
  render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  fireEvent.change(screen.getByLabelText("Narration presenter"), { target: { value: "preset:aiden" } }); expect(presenter).toHaveBeenCalledWith("preset:aiden", "Aiden");
  fireEvent.change(screen.getByLabelText("Narration pace"), { target: { value: "1.2" } }); expect(pace).toHaveBeenCalledWith(1.2);
  fireEvent.click(screen.getByText("Generate audio")); expect(generate).toHaveBeenCalledWith("talk", "intro");
});
it("shows actual duration and identifies old audio after a script edit", () => {
  useSpeech.setState({ takes: { intro: take } }); render(<SpeechPlayback />);
  expect(screen.getByText("10.0s · 1.1× pace")).toBeTruthy();
  expect(screen.getByLabelText("Narration audio").getAttribute("src")).toContain("/audio/recording.wav");
  act(() => useNarration.setState({ edits: { intro: { text: "Changed" } } })); expect(screen.getByText("Previous recording")).toBeTruthy();
  act(() => useApp.setState({ selected: "outro" })); expect(screen.queryByLabelText("Narration audio")).toBeNull();
});
it("explains setup size and blocks simultaneous generation", () => {
  useSpeech.setState({ status: { providers: [{ ...testSpeechProvider(), ready: false }], job: null } });
  render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  expect(screen.getByText(/2.50 GB/)).toBeTruthy(); expect(screen.getByText("Download voice pack")).toBeTruthy();
  act(() => useSpeech.setState({ job: { id: "job", kind: "setup", deckId: null, sourceRevision: null, stage: "installing", completed: 1, total: 10, detail: "Installing" } }));
  expect((screen.getByText("Download voice pack") as HTMLButtonElement).disabled).toBe(true); expect(screen.getByText("Cancel")).toBeTruthy();
});

it("uses alternate provider capabilities and requires an explicit compatible presenter", () => {
  useSpeech.setState({ status: { providers: [testSpeechProvider(), alternateSpeechProvider()], job: null }, takes: { intro: take } });
  const choose = vi.spyOn(useNarration.getState(), "setProvider");
  const view = render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  fireEvent.change(screen.getByLabelText("Speech provider"), { target: { value: "fixture-tone" } });
  expect(choose).toHaveBeenCalledWith("fixture-tone");
  const switched = { ...doc.manifest, speechProviderId: "fixture-tone" };
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={switched} editable />);
  expect((screen.getByText("Generate audio") as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("440 Hz test tone")).toBeTruthy();
  expect(screen.queryByText("Aiden")).toBeNull();
  expect(screen.queryByText("Manage voice pack")).toBeNull();
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={{ ...switched, presenterId: "tone:440", presenterNameSnapshot: "440 Hz test tone", pace: 1 }} editable />);
  expect((screen.getByText("Generate audio") as HTMLButtonElement).disabled).toBe(false);
  expect(screen.getByText("2.0×")).toBeTruthy();
});
it("keeps accepted audio playable with its provider missing", () => {
  useSpeech.setState({ status: { providers: [], job: null }, takes: { intro: take } });
  render(<SpeechPlayback />);
  expect(screen.getByLabelText("Narration audio")).toBeTruthy();
  expect(screen.getByText("Speech preview")).toBeTruthy();
});
