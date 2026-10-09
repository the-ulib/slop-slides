import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));

import type { Deck } from "../lib/api";
import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { SlideRail } from "./SlideRail";

const DECK = deckFor(DECK_HTML);
const withSlides = (...ids: string[]): Deck => ({ ...DECK, slides: ids.map((id) => ({ id, hash: id, hidden: false, locked: false, moved: false })) });

beforeEach(() => {
  invoke.mockReset();
  useApp.setState({ deck: DECK, selected: "intro", error: null, revealRev: 0, templates: [{ id: "swiss", title: "Swiss Design", builtin: true, path: null, slides: ["title"] }] });
});

// dnd-kit gives sortable items role="button", so find them by tag.
const items = () => [...document.querySelectorAll<HTMLElement>("ol > li")];
const item = (index: number) => within(items()[index]!);

const HIDDEN_HTML = DECK_HTML.replace(`id="outro"`, `id="outro" data-hidden`);
const thumbnail = (li: HTMLElement) => li.querySelector("button > div") as HTMLElement;

describe("SlideRail", () => {
  it("numbers every slide and shows the count", () => {
    render(<SlideRail />);
    expect(screen.getByText("Slides · 3")).toBeTruthy();
    expect(items().map((li) => li.textContent)).toEqual(["1", "2", "3"]);
  });

  it("pads the top of the scroll area so the selection ring is not clipped", () => {
    const { container } = render(<SlideRail />);
    const scroller = container.querySelector(".overflow-y-auto");
    expect(scroller?.classList.contains("pt-1.5")).toBe(true);
  });

  it("explains what to do in an empty deck", () => {
    useApp.setState({ deck: { ...DECK, slides: [] }, selected: null });
    render(<SlideRail />);
    expect(screen.getByText("Slides · 0")).toBeTruthy();
    expect(screen.getByText(/No slides yet/)).toBeTruthy();
    expect(items()).toEqual([]);
  });

  it("renders nothing without a deck", () => {
    useApp.setState({ deck: null });
    const { container } = render(<SlideRail />);
    expect(container.innerHTML).toBe("");
  });

  it("selects a slide when its thumbnail is clicked", () => {
    render(<SlideRail />);
    const thumbnail = items()[2]!.querySelector("button")!;
    fireEvent.click(thumbnail);
    expect(useApp.getState().selected).toBe("outro");
    expect(useApp.getState().revealRev).toBe(1);
  });

  it("highlights the selected slide's number", () => {
    render(<SlideRail />);
    expect(item(0).getByText("1").className).toContain("font-semibold");
    expect(item(1).getByText("2").className).not.toContain("font-semibold");
  });

  it("adds a blank slide after the selected one and selects it", async () => {
    invoke.mockResolvedValue({ deck: withSlides("intro", "slide", "#2", "outro"), slide: "slide" });
    render(<SlideRail />);
    await act(async () => fireEvent.click(screen.getByTitle("New slide")));
    await act(async () => fireEvent.click(screen.getByTitle("Add blank slide")));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("add_slide", { id: "talk", after: "intro" });
    expect(useApp.getState().selected).toBe("slide");
    expect(items()).toHaveLength(4);
  });

  it("adds the first slide of an empty deck", async () => {
    useApp.setState({ deck: { ...DECK, slides: [] }, selected: null });
    invoke.mockResolvedValue({ deck: withSlides("slide"), slide: "slide" });
    render(<SlideRail />);
    await act(async () => fireEvent.click(screen.getByTitle("New slide")));
    await act(async () => fireEvent.click(screen.getByTitle("Add blank slide")));
    expect(invoke).toHaveBeenCalledWith("add_slide", { id: "talk", after: null });
    expect(useApp.getState().selected).toBe("slide");
  });

  it("duplicates a slide and selects the copy", async () => {
    invoke.mockResolvedValue({ deck: withSlides("intro", "#2", "outro", "outro-copy"), slide: "outro-copy" });
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Duplicate")));
    expect(invoke).toHaveBeenCalledWith("duplicate_slide", { id: "talk", slide: "outro" });
    expect(useApp.getState().selected).toBe("outro-copy");
  });

  it("deleting the selected slide selects the next one", async () => {
    useApp.setState({ selected: "#2" });
    invoke.mockResolvedValue(withSlides("intro", "outro"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(1).getByTitle("Delete")));
    expect(invoke).toHaveBeenCalledWith("delete_slide", { id: "talk", slide: "#2" });
    expect(useApp.getState().selected).toBe("outro");
    expect(items()).toHaveLength(2);
  });

  it("deleting a slide drops its sketch", async () => {
    const stroke = { tool: "pen" as const, color: "#ef4444", points: [[0.1, 0.2]] as [number, number][] };
    useApp.setState({ sketches: { intro: [stroke], outro: [stroke] } });
    invoke.mockResolvedValue(withSlides("intro", "#2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Delete")));
    expect(useApp.getState().sketches).toEqual({ intro: [stroke] });
  });

  it("keeps the sketch when deleting fails", async () => {
    const stroke = { tool: "pen" as const, color: "#ef4444", points: [[0.1, 0.2]] as [number, number][] };
    useApp.setState({ sketches: { outro: [stroke] } });
    invoke.mockRejectedValue("Slide not found: outro");
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Delete")));
    expect(useApp.getState().sketches).toEqual({ outro: [stroke] });
  });

  it("deleting the last slide selects the one before it", async () => {
    useApp.setState({ selected: "outro" });
    invoke.mockResolvedValue(withSlides("intro", "#2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Delete")));
    expect(useApp.getState().selected).toBe("#2");
  });

  it("deleting another slide keeps the selection", async () => {
    invoke.mockResolvedValue(withSlides("intro", "#2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Delete")));
    expect(useApp.getState().selected).toBe("intro");
  });

  it("slide actions do not also select the slide", async () => {
    invoke.mockResolvedValue(withSlides("intro", "#2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Delete")));
    expect(useApp.getState().revealRev).toBe(0);
  });

  it.each([
    [
      "Add blank slide",
      () => {
        fireEvent.click(screen.getByTitle("New slide"));
        return screen.getByTitle("Add blank slide");
      },
    ],
    ["Duplicate", () => item(0).getByTitle("Duplicate")],
    ["Delete", () => item(0).getByTitle("Delete")],
  ])("reports a failed %s", async (_, button) => {
    invoke.mockRejectedValue("Slide not found: intro");
    render(<SlideRail />);
    const target = button();
    await act(async () => fireEvent.click(target));
    await waitFor(() => expect(useApp.getState().error).toBe("Slide not found: intro"));
    expect(useApp.getState().deck).toEqual(DECK);
  });
});

