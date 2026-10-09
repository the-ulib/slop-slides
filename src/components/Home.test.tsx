import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const ask = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: (...args: unknown[]) => ask(...args) }));

import type { DeckSummary } from "../lib/api";
import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { Home } from "./Home";

const NOW = Date.now();
let decks: DeckSummary[];

beforeEach(() => {
  decks = [
    { id: "pitch", title: "Series A pitch", slideCount: 6, firstSlide: "title", updatedMs: NOW - 5 * 60_000 },
    { id: "one", title: "One-pager", slideCount: 1, firstSlide: "only", updatedMs: NOW - 3 * 3_600_000 },
    { id: "blank", title: "Blank", slideCount: 0, firstSlide: null, updatedMs: NOW },
  ];
  invoke.mockReset().mockImplementation(async (command: string, args?: { id?: string; title?: string }) => {
    switch (command) {
      case "list_decks":
        return decks;
      case "delete_deck":
        decks = decks.filter((d) => d.id !== args?.id);
        return;
      case "open_deck":
      case "create_deck":
        return deckFor(DECK_HTML);
      case "load_chat":
        return null;
      case "list_templates":
        return [
          { id: "mine", title: "Mine", builtin: false, path: "/t/mine", slides: ["cover"] },
          { id: "swiss", title: "Swiss Design", builtin: true, path: null, slides: ["title"] },
        ];
      case "agent_running":
        return false;
    }
  });
  ask.mockReset();
  useApp.setState({ deck: null, error: null });
});

const cards = () => screen.queryAllByRole("listitem");

describe("Home", () => {
  it("lists recent decks with slide counts and ages", async () => {
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    expect(cards()[0]!.textContent).toContain("Series A pitch");
    expect(cards()[0]!.textContent).toContain("6 slides · 5m ago");
    expect(cards()[1]!.textContent).toContain("1 slide · 3h ago");
    expect(cards()[2]!.textContent).toContain("0 slides · just now");
  });

  it("shows a placeholder for decks without slides", async () => {
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    expect(cards()[2]!.textContent).toContain("Empty deck");
    expect(cards()[0]!.textContent).not.toContain("Empty deck");
  });

  it("hides the recent list when there are no decks", async () => {
    decks = [];
    render(<Home />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("list_decks"));
    expect(screen.queryByText("Recent decks")).toBeNull();
  });

  it("reports a library that cannot be read", async () => {
    invoke.mockRejectedValue("cannot locate documents folder");
    render(<Home />);
    await waitFor(() => expect(useApp.getState().error).toBe("cannot locate documents folder"));
  });

  it("can retry a failed library read without restarting the app", async () => {
    const normalInvoke = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, ...args: unknown[]) => command === "list_decks"
      ? Promise.reject("Interrupted system call") : normalInvoke(command, ...args));
    render(<Home />);
    await screen.findByRole("button", { name: "Retry opening library" });
    invoke.mockImplementation(normalInvoke);
    fireEvent.click(screen.getByRole("button", { name: "Retry opening library" }));
    await waitFor(() => expect(cards()).toHaveLength(3));
    expect(screen.queryByRole("button", { name: "Retry opening library" })).toBeNull();
    expect(useApp.getState().error).toBeNull();
  });

  it("does not clear an unrelated error when loading the library", async () => {
    let finish!: (value: DeckSummary[]) => void;
    const normalInvoke = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, ...args: unknown[]) => command === "list_decks"
      ? new Promise<DeckSummary[]>((resolve) => { finish = resolve; }) : normalInvoke(command, ...args));
    render(<Home />);
    useApp.getState().setError("An unrelated operation failed");
    await act(async () => finish(decks));
    expect(cards()).toHaveLength(3);
    expect(useApp.getState().error).toBe("An unrelated operation failed");
  });

  it("creates a deck with the typed title", async () => {
    render(<Home />);
    const input = screen.getByPlaceholderText(/Deck title/);
    fireEvent.change(input, { target: { value: "  Quarterly update  " } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: /New deck/ })));
    expect(invoke).toHaveBeenCalledWith("create_deck", { title: "Quarterly update", template: null });
    await waitFor(() => expect(useApp.getState().deck).not.toBeNull());
  });

  it("creates an untitled deck when no title is given", async () => {
    render(<Home />);
    await act(async () => fireEvent.submit(screen.getByPlaceholderText(/Deck title/)));
    expect(invoke).toHaveBeenCalledWith("create_deck", { title: "Untitled deck", template: null });
  });

  it("offers the templates as styles for a new deck", async () => {
    useApp.setState({ templates: undefined });
    render(<Home />);
    const style = screen.getByLabelText("Style") as HTMLSelectElement;
    await waitFor(() => expect(style.options).toHaveLength(3));
    expect([...style.options].map((o) => o.textContent)).toEqual(["Any style", "Mine (yours)", "Swiss Design"]);
    fireEvent.change(style, { target: { value: "swiss" } });
    fireEvent.change(screen.getByPlaceholderText(/Deck title/), { target: { value: "Board" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: /New deck/ })));
    expect(invoke).toHaveBeenCalledWith("create_deck", { title: "Board", template: "swiss" });
  });

  it("opens a deck when its card is clicked", async () => {
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    await act(async () => fireEvent.click(screen.getByText("One-pager")));
    expect(invoke).toHaveBeenCalledWith("open_deck", { id: "one" });
  });

  it("deletes a deck after confirmation and refreshes the list", async () => {
    ask.mockResolvedValue(true);
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    await act(async () => fireEvent.click(screen.getAllByTitle("Delete deck")[1]!));
    expect(ask).toHaveBeenCalledWith(expect.stringContaining("“One-pager”"), expect.objectContaining({ kind: "warning" }));
    expect(invoke).toHaveBeenCalledWith("delete_deck", { id: "one" });
    await waitFor(() => expect(cards()).toHaveLength(2));
    expect(screen.queryByText("One-pager")).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("open_deck", expect.anything());
  });

  it("keeps the deck when deletion is cancelled", async () => {
    ask.mockResolvedValue(false);
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    await act(async () => fireEvent.click(screen.getAllByTitle("Delete deck")[0]!));
    expect(invoke).not.toHaveBeenCalledWith("delete_deck", expect.anything());
    expect(cards()).toHaveLength(3);
  });

  it("reports a failed deletion", async () => {
    ask.mockResolvedValue(true);
    render(<Home />);
    await waitFor(() => expect(cards()).toHaveLength(3));
    invoke.mockRejectedValueOnce("permission denied");
    await act(async () => fireEvent.click(screen.getAllByTitle("Delete deck")[0]!));
    expect(useApp.getState().error).toBe("permission denied");
  });
});
