import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const reveal = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: (...args: unknown[]) => reveal(...args) }));

import type { TemplateSummary } from "../lib/api";
import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { SlideRail } from "./SlideRail";
import { LayoutPicker } from "./Templates";

const DECK = deckFor(DECK_HTML);
const MINE: TemplateSummary = { id: "mine", title: "Mine", builtin: false, path: "/home/.slopslides/templates/mine", slides: ["cover", "pricing-tiers"] };
const SWISS: TemplateSummary = { id: "swiss", title: "Swiss Design", builtin: true, path: null, slides: ["title", "split", "quote"] };
const BENTO: TemplateSummary = { id: "bento-grid", title: "Bento Grid", builtin: true, path: null, slides: ["title", "stats"] };

beforeEach(() => {
  reveal.mockReset();
  invoke.mockReset().mockImplementation(async (command: string) => {
    switch (command) {
      case "list_templates":
        return [MINE, SWISS, BENTO];
      case "stage_template":
        return ".slopslide/templates/x.html";
    }
  });
  useApp.setState({ deck: DECK, selected: "intro", templates: [MINE, SWISS, BENTO], composerFill: null, error: null, running: false, chatOpen: true });
});

const layoutNames = () => screen.getAllByRole("button", { name: / layout$/ }).map((b) => b.getAttribute("aria-label"));

describe("LayoutPicker", () => {
  it("loads the templates when they are not known yet", async () => {
    useApp.setState({ templates: undefined });
    render(<LayoutPicker mode="add" onDone={() => {}} />);
    expect(screen.getByText(/Loading templates/)).toBeTruthy();
    await waitFor(() => expect(layoutNames()).toEqual(["Cover layout", "Pricing tiers layout"]));
    expect(invoke).toHaveBeenCalledWith("list_templates");
  });

  it("starts on the deck's template without a style switcher when adding", () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    render(<LayoutPicker mode="add" onDone={() => {}} />);
    expect(screen.queryByLabelText("Template")).toBeNull();
    expect(layoutNames()).toEqual(["Title layout", "Split layout", "Quote layout"]);
    expect(screen.getByText(/Adds a copy of the layout/)).toBeTruthy();
  });

  it("starts on the deck's template and lists the others by group when changing", () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    render(<LayoutPicker mode="change" onDone={() => {}} />);
    const select = screen.getByLabelText("Template") as HTMLSelectElement;
    expect(select.value).toBe("swiss");
    expect([...select.querySelectorAll("optgroup")].map((g) => g.label)).toEqual(["Your templates", "Built-in"]);
    expect(screen.getByRole("option", { name: "Swiss Design (this deck)" })).toBeTruthy();
    expect(layoutNames()).toEqual(["Title layout", "Split layout", "Quote layout"]);
  });

  it("switches templates and remembers the last one picked", async () => {
    const done = vi.fn();
    render(<LayoutPicker mode="change" onDone={done} />);
    fireEvent.change(screen.getByLabelText("Template"), { target: { value: "bento-grid" } });
    expect(layoutNames()).toEqual(["Title layout", "Stats layout"]);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Stats layout" })));
    expect(done).toHaveBeenCalled();
    expect(localStorage.getItem("slopslide.layoutTemplate")).toBe("bento-grid");
    await waitFor(() => expect(useApp.getState().composerFill?.text).toContain('"Stats" layout of the "Bento Grid" template'));

    // Next time it opens on the template picked last (the deck has none).
    render(<LayoutPicker mode="change" onDone={() => {}} />);
    expect((screen.getAllByLabelText("Template")[1] as HTMLSelectElement).value).toBe("bento-grid");
  });

  it("adds a copy when the layout comes from the deck's template", async () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    invoke.mockImplementation(async (command: string) =>
      command === "add_template_slide" ? { deck: { ...DECK, template: "swiss" }, slide: "outro" } : undefined,
    );
    render(<LayoutPicker mode="add" onDone={() => {}} />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Quote layout" })));
    expect(invoke).toHaveBeenCalledWith("add_template_slide", { id: DECK.id, template: "swiss", slide: "quote", after: "intro" });
    expect(useApp.getState().selected).toBe("outro");
  });

  it("asks the agent to change the slide's layout", async () => {
    render(<LayoutPicker mode="change" onDone={() => {}} />);
    expect(screen.getByText("Change layout")).toBeTruthy();
    expect(screen.getByText(/rebuild this slide on the layout/)).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Cover layout" })));
    await waitFor(() => expect(useApp.getState().composerFill?.text).toMatch(/^Change the layout of this slide to the "Cover" layout of the "Mine" template/));
  });

  it("says so when there are no templates", () => {
    useApp.setState({ templates: [] });
    render(<LayoutPicker mode="add" onDone={() => {}} />);
    expect(screen.getByText("No templates found.")).toBeTruthy();
    expect(screen.queryByLabelText("Template")).toBeNull();
  });
});

