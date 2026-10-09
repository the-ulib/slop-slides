import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
// The panels have their own tests; here only the layout matters.
vi.mock("./components/Home", () => ({ Home: () => <div data-testid="stub-home" /> }));
vi.mock("./components/TopBar", () => ({ TopBar: () => <div data-testid="stub-top-bar" /> }));
vi.mock("./components/SlideRail", () => ({ SlideRail: () => <div data-testid="stub-rail" /> }));
vi.mock("./components/Stage", () => ({ Stage: () => <div data-testid="stub-stage" /> }));
vi.mock("./components/ChatPanel", () => ({ ChatPanel: () => <input data-testid="stub-chat" aria-label="Chat draft" /> }));
vi.mock("./components/NarrationPanel", () => ({ NarrationPanel: () => <div data-testid="stub-narration" /> }));
vi.mock("./components/Presenter", () => ({ Presenter: () => <div data-testid="stub-presenter" /> }));
vi.mock("./components/CodeView", () => ({
  CodeView: ({ active }: { active: boolean }) => <div data-testid="stub-code" data-active={String(active)} />,
}));

import { App } from "./App";
import { useApp } from "./store";
import { DECK_HTML, deckFor } from "./test/fixtures";

beforeEach(() => {
  useApp.setState({ deck: null, view: "slides", chatOpen: true, sidebarTab: "chat", presenting: false, error: null });
});

// Stub ids are prefixed: react-resizable-panels sets data-testid to each panel's id.
const shown = (id: string) => screen.queryByTestId(`stub-${id}`) !== null;

describe("App", () => {
  it("shows the deck library until a deck is open", () => {
    render(<App />);
    expect(shown("home")).toBe(true);
    expect(shown("top-bar")).toBe(false);
  });

  it("shows the editor for an open deck", () => {
    useApp.setState({ deck: deckFor(DECK_HTML) });
    render(<App />);
    expect(["top-bar", "rail", "stage", "chat"].map(shown)).toEqual([true, true, true, true]);
    expect(shown("home")).toBe(false);
  });

  it("keeps the HTML view mounted while showing slides, so unsaved edits survive", () => {
    useApp.setState({ deck: deckFor(DECK_HTML) });
    render(<App />);
    expect(screen.getByTestId("stub-code").dataset.active).toBe("false");
    act(() => useApp.getState().setView("code"));
    expect(screen.getByTestId("stub-code").dataset.active).toBe("true");
    expect(shown("stage")).toBe(false);
  });

  it("collapses and reopens the chat panel", () => {
    useApp.setState({ deck: deckFor(DECK_HTML) });
    render(<App />);
    act(() => useApp.getState().setChatOpen(false));
    expect(shown("chat")).toBe(false);
    expect(["rail", "stage"].map(shown)).toEqual([true, true]);
    act(() => useApp.getState().setChatOpen(true));
    expect(shown("chat")).toBe(true);
  });

  it("keeps the chat draft mounted when switching the right sidebar to narration", () => {
    useApp.setState({ deck: deckFor(DECK_HTML) });
    render(<App />);
    fireEvent.change(screen.getByLabelText("Chat draft"), { target: { value: "Keep this draft" } });
    fireEvent.click(screen.getByRole("tab", { name: "Narration" }));
    expect(screen.getByRole("tabpanel").getAttribute("id")).toBe("panel-narration");
    fireEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect((screen.getByLabelText("Chat draft") as HTMLInputElement).value).toBe("Keep this draft");
    expect(useApp.getState().selected).toBeNull();
  });

  it("overlays the presenter", () => {
    useApp.setState({ deck: deckFor(DECK_HTML), presenting: true });
    render(<App />);
    expect(shown("presenter")).toBe(true);
    expect(shown("stage")).toBe(true);
  });

  it("shows errors in a dismissible toast", () => {
    render(<App />);
    expect(screen.queryByText("deck not found: x")).toBeNull();
    act(() => useApp.getState().setError("deck not found: x"));
    expect(screen.getByText("deck not found: x")).toBeTruthy();
    fireEvent.click(screen.getByRole("button"));
    expect(useApp.getState().error).toBeNull();
    expect(screen.queryByText("deck not found: x")).toBeNull();
  });
});

describe("lint status", () => {
  it("re-lints the open deck whenever deck.html changes", async () => {
    const refreshLint = vi.fn(async () => {});
    useApp.setState({ deck: deckFor(DECK_HTML), refreshLint });
    render(<App />);
    expect(refreshLint).toHaveBeenCalledTimes(1);
    act(() => useApp.setState({ deck: deckFor(DECK_HTML) }));
    expect(refreshLint).toHaveBeenCalledTimes(1);
    act(() => useApp.setState({ deck: deckFor(DECK_HTML, "2") }));
    expect(refreshLint).toHaveBeenCalledTimes(2);
  });

  it("does not lint without an open deck", () => {
    const refreshLint = vi.fn(async () => {});
    useApp.setState({ refreshLint });
    render(<App />);
    expect(refreshLint).not.toHaveBeenCalled();
  });
});
