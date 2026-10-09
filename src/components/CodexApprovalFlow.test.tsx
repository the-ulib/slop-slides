import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const listeners = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
  listeners.set(name, handler); return () => listeners.delete(name);
}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));

import type { AgentEvent } from "../lib/api";
import type { ApprovalDecision } from "../lib/permissions";
import type { ProviderInfo } from "../lib/models";
import { initEventBridge, useApp } from "../store";
import { DECK_HTML, deckFor } from "../test/fixtures";
import { emptyNarration, emptyScript } from "../lib/narration";
import { useNarration } from "../narrationStore";
import { ChatPanel } from "./ChatPanel";

const providers: ProviderInfo[] = [{ id: "codex", installed: true, path: "/mock/codex", error: null,
  models: [{ id: "test", label: "Test model", isDefault: true, efforts: ["low"], defaultEffort: "low", contextWindows: [], defaultContextWindow: null }] }];
const approval = { id: "mcp-approval", title: "Use an MCP tool", reason: "Allow lint_deck?",
  details: '{"server":"slopslide"}', acceptLabel: "Allow once", decisions: ["accept", "acceptForSession", "decline"] as ApprovalDecision[] };
const emit = (event: AgentEvent, deckId = "talk") => act(() => listeners.get("agent-event")!({ payload: { deckId, event } }));
const calls = (command: string) => invoke.mock.calls.filter(([name]) => name === command);

beforeEach(async () => {
  listeners.clear();
  invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "list_providers") return providers;
    if (command === "codex_permission_modes") return ["ask", "fullAccess", "custom"];
  });
  useApp.setState({ deck: deckFor(DECK_HTML), selected: "intro", messages: [], running: false,
    permissionMode: "ask", providers, selection: { provider: "codex", model: "test", label: "Test model", effort: "low", contextWindow: null },
    sketches: {}, sketchesSent: {}, favoriteModels: [], error: null });
  await initEventBridge();
  render(<ChatPanel />);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Please lint the deck" } });
  fireEvent.click(screen.getByTitle("Send"));
  await waitFor(() => expect(calls("send_message")).toHaveLength(1));
  expect(calls("send_message")[0]![1].args).toMatchObject({ provider: "codex", permissionMode: "ask" });
});

describe("Codex approval event → store → chat → IPC", () => {
  it.each([
    ["Allow once", "accept"], ["Allow for session", "acceptForSession"], ["Deny", "decline"],
  ] as const)("routes %s and persists a resolved card", async (label, decision) => {
    emit({ type: "approvalRequested", approval });
    fireEvent.click(screen.getByRole("button", { name: label }));
    await waitFor(() => expect(calls("respond_approval")).toHaveLength(1));
    expect(calls("respond_approval")[0]![1]).toEqual({ deckId: "talk", id: "mcp-approval", decision });
    emit({ type: "approvalResolved", id: approval.id });
    emit({ type: "finished", interrupted: false });
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(useApp.getState().running).toBe(false);
    await waitFor(() => expect(calls("save_chat")).toHaveLength(1));
    const saved = calls("save_chat")[0]![1].chat;
    expect(saved.at(-1).parts).toContainEqual({ kind: "approval", approval, status: "resolved" });
  });

  it("drafts narration with Ask permissions and routes read/write approvals before review", async () => {
    act(() => useApp.setState({ running: false, messages: [] }));
    await act(async () => useNarration.getState().load("talk"));
    await act(async () => useApp.getState().draftNarration("slide", "general", "1"));
    expect(calls("send_message").at(-1)![1].args).toMatchObject({ provider: "codex", permissionMode: "ask" });
    expect(calls("send_message").at(-1)![1].args.prompt).toContain("read_narration and write_narration");
    for (const tool of ["read_narration", "write_narration"]) {
      const request = { ...approval, id: tool, reason: `Allow ${tool}?`, details: JSON.stringify({ server: "slopslide", tool }) };
      emit({ type: "approvalRequested", approval: request });
      fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
      await waitFor(() => expect(calls("respond_approval").at(-1)![1]).toEqual({ deckId: "talk", id: tool, decision: "accept" }));
      emit({ type: "approvalResolved", id: tool });
    }
    const document = emptyNarration();
    document.version = "drafted";
    document.manifest.slides.intro = { ...emptyScript(), text: "Welcome to this presentation." };
    invoke.mockImplementation(async (command: string) => command === "load_narration" ? document : undefined);
    emit({ type: "finished", interrupted: false });
    await waitFor(() => expect(screen.getByRole("button", { name: "Review narration" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Review narration" }));
    expect(useApp.getState().sidebarTab).toBe("narration");
    expect(useNarration.getState().document?.manifest.slides.intro?.text).toBe("Welcome to this presentation.");
    await act(async () => useNarration.getState().load(null));
  });

  it("Stop closes the request without approving it", async () => {
    emit({ type: "approvalRequested", approval });
    fireEvent.click(screen.getByText("Stop", { selector: "button" }));
    await waitFor(() => expect(calls("interrupt_agent")).toHaveLength(1));
    expect(calls("respond_approval")).toHaveLength(0);
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    emit({ type: "finished", interrupted: true });
    expect(useApp.getState().running).toBe(false);
  });

  it("ignores another deck's request and closes unanswered requests on failure", () => {
    emit({ type: "approvalRequested", approval }, "another-deck");
    expect(screen.queryByText("Allow lint_deck?")).toBeNull();
    emit({ type: "approvalRequested", approval });
    emit({ type: "error", message: "Codex disconnected" });
    emit({ type: "finished", interrupted: false });
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(screen.getByText("Codex disconnected")).toBeTruthy();
    expect(calls("respond_approval")).toHaveLength(0);
  });
});