describe("hidden slides in the rail", () => {
  beforeEach(() => {
    useApp.setState({ deck: deckFor(HIDDEN_HTML) });
  });

  it("mutes hidden slides and strikes them through", () => {
    render(<SlideRail />);
    const [intro, , outro] = items() as [HTMLElement, HTMLElement, HTMLElement];
    expect(thumbnail(outro).className).toMatch(/opacity-35/);
    expect(thumbnail(outro).className).toMatch(/grayscale/);
    expect(within(outro).getByTestId("hidden-mark")).toBeTruthy();
    expect(within(outro).getByText("3").className).toMatch(/line-through/);

    expect(thumbnail(intro).className).not.toMatch(/opacity-35/);
    expect(within(intro).queryByTestId("hidden-mark")).toBeNull();
  });

  it("still renders hidden slides in place, keeping their number", () => {
    render(<SlideRail />);
    expect(items()).toHaveLength(3);
    expect(screen.getByText("Slides · 3")).toBeTruthy();
    expect(items().map((li) => li.querySelector("span")?.textContent)).toEqual(["1", "2", "3"]);
    expect(thumbnail(items()[2]!)).toBeTruthy();
  });

  it("hides a shown slide", async () => {
    const next = deckFor(HIDDEN_HTML.replace(`id="intro"`, `id="intro" data-hidden`), "2");
    invoke.mockResolvedValue(next);
    render(<SlideRail />);
    await act(async () => fireEvent.click(within(items()[0]!).getByTitle("Hide slide")));
    expect(invoke).toHaveBeenCalledWith("set_slide_hidden", { id: "talk", slide: "intro", hidden: true });
    expect(useApp.getState().deck).toBe(next);
    expect(within(items()[0]!).getByTestId("hidden-mark")).toBeTruthy();
  });

  it("shows a hidden slide again", async () => {
    invoke.mockResolvedValue(deckFor(DECK_HTML, "2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(within(items()[2]!).getByTitle("Show slide")));
    expect(invoke).toHaveBeenCalledWith("set_slide_hidden", { id: "talk", slide: "outro", hidden: false });
    expect(screen.queryByTestId("hidden-mark")).toBeNull();
  });

  it("reports a failed toggle", async () => {
    invoke.mockRejectedValue("disk full");
    render(<SlideRail />);
    await act(async () => fireEvent.click(within(items()[0]!).getByTitle("Hide slide")));
    expect(useApp.getState().error).toBe("disk full");
    expect(useApp.getState().deck?.slides[0]?.hidden).toBe(false);
  });
});

const LOCKED_HTML = DECK_HTML.replace(`id="outro"`, `id="outro" data-locked`);

describe("locked slides in the rail", () => {
  beforeEach(() => {
    useApp.setState({ deck: deckFor(LOCKED_HTML) });
  });

  it("marks locked slides and offers to unlock them", () => {
    render(<SlideRail />);
    expect(item(2).getByTestId("locked-mark")).toBeTruthy();
    expect(item(2).getByTitle("Unlock slide")).toBeTruthy();
    expect(item(0).queryByTestId("locked-mark")).toBeNull();
    expect(item(0).getByTitle("Lock slide")).toBeTruthy();
  });

  it("cannot delete a locked slide, but can still duplicate and hide it", () => {
    render(<SlideRail />);
    expect(item(2).queryByTitle("Delete")).toBeNull();
    expect(item(2).getByTitle("Duplicate")).toBeTruthy();
    expect(item(2).getByTitle("Hide slide")).toBeTruthy();
    expect(item(0).getByTitle("Delete")).toBeTruthy();
  });

  it("locks a slide", async () => {
    const next = deckFor(LOCKED_HTML.replace(`id="intro"`, `id="intro" data-locked`), "2");
    invoke.mockResolvedValue(next);
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(0).getByTitle("Lock slide")));
    expect(invoke).toHaveBeenCalledWith("set_slide_locked", { id: "talk", slide: "intro", locked: true });
    expect(useApp.getState().deck).toBe(next);
    expect(item(0).getByTestId("locked-mark")).toBeTruthy();
    expect(item(0).queryByTitle("Delete")).toBeNull();
  });

  it("unlocks a slide", async () => {
    invoke.mockResolvedValue(deckFor(DECK_HTML, "2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(2).getByTitle("Unlock slide")));
    expect(invoke).toHaveBeenCalledWith("set_slide_locked", { id: "talk", slide: "outro", locked: false });
    expect(screen.queryByTestId("locked-mark")).toBeNull();
    expect(item(2).getByTitle("Delete")).toBeTruthy();
  });

  it("reports a failed lock", async () => {
    invoke.mockRejectedValue("disk full");
    render(<SlideRail />);
    await act(async () => fireEvent.click(item(0).getByTitle("Lock slide")));
    expect(useApp.getState().error).toBe("disk full");
  });
});

const SECTIONED_HTML = DECK_HTML.replace(
  `<section class="slide">`,
  `<div class="deck-section" data-title="Middle"></div>\n<section class="slide">`,
);
const headers = () => [...document.querySelectorAll<HTMLElement>("[data-testid=section-header]")];

describe("sections in the rail", () => {
  beforeEach(() => {
    useApp.setState({ deck: deckFor(SECTIONED_HTML) });
  });

  it("shows a header row before the slide that starts the section, without numbering it", () => {
    render(<SlideRail />);
    expect(items().map((li) => (li.dataset.testid === "section-header" ? li.textContent : li.querySelector("span")?.textContent))).toEqual([
      "1",
      "Middle",
      "2",
      "3",
    ]);
    expect(screen.getByText("Slides · 3")).toBeTruthy();
  });

  it("shows a placeholder for an untitled section", () => {
    useApp.setState({ deck: { ...deckFor(SECTIONED_HTML), sections: [{ index: 0, title: "", before: 1 }] } });
    render(<SlideRail />);
    expect(screen.getByText("Untitled section")).toBeTruthy();
  });

  it("starts a section at the selected slide and lets you name it", async () => {
    useApp.setState({ deck: DECK, selected: "outro" });
    invoke.mockResolvedValue(deckFor(DECK_HTML.replace(`<section class="slide" id="outro">`, `<div class="deck-section" data-title="New section"></div>\n<section class="slide" id="outro">`)));
    render(<SlideRail />);
    await act(async () => fireEvent.click(screen.getByTitle("Start a section at this slide")));
    expect(invoke).toHaveBeenCalledWith("add_section", { id: "talk", before: "outro", title: "New section" });
    expect(headers()).toHaveLength(1);
    const input = screen.getByLabelText("Section title") as HTMLInputElement;
    expect(input.value).toBe("New section");
  });

  it("cannot start a second section at a slide that already starts one", () => {
    useApp.setState({ selected: "#2" });
    render(<SlideRail />);
    const button = screen.getByTitle("This slide already starts a section") as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("cannot add a section without a selected slide", () => {
    useApp.setState({ deck: { ...DECK, slides: [] }, selected: null });
    render(<SlideRail />);
    expect((screen.getByTitle("Start a section at this slide") as HTMLButtonElement).disabled).toBe(true);
  });

  it("renames a section by double-clicking its title", async () => {
    const renamed = deckFor(SECTIONED_HTML.replace("Middle", "Core"), "2");
    invoke.mockResolvedValue(renamed);
    render(<SlideRail />);
    fireEvent.doubleClick(screen.getByText("Middle"));
    const input = screen.getByLabelText("Section title") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  Core  " } });
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    await act(async () => fireEvent.blur(input));
    expect(invoke).toHaveBeenCalledWith("rename_section", { id: "talk", index: 0, title: "Core" });
    expect(useApp.getState().deck).toBe(renamed);
    expect(screen.queryByLabelText("Section title")).toBeNull();
  });

  it("renames through the hover action and cancels with Escape", async () => {
    render(<SlideRail />);
    fireEvent.click(within(headers()[0]!).getByTitle("Rename section"));
    const input = screen.getByLabelText("Section title") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Changed" } });
    await act(async () => fireEvent.keyDown(input, { key: "Escape" }));
    await act(async () => fireEvent.blur(input));
    expect(invoke).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Section title")).toBeNull();
    expect(screen.getByText("Middle")).toBeTruthy();
  });

  it("does not save an empty or unchanged title", async () => {
    render(<SlideRail />);
    for (const value of ["", "   ", "Middle"]) {
      fireEvent.doubleClick(screen.getByText("Middle"));
      const input = screen.getByLabelText("Section title");
      fireEvent.change(input, { target: { value } });
      await act(async () => fireEvent.blur(input));
    }
    expect(invoke).not.toHaveBeenCalled();
  });

  it("removes a section but keeps its slides", async () => {
    invoke.mockResolvedValue(deckFor(DECK_HTML, "2"));
    render(<SlideRail />);
    await act(async () => fireEvent.click(within(headers()[0]!).getByTitle("Remove section")));
    expect(invoke).toHaveBeenCalledWith("delete_section", { id: "talk", index: 0 });
    expect(headers()).toHaveLength(0);
    expect(items()).toHaveLength(3);
  });

  it.each([
    ["Rename section", "rename_section"],
    ["Remove section", "delete_section"],
  ])("reports a failed %s", async (title, command) => {
    invoke.mockRejectedValue("Section not found: 0");
    render(<SlideRail />);
    if (command === "rename_section") {
      fireEvent.click(within(headers()[0]!).getByTitle(title));
      const input = screen.getByLabelText("Section title");
      fireEvent.change(input, { target: { value: "X" } });
      await act(async () => fireEvent.blur(input));
    } else {
      await act(async () => fireEvent.click(within(headers()[0]!).getByTitle(title)));
    }
    await waitFor(() => expect(useApp.getState().error).toBe("Section not found: 0"));
  });

  it("reports a failed add", async () => {
    invoke.mockRejectedValue("Slide not found: intro");
    render(<SlideRail />);
    await act(async () => fireEvent.click(screen.getByTitle("Start a section at this slide")));
    await waitFor(() => expect(useApp.getState().error).toBe("Slide not found: intro"));
    expect(screen.queryByLabelText("Section title")).toBeNull();
  });
});
