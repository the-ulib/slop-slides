import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const save = vi.fn();
const openDialog = vi.fn();
const revealItemInDir = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: vi.fn(),
  save: (...args: unknown[]) => save(...args),
  open: (...args: unknown[]) => openDialog(...args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: (...args: unknown[]) => revealItemInDir(...args) }));

import { useVideo } from "../videoStore";
import { useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { TopBar } from "./TopBar";

const toggle = (name: "Slides" | "HTML") => screen.getByRole("button", { name }) as HTMLButtonElement;

beforeEach(() => {
  invoke.mockReset();
  save.mockReset();
  openDialog.mockReset();
  revealItemInDir.mockReset();
  useApp.setState({ deck: deckFor(DECK_HTML), view: "slides", chatOpen: true, sidebarTab: "chat", codeDirty: false, presenting: false, error: null, imageExport: null });
});

describe("sidebar navigation", () => {
  it("hides and shows the sidebar without changing its selected tab", () => {
    useApp.setState({ sidebarTab: "narration" });
    render(<TopBar />);
    const hide = screen.getByRole("button", { name: "Hide sidebar" });
    expect(hide.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(hide);
    expect(useApp.getState().chatOpen).toBe(false);
    const show = screen.getByRole("button", { name: "Show sidebar" });
    expect(show.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(show);
    expect(useApp.getState().chatOpen).toBe(true);
    expect(useApp.getState().sidebarTab).toBe("narration");
  });

  it.each([false, true])("opens narration directly when sidebar visibility is %s", (chatOpen) => {
    useApp.setState({ chatOpen, sidebarTab: "chat" });
    render(<TopBar />);
    const button = screen.getByRole("button", { name: "Open narration" });
    expect(button.textContent).toBe("Narration");
    expect(button.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(button);
    expect(useApp.getState().chatOpen).toBe(true);
    expect(useApp.getState().sidebarTab).toBe("narration");
    expect(localStorage.getItem("slopslide.sidebarTab")).toBe("narration");
    expect(localStorage.getItem("slopslide.chatOpen")).toBe("true");
    expect(button.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(button);
    expect(useApp.getState().chatOpen).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("Slides / HTML toggle", () => {
  it("marks the current view as pressed", () => {
    render(<TopBar />);
    expect(toggle("Slides").getAttribute("aria-pressed")).toBe("true");
    expect(toggle("HTML").getAttribute("aria-pressed")).toBe("false");
  });

  it("switches to the HTML view and back", () => {
    render(<TopBar />);
    fireEvent.click(toggle("HTML"));
    expect(useApp.getState().view).toBe("code");
    expect(toggle("HTML").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(toggle("Slides"));
    expect(useApp.getState().view).toBe("slides");
    expect(localStorage.getItem("slopslide.view")).toBe("slides");
  });

  it("shows an unsaved-changes dot on the HTML button", () => {
    render(<TopBar />);
    expect(screen.queryByTitle("Unsaved changes")).toBeNull();
    act(() => useApp.setState({ codeDirty: true }));
    const dot = screen.getByTitle("Unsaved changes");
    expect(toggle("HTML").contains(dot)).toBe(true);
  });

  it("explains each view in its tooltip", () => {
    render(<TopBar />);
    expect(toggle("HTML").title).toMatch(/deck\.html/);
    expect(toggle("Slides").title).toMatch(/rendered slide/);
  });
});

describe("deck title", () => {
  const titleInput = () => document.querySelector("header input") as HTMLInputElement;

  it("shows the deck title", () => {
    render(<TopBar />);
    expect(titleInput().value).toBe("Talk");
  });

  it("renames on Enter", async () => {
    invoke.mockResolvedValue({ ...deckFor(DECK_HTML), title: "Board meeting" });
    render(<TopBar />);
    fireEvent.change(titleInput(), { target: { value: "  Board meeting " } });
    titleInput().focus();
    await act(async () => fireEvent.keyDown(titleInput(), { key: "Enter" }));
    expect(invoke).toHaveBeenCalledWith("rename_deck", { id: "talk", title: "Board meeting" });
    expect(useApp.getState().deck!.title).toBe("Board meeting");
    expect(titleInput().value).toBe("Board meeting");
  });

  it("renames when focus leaves the field", async () => {
    invoke.mockResolvedValue({ ...deckFor(DECK_HTML), title: "New" });
    render(<TopBar />);
    fireEvent.change(titleInput(), { target: { value: "New" } });
    await act(async () => fireEvent.blur(titleInput()));
    expect(invoke).toHaveBeenCalledWith("rename_deck", { id: "talk", title: "New" });
  });

  it("Escape restores the saved title without renaming", async () => {
    render(<TopBar />);
    fireEvent.change(titleInput(), { target: { value: "Oops" } });
    titleInput().focus();
    // Escape blurs the field, and blurring normally commits the edit.
    await act(async () => fireEvent.keyDown(titleInput(), { key: "Escape" }));
    expect(titleInput().value).toBe("Talk");
    expect(document.activeElement).not.toBe(titleInput());
    expect(invoke).not.toHaveBeenCalled();
  });

  it("still renames normally after an Escape", async () => {
    invoke.mockResolvedValue({ ...deckFor(DECK_HTML), title: "Second try" });
    render(<TopBar />);
    titleInput().focus();
    await act(async () => fireEvent.keyDown(titleInput(), { key: "Escape" }));
    fireEvent.change(titleInput(), { target: { value: "Second try" } });
    await act(async () => fireEvent.blur(titleInput()));
    expect(invoke).toHaveBeenCalledWith("rename_deck", { id: "talk", title: "Second try" });
  });

  it.each([[""], ["   "], ["Talk"], [" Talk "]])("does not rename to %j", async (value) => {
    render(<TopBar />);
    fireEvent.change(titleInput(), { target: { value } });
    await act(async () => fireEvent.blur(titleInput()));
    expect(invoke).not.toHaveBeenCalled();
    expect(titleInput().value).toBe("Talk");
  });

  it("reports a failed rename", async () => {
    invoke.mockRejectedValue("deck not found: talk");
    render(<TopBar />);
    fireEvent.change(titleInput(), { target: { value: "New" } });
    await act(async () => fireEvent.blur(titleInput()));
    expect(useApp.getState().error).toBe("deck not found: talk");
  });

  it("picks up title changes made elsewhere (e.g. by the agent)", () => {
    render(<TopBar />);
    act(() => useApp.getState().setDeck({ ...deckFor(DECK_HTML), title: "Agent's title" }));
    expect(titleInput().value).toBe("Agent's title");
  });
});

describe("toolbar actions", () => {
  const exportButton = () => screen.getByRole("button", { name: /Export/ }) as HTMLButtonElement;
  const presentButton = () => screen.getByRole("button", { name: /Present/ }) as HTMLButtonElement;
  async function exportAs(item: "HTML file" | "PNG images") {
    fireEvent.click(exportButton());
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: new RegExp(item) })));
  }

  it("exports to the chosen file and reveals it", async () => {
    save.mockResolvedValue("/Users/me/Desktop/Talk.html");
    render(<TopBar />);
    await exportAs("HTML file");
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "Talk.html" }));
    expect(invoke).toHaveBeenCalledWith("export_deck", { id: "talk", dest: "/Users/me/Desktop/Talk.html" });
    expect(revealItemInDir).toHaveBeenCalledWith("/Users/me/Desktop/Talk.html");
  });

  it.each([
    [`Q3: "Plan" / <2025>?`, "Q3 Plan  2025.html"],
    [`a\\b|c*d`, "abcd.html"],
    [`///`, "presentation.html"],
  ])("suggests a file name safe on every OS for %j", async (title, expected) => {
    useApp.setState({ deck: { ...deckFor(DECK_HTML), title } });
    save.mockResolvedValue(null);
    render(<TopBar />);
    await exportAs("HTML file");
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: expected }));
  });

  it("does nothing when the save dialog is cancelled", async () => {
    save.mockResolvedValue(null);
    render(<TopBar />);
    await exportAs("HTML file");
    expect(invoke).not.toHaveBeenCalled();
    expect(revealItemInDir).not.toHaveBeenCalled();
  });

  it("reports a failed export", async () => {
    save.mockResolvedValue("/read-only/Talk.html");
    invoke.mockRejectedValue("Permission denied (os error 13)");
    render(<TopBar />);
    await exportAs("HTML file");
    expect(useApp.getState().error).toBe("Permission denied (os error 13)");
    expect(revealItemInDir).not.toHaveBeenCalled();
  });

  it("opens and closes the export menu", () => {
    render(<TopBar />);
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(exportButton());
    expect(exportButton().getAttribute("aria-expanded")).toBe("true");
    expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
      "HTML fileOne self-contained file to share",
      "Narrated MP41080p video with saved narration",
      "PNG imagesOne image per slide, in a new folder",
    ]);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(exportButton());
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("prepares a narrated video from the export menu", async () => {
    const start = vi.spyOn(useVideo.getState(), "open").mockResolvedValue();
    render(<TopBar />); fireEvent.click(exportButton());
    fireEvent.click(screen.getByRole("menuitem", { name: /Narrated MP4/ }));
    expect(start).toHaveBeenCalledWith("talk", true); start.mockRestore();
  });

  it("exports PNGs into a new folder named after the deck", async () => {
    openDialog.mockResolvedValue("/Users/me/Desktop");
    invoke.mockImplementation(async (command: string) =>
      command === "create_image_export_dir" ? "/Users/me/Desktop/Talk" : undefined,
    );
    render(<TopBar />);
    await exportAs("PNG images");
    expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ directory: true }));
    expect(invoke).toHaveBeenCalledWith("create_image_export_dir", { id: "talk", parent: "/Users/me/Desktop" });
    expect(useApp.getState().imageExport).toEqual({ dir: "/Users/me/Desktop/Talk", slides: ["intro", "#2", "outro"] });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("does nothing when no folder is chosen", async () => {
    openDialog.mockResolvedValue(null);
    render(<TopBar />);
    await exportAs("PNG images");
    expect(invoke).not.toHaveBeenCalled();
    expect(useApp.getState().imageExport).toBeNull();
  });

  it("reports a folder that cannot be created", async () => {
    openDialog.mockResolvedValue("/read-only");
    invoke.mockRejectedValue("Permission denied (os error 13)");
    render(<TopBar />);
    await exportAs("PNG images");
    expect(useApp.getState().error).toBe("Permission denied (os error 13)");
    expect(useApp.getState().imageExport).toBeNull();
  });

  it("starts the presentation", () => {
    render(<TopBar />);
    fireEvent.click(presentButton());
    expect(useApp.getState().presenting).toBe(true);
  });

  it("cannot export or present an empty deck", () => {
    useApp.setState({ deck: { ...deckFor(DECK_HTML), slides: [] } });
    render(<TopBar />);
    expect(exportButton().disabled).toBe(true);
    expect(presentButton().disabled).toBe(true);
  });

  it("reveals deck.html in the file manager", () => {
    render(<TopBar />);
    fireEvent.click(screen.getByTitle("Show deck folder"));
    expect(revealItemInDir).toHaveBeenCalledWith("/decks/talk/deck.html");
  });

  it("goes back to the deck library", async () => {
    invoke.mockResolvedValue(undefined);
    render(<TopBar />);
    await act(async () => fireEvent.click(screen.getByTitle("All decks")));
    expect(invoke).toHaveBeenCalledWith("close_deck");
    expect(useApp.getState().deck).toBeNull();
  });

  it("renders nothing without a deck", () => {
    useApp.setState({ deck: null });
    const { container } = render(<TopBar />);
    expect(container.innerHTML).toBe("");
  });
});