describe("SlideRail pickers", () => {
  it("opens the new slide gallery from + and closes it on Escape", () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    render(<SlideRail />);
    fireEvent.click(screen.getByTitle("New slide"));
    const dialog = screen.getByRole("dialog", { name: "New slide" });
    expect(within(dialog).getAllByRole("button").map((b) => b.getAttribute("aria-label"))).toEqual([
      "Blank slide",
      "Title layout",
      "Split layout",
      "Quote layout",
    ]);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes a picker on a click outside, and switches between them", () => {
    render(<SlideRail />);
    fireEvent.click(screen.getByTitle("New slide"));
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByTitle(/Pick a style/));
    expect(screen.getByRole("dialog", { name: "Deck style" })).toBeTruthy();
    fireEvent.click(screen.getByTitle("New slide"));
    expect(screen.getByRole("dialog", { name: "New slide" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "Deck style" })).toBeNull();
  });
});

describe("style picker", () => {
  const open = () => fireEvent.click(screen.getByTitle(/Style: |Pick a style/));

  it("names the deck's style on its button", () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    render(<SlideRail />);
    expect(screen.getByTitle(/^Style: Swiss Design\./)).toBeTruthy();
  });
  it("shows the deck's style and marks it in the list", () => {
    useApp.setState({ deck: { ...DECK, template: "swiss" } });
    render(<SlideRail />);
    open();
    const dialog = screen.getByRole("dialog", { name: "Deck style" });
    const swiss = within(dialog).getByRole("button", { name: "Swiss Design style" });
    expect(swiss.getAttribute("aria-current")).toBe("true");
    expect(within(dialog).getByRole("button", { name: "Mine style" }).textContent).toContain("Yours");
    expect(within(dialog).getByText(/restyle every slide/)).toBeTruthy();
  });

  it("puts a restyle prompt in the composer and closes", async () => {
    render(<SlideRail />);
    open();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Bento Grid style" })));
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(useApp.getState().composerFill?.text).toContain('Restyle the whole deck in the "Bento Grid" style'));
  });

  it("styles an empty deck directly and offers no template saving", async () => {
    const empty = { ...DECK, slides: [] };
    useApp.setState({ deck: empty, selected: null });
    invoke.mockImplementation(async (command: string) => (command === "apply_template" ? { ...empty, template: "swiss" } : undefined));
    render(<SlideRail />);
    open();
    expect(screen.getByText(/takes the style's fonts/)).toBeTruthy();
    expect(screen.queryByText(/Save deck as template/)).toBeNull();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Swiss Design style" })));
    expect(invoke).toHaveBeenCalledWith("apply_template", { id: DECK.id, template: "swiss" });
    expect(useApp.getState().deck?.template).toBe("swiss");
  });

  it("disables restyling while the agent is working", () => {
    useApp.setState({ running: true });
    render(<SlideRail />);
    open();
    expect((screen.getByRole("button", { name: "Swiss Design style" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("saves the deck as a template under the name given", async () => {
    const created: TemplateSummary = { id: "board", title: "Board", builtin: false, path: "/home/.slopslides/templates/board", slides: ["intro"] };
    invoke.mockImplementation(async (command: string) => {
      if (command === "create_template") return created;
      if (command === "list_templates") return [created, MINE, SWISS];
    });
    render(<SlideRail />);
    open();
    fireEvent.click(screen.getByRole("button", { name: /Save deck as template/ }));
    const name = screen.getByLabelText("Template name") as HTMLInputElement;
    expect(name.value).toBe(DECK.title);
    fireEvent.change(name, { target: { value: "Board" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
    expect(invoke).toHaveBeenCalledWith("create_template", { id: DECK.id, name: "Board" });
    expect(screen.getByText("Saved “Board”.")).toBeTruthy();
    expect(useApp.getState().templates?.[0]).toEqual(created);
    fireEvent.click(screen.getByTitle("Show the template's folder"));
    expect(reveal).toHaveBeenCalledWith("/home/.slopslides/templates/board/deck.html");
  });

  it("cancels naming a template with Escape, keeping the menu open", () => {
    render(<SlideRail />);
    open();
    fireEvent.click(screen.getByRole("button", { name: /Save deck as template/ }));
    fireEvent.keyDown(screen.getByLabelText("Template name"), { key: "Escape" });
    expect(screen.queryByLabelText("Template name")).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("create_template", expect.anything());
  });
});
