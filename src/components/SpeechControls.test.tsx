import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
import { api } from "../lib/api";
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
  vi.spyOn(api, "speechHistory").mockResolvedValue([]);
  useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro" });
  useNarration.setState({ deckId: "talk", document: doc, edits: {}, settingsEdits: {}, languageEdit: null, error: null });
  useSpeech.setState({ status: { providers: [testSpeechProvider()], job: null }, deckId: "talk", takes: {}, job: null, error: null, message: null });
  vi.spyOn(useSpeech.getState(), "initialize").mockResolvedValue(); vi.spyOn(useSpeech.getState(), "loadTakes").mockResolvedValue();
});
it("applies presenter and pace to this slide without changing deck defaults", () => {
  const edit = vi.spyOn(useNarration.getState(), "edit");
  const generate = vi.spyOn(useSpeech.getState(), "generate").mockResolvedValue();
  render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  fireEvent.change(screen.getByLabelText("Narration presenter"), { target: { value: "preset:aiden" } }); expect(edit).toHaveBeenCalledWith("intro", expect.objectContaining({ presenterIdOverride: "preset:aiden", presenterNameSnapshotOverride: "Aiden", speechProviderIdOverride: "qwen-local" }));
  fireEvent.change(screen.getByLabelText("Narration pace"), { target: { value: "1.2" } }); expect(edit).toHaveBeenCalledWith("intro", expect.objectContaining({ paceOverride: 1.2 }));
  expect(useNarration.getState().settingsEdits).toEqual({});
  fireEvent.click(screen.getByText("Generate audio")); expect(generate).toHaveBeenCalledWith("talk", "intro", false);
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
  const choose = vi.spyOn(useNarration.getState(), "edit");
  const view = render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  fireEvent.change(screen.getByLabelText("Speech provider"), { target: { value: "fixture-tone" } });
  expect(choose).toHaveBeenCalledWith("intro", expect.objectContaining({ speechProviderIdOverride: "fixture-tone" }));
  const switched = { ...doc.manifest, slides: { ...doc.manifest.slides, intro: { ...doc.manifest.slides.intro!, speechProviderIdOverride: "fixture-tone" } } };
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={switched} editable />);
  expect((screen.getByText("Generate another take") as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("440 Hz test tone")).toBeTruthy();
  expect(screen.queryByText("Aiden")).toBeNull();
  expect(screen.queryByText("Manage voice pack")).toBeNull();
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={{ ...switched, slides: { ...switched.slides, intro: { ...switched.slides.intro!, presenterIdOverride: "tone:440", presenterNameSnapshotOverride: "440 Hz test tone", paceOverride: 1 } } }} editable />);
  expect((screen.getByText("Generate another take") as HTMLButtonElement).disabled).toBe(false);
  expect(screen.getByText("2.0×")).toBeTruthy();
});
it("keeps accepted audio playable with its provider missing", () => {
  useSpeech.setState({ status: { providers: [], job: null }, takes: { intro: take } });
  render(<SpeechPlayback />);
  expect(screen.getByLabelText("Narration audio")).toBeTruthy();
  expect(screen.getByText("Speech preview")).toBeTruthy();
});

it("offers explicit deck defaults and returns to slide scope on navigation", () => {
  const view = render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  fireEvent.change(screen.getByLabelText("Speech settings scope"), { target: { value: "deck" } });
  fireEvent.change(screen.getByLabelText("Narration pace"), { target: { value: "1.2" } });
  expect(useNarration.getState().settingsEdits.pace).toBe(1.2);
  expect(screen.getByText(/Changes affect slides using deck defaults/)).toBeTruthy();
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="outro" manifest={doc.manifest} editable />);
  expect((screen.getByLabelText("Speech settings scope") as HTMLSelectElement).value).toBe("slide");
});
it("restores the accepted recording's settings without replacing its script or take", () => {
  useSpeech.setState({ takes: { intro: take } });
  const changed = { ...doc.manifest, pace: 1.2, presenterId: "preset:aiden" };
  const view = render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={changed} editable />);
  fireEvent.click(screen.getByText("Restore recording settings"));
  expect(useNarration.getState().edits.intro).toMatchObject({ paceOverride: 1.1, presenterIdOverride: "preset:ryan", languageOverride: "en" });
  expect(useNarration.getState().edits.intro).not.toHaveProperty("text");
  expect(useNarration.getState().edits.intro).not.toHaveProperty("acceptedTakeId");
  view.rerender(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={{ ...changed, slides: { intro: { ...doc.manifest.slides.intro!, text: "Different words" } } }} editable />);
  expect(screen.queryByText("Restore recording settings")).toBeNull();
});
it("clears explicit slide settings to inherit defaults again", () => {
  const manifest = { ...doc.manifest, slides: { intro: { ...doc.manifest.slides.intro!, presenterIdOverride: "preset:aiden", paceOverride: 1.2 } } };
  render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={manifest} editable />);
  expect((screen.getByLabelText("Narration presenter") as HTMLSelectElement).value).toBe("preset:aiden");
  fireEvent.click(screen.getByText("Use deck defaults"));
  expect(useNarration.getState().edits.intro).toMatchObject({ speechProviderIdOverride: null, presenterIdOverride: null, paceOverride: null });
});

it("keeps advanced controls closed and explicitly generates a new take", () => {
  useSpeech.setState({ takes: { intro: take } });
  const generate = vi.spyOn(useSpeech.getState(), "generate").mockResolvedValue();
  render(<SpeechControls deck={deckFor(DECK_HTML)} selected="intro" manifest={doc.manifest} editable />);
  expect((screen.getByText("Voice & timing").closest("details") as HTMLDetailsElement).open).toBe(false);
  expect((screen.getByText("Generate audio for all slides").closest("details") as HTMLDetailsElement).open).toBe(false);
  fireEvent.click(screen.getByText("Generate another take"));
  expect(generate).toHaveBeenCalledWith("talk", "intro", true);
});
it("previews history in the player without changing the accepted recording", () => {
  useSpeech.setState({ takes: { intro: take }, preview: { deckId: "talk", slide: "intro", take: { ...take, id: "older" } } });
  render(<SpeechPlayback />);
  expect(screen.getByText("History preview")).toBeTruthy();
  expect(screen.getByLabelText("Narration audio").getAttribute("src")).toContain("/audio/older.wav");
  fireEvent.click(screen.getByText("Back to selected"));
  expect(screen.getByLabelText("Narration audio").getAttribute("src")).toContain("/audio/recording.wav");
  expect(useSpeech.getState().takes.intro?.id).toBe("recording");
});
it("restarts playback when listening to the accepted take or replaying a history take", () => {
  useSpeech.setState({ preview: null, takes: { intro: take } });
  render(<SpeechPlayback />);
  const original = screen.getByLabelText("Narration audio");
  act(() => useSpeech.setState({ preview: { deckId: "talk", slide: "intro", take, requestId: "first" } }));
  const first = screen.getByLabelText("Narration audio");
  expect(first).not.toBe(original); expect((first as HTMLAudioElement).autoplay).toBe(true);
  act(() => useSpeech.setState({ preview: { deckId: "talk", slide: "intro", take, requestId: "second" } }));
  expect(screen.getByLabelText("Narration audio")).not.toBe(first);
});
