import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));
import { useSpeech } from "../speechStore";
import { testSpeechProvider } from "../test/speech";
import { api } from "../lib/api";
import { useApp } from "../store";
import { useNarration } from "../narrationStore";
import { emptyNarration, emptyScript, slideReviewHash } from "../lib/narration";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { NarrationPanel } from "./NarrationPanel";
beforeEach(() => {
  vi.spyOn(api, "speechHistory").mockResolvedValue([]);
  vi.useFakeTimers();
  useSpeech.setState({ status: { providers: [testSpeechProvider()], job: null }, job: null, error: null, message: null, preview: null, takes: {} });
  vi.spyOn(useSpeech.getState(), "initialize").mockResolvedValue();
  vi.spyOn(useSpeech.getState(), "loadTakes").mockResolvedValue();
  useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro", running: false });
  useNarration.setState({ deckId: "talk", document: emptyNarration(), edits: {}, languageEdit: null, error: null, conflict: null, saving: false });
});
afterEach(async () => { await useNarration.getState().load(null); vi.useRealTimers(); });
describe("NarrationPanel", () => {
  it("puts voice creation before the script and allows it without a selected slide", () => {
    useSpeech.setState({ status: { providers: [{ ...testSpeechProvider(), supportsCloning: true, cloneReady: true }], job: null, presenters: [] } });
    render(<NarrationPanel />);
    const create = screen.getByRole("button", { name: "Create voice from recording…" });
    expect(create.compareDocumentPosition(screen.getByLabelText("Narration script")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    act(() => useApp.setState({ selected: null }));
    expect((create as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(create);
    expect(screen.getByRole("dialog", { name: "Create voice from recording" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Import reference audio…" })).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Close voice setup"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("starts missing and legacy unset silent slides at 5 seconds and retains custom durations", () => {
    render(<NarrationPanel />);
    expect((screen.getByLabelText("Silent slide duration") as HTMLInputElement).value).toBe("5");
    const document = emptyNarration();
    document.manifest.slides.intro = { ...emptyScript(), silentDurationMs: null };
    act(() => useNarration.setState({ document }));
    expect((screen.getByLabelText("Silent slide duration") as HTMLInputElement).value).toBe("5");
    fireEvent.change(screen.getByLabelText("Silent slide duration"), { target: { value: "7.5" } });
    expect(useNarration.getState().edits.intro!.silentDurationMs).toBe(7500);
    act(() => useApp.getState().select("outro"));
    expect((screen.getByLabelText("Silent slide duration") as HTMLInputElement).value).toBe("5");
    act(() => useApp.getState().select("intro"));
    expect((screen.getByLabelText("Silent slide duration") as HTMLInputElement).value).toBe("7.5");
  });
  it("preserves scripts when selecting another slide and flags visual edits for review", () => {
    render(<NarrationPanel />);
    fireEvent.change(screen.getByLabelText("Narration script"), { target: { value: "Welcome" } });
    expect(screen.getByText("Reviewed")).toBeTruthy();
    act(() => useApp.getState().select("outro"));
    fireEvent.change(screen.getByLabelText("Narration script"), { target: { value: "Thanks" } });
    act(() => useApp.getState().select("intro"));
    expect((screen.getByLabelText("Narration script") as HTMLTextAreaElement).value).toBe("Welcome");
    act(() => useApp.getState().setDeck({ ...deckFor(DECK_HTML), shellHash: "changed" }));
    expect(screen.getByText("Review needed")).toBeTruthy();
    fireEvent.click(screen.getByText("Mark reviewed"));
    expect(useNarration.getState().edits.intro!.reviewedSlideHash).toBe(slideReviewHash(useApp.getState().deck!, "intro"));
  });
  it("provides language overrides, pauses and explicit silence", () => {
    render(<NarrationPanel />);
    fireEvent.change(screen.getByLabelText("Deck narration language"), { target: { value: "de" } });
    fireEvent.change(screen.getByLabelText("Slide narration language"), { target: { value: "en" } });
    fireEvent.change(screen.getByLabelText("Pause after speech"), { target: { value: "900" } });
    fireEvent.change(screen.getByLabelText("Silent slide duration"), { target: { value: "4" } });
    expect(useNarration.getState().languageEdit).toBe("de");
    expect(useNarration.getState().edits.intro).toMatchObject({ languageOverride: "en", tailMs: 900, silentDurationMs: 4000 });
  });
  it("retains accepted-take references when editing scripts or language", () => {
    const document = emptyNarration();
    document.manifest.slides.intro = { ...emptyScript(), text: "Old text", acceptedTakeId: "previous-take" };
    useNarration.setState({ document });
    render(<NarrationPanel />);
    fireEvent.change(screen.getByLabelText("Narration script"), { target: { value: "New text" } });
    fireEvent.change(screen.getByLabelText("Slide narration language"), { target: { value: "de" } });
    expect(useNarration.getState().edits.intro?.acceptedTakeId).toBeUndefined();
    expect(useNarration.getState().document?.manifest.slides.intro?.acceptedTakeId).toBe("previous-take");
  });
  it("can recover a deleted script without erasing the original", () => {
    const document = emptyNarration();
    document.manifest.slides.deleted = { ...emptyScript(), text: "Recovered text" };
    useNarration.setState({ document });
    render(<NarrationPanel />);
    fireEvent.click(screen.getByText("Copy to selected slide"));
    expect(useNarration.getState().edits.intro!.text).toBe("Recovered text");
    expect(useNarration.getState().document?.manifest.slides.deleted!.text).toBe("Recovered text");
  });
  it("cannot edit another deck's manifest while a deck switch is loading", () => {
    useNarration.setState({ deckId: "next-deck" });
    render(<NarrationPanel />);
    expect((screen.getByLabelText("Narration script") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByText("Draft narration") as HTMLButtonElement).disabled).toBe(true);
  });
  it("disables editing and drafting on a corrupt narration file", () => {
    useNarration.setState({ document: null, error: "Cannot read narration.json" });
    render(<NarrationPanel />);
    expect((screen.getByLabelText("Narration script") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByText("Draft narration") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("Cannot read");
  });
});
it("inserts a pause at the cursor and estimates spoken words plus silence", () => {
  const document = emptyNarration();
  document.manifest.slides.intro = { ...emptyScript(), text: "First. Second." };
  useNarration.setState({ document });
  render(<NarrationPanel />);
  const input = screen.getByLabelText("Narration script") as HTMLTextAreaElement;
  input.setSelectionRange(6, 6);
  fireEvent.click(screen.getByText("Insert pause"));
  expect(input.value).toBe("First. [pause:800ms] Second.");
  expect(screen.getByText("0.8s of explicit pauses")).toBeTruthy();
  expect(screen.getByText(/2 words/)).toBeTruthy();
  expect(useNarration.getState().edits.intro?.text).toBe(input.value);
});
it("keeps an unsupported tone marker editable but blocks speech until corrected", () => {
  render(<NarrationPanel />);
  fireEvent.change(screen.getByLabelText("Narration script"), { target: { value: "Hello [tone:confident] world." } });
  expect(screen.getByRole("alert").textContent).toContain("Unsupported narration marker");
  expect((screen.getByText("Generate audio") as HTMLButtonElement).disabled).toBe(true);
  expect(useNarration.getState().edits.intro?.text).toBe("Hello [tone:confident] world.");
  fireEvent.change(screen.getByLabelText("Narration script"), { target: { value: "Hello [pause:800ms] world." } });
  expect(screen.queryByRole("alert")).toBeNull();
  expect((screen.getByText("Generate audio") as HTMLButtonElement).disabled).toBe(false);
});