describe("lint status", () => {
  const issues = [
    { rule: "unclosed-tag", severity: "error" as const, message: "<div> is never closed.", line: 12, slide: "intro" },
    { rule: "title", severity: "warning" as const, message: "Needs a title.", line: 1, slide: null },
  ];

  it("shows a pending state until the first check finishes", () => {
    useApp.setState({ lint: null });
    render(<TopBar />);
    expect(screen.getByTitle(/Checking deck\.html/)).toBeTruthy();
  });

  it("shows OK and re-checks on click", () => {
    const refreshLint = vi.fn(async () => {});
    useApp.setState({ lint: [], refreshLint });
    render(<TopBar />);
    fireEvent.click(screen.getByRole("button", { name: /Lint OK/ }));
    expect(refreshLint).toHaveBeenCalled();
    expect(useApp.getState().composerFill).toBeNull();
  });

  it("counts errors and warnings and lists them in the tooltip", () => {
    useApp.setState({ lint: issues });
    render(<TopBar />);
    const button = screen.getByRole("button", { name: /Lint: 1 error, 1 warning/ });
    expect(button.title).toContain("Line 12: <div> is never closed.");
    expect(button.className).toContain("text-destructive");
  });

  it("uses the warning style when there are only warnings", () => {
    useApp.setState({ lint: [issues[1]!, issues[1]!] });
    render(<TopBar />);
    expect(screen.getByRole("button", { name: /Lint: 2 warnings/ }).className).toContain("text-amber-600");
  });

  it("fills the chat composer with fix instructions when clicked", () => {
    useApp.setState({ lint: issues, composerFill: null, running: false });
    render(<TopBar />);
    fireEvent.click(screen.getByRole("button", { name: /Lint: 1 error/ }));
    const fill = useApp.getState().composerFill!;
    expect(fill.text).toContain("[unclosed-tag] (slide `intro`)");
    expect(fill.text).toContain("lint_deck");
  });

  it("is disabled while the agent works", () => {
    useApp.setState({ lint: issues, running: true });
    render(<TopBar />);
    expect((screen.getByRole("button", { name: /Lint: 1 error/ }) as HTMLButtonElement).disabled).toBe(true);
    useApp.setState({ running: false });
  });
});
