import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
const invoke = vi.fn();
const ask = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: (...args: unknown[]) => ask(...args) }));
import { useApp, type ChatPart } from "../store";
import { deckFor, DECK_HTML } from "../test/fixtures";
import { ApprovalCard, ApprovalReview, PermissionPicker } from "./Permissions";

const interrupt = vi.fn();
beforeEach(() => {
  invoke.mockReset().mockResolvedValue(["ask", "autoReview", "fullAccess", "custom"]);
  ask.mockReset().mockResolvedValue(true);
  interrupt.mockReset();
  useApp.setState({ deck: deckFor(DECK_HTML), selection: { provider: "codex", model: "m", label: "M", effort: "high", contextWindow: null }, permissionMode: "ask", running: false, interrupt });
});
const picker = () => screen.getByRole("button", { name: "Codex permissions" });
const openPicker = async () => { fireEvent.click(picker()); await screen.findByRole("button", { name: /Approve for me/ }); };
const part = (): Extract<ChatPart, { kind: "approval" }> => ({ kind: "approval", status: "pending", approval: { id: "r1", title: "Run a command", reason: "Read reference", details: "cat /reference", acceptLabel: "Allow once", decisions: ["accept", "decline"] } });

describe("permission picker", () => {
  it("appears only for Codex and applies the chosen mode", async () => {
    render(<PermissionPicker />);
    await openPicker();
    fireEvent.click(screen.getByRole("button", { name: /Approve for me/ }));
    expect(useApp.getState().permissionMode).toBe("autoReview");
    expect(picker().textContent).toContain("Approve for me");
    expect(invoke).toHaveBeenCalledWith("codex_permission_modes", { id: "talk" });
    act(() => useApp.setState({ selection: { ...useApp.getState().selection, provider: "claude" } }));
    expect(screen.queryByRole("button", { name: "Codex permissions" })).toBeNull();
  });

  it("confirms Full access and keeps the old mode when cancelled", async () => {
    ask.mockResolvedValueOnce(false);
    render(<PermissionPicker />); await openPicker();
    fireEvent.click(screen.getByRole("button", { name: /Full access/ }));
    await waitFor(() => expect(ask).toHaveBeenCalled());
    expect(useApp.getState().permissionMode).toBe("ask");
    fireEvent.click(screen.getByRole("button", { name: /Full access/ }));
    await waitFor(() => expect(useApp.getState().permissionMode).toBe("fullAccess"));
  });

  it("locks during turns and cannot apply a delayed confirmation", async () => {
    let resolve!: (value: boolean) => void;
    ask.mockReturnValue(new Promise<boolean>((r) => { resolve = r; }));
    render(<PermissionPicker />); await openPicker();
    fireEvent.click(screen.getByRole("button", { name: /Full access/ }));
    act(() => useApp.setState({ running: true }));
    await act(async () => resolve(true));
    expect((picker() as HTMLButtonElement).disabled).toBe(true);
    expect(useApp.getState().permissionMode).toBe("ask");
  });

  it("keeps the old mode if the confirmation dialog fails", async () => {
    ask.mockRejectedValueOnce("Dialog unavailable");
    render(<PermissionPicker />); await openPicker();
    fireEvent.click(screen.getByRole("button", { name: /Full access/ }));
    expect((await screen.findByRole("alert")).textContent).toBe("Dialog unavailable");
    expect(useApp.getState().permissionMode).toBe("ask");
  });

  it("shows only modes returned by Codex and reports an unavailable saved mode", async () => {
    invoke.mockResolvedValue(["ask", "custom"]);
    useApp.setState({ permissionMode: "autoReview" });
    render(<PermissionPicker />); fireEvent.click(picker());
    await screen.findByRole("button", { name: /Use your existing/ });
    expect(screen.queryByRole("button", { name: /Full access/ })).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("saved mode is unavailable");
  });

  it("reports capability errors without inventing available modes", async () => {
    invoke.mockRejectedValue("CLI unavailable");
    render(<PermissionPicker />); fireEvent.click(picker());
    expect((await screen.findByRole("alert")).textContent).toBe("CLI unavailable");
    expect(screen.queryByRole("button", { name: /Full access/ })).toBeNull();
  });

  it("dismisses on Escape", async () => {
    render(<PermissionPicker />); await openPicker();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("group", { name: "Permission modes" })).toBeNull();
  });
});

describe("approval card", () => {
  beforeEach(() => { useApp.setState({ running: true }); invoke.mockResolvedValue(undefined); });
  it.each(["Allow once", "Deny"])("sends %s with the exact deck and request ID, once", async (label) => {
    render(<ApprovalCard part={part()} />);
    const button = screen.getByRole("button", { name: label });
    fireEvent.click(button); fireEvent.click(button);
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith("respond_approval", { deckId: "talk", id: "r1", decision: label === "Deny" ? "decline" : "accept" });
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Allow for session" })).toBeNull();
  });
  it("offers session approval only when supplied by Codex", async () => {
    const p = part(); p.approval.decisions.push("acceptForSession");
    render(<ApprovalCard part={p} />); fireEvent.click(screen.getByRole("button", { name: "Allow for session" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("respond_approval", { deckId: "talk", id: "r1", decision: "acceptForSession" }));
  });
  it("shows an MCP tool confirmation and sends the user's decision through the existing card", async () => {
    const p = part();
    p.approval.title = "Use an MCP tool";
    p.approval.reason = "Allow lint_deck?\nThis checks the presentation.";
    p.approval.details = '{"server":"slopslide","tool_params":{}}';
    render(<ApprovalCard part={p} />);
    expect(screen.getByText("Use an MCP tool")).toBeTruthy();
    expect(screen.getByText(/This checks the presentation/).textContent).toContain("\n");
    expect(screen.getByText(/tool_params/).textContent).toContain("slopslide");
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("respond_approval", { deckId: "talk", id: "r1", decision: "accept" }));
    expect(screen.queryByRole("button", { name: "Allow for session" })).toBeNull();
  });
  it("stops the turn without sending an approval", () => {
    render(<ApprovalCard part={part()} />); fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(interrupt).toHaveBeenCalledOnce(); expect(invoke).not.toHaveBeenCalled();
  });
  it("reports a failed response and allows a retry", async () => {
    invoke.mockRejectedValueOnce("No longer pending");
    render(<ApprovalCard part={part()} />); fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    expect((await screen.findByRole("alert")).textContent).toBe("No longer pending");
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  });
  it.each(["expired", "resolved"] as const)("has no action buttons for a %s request", (status) => {
    render(<ApprovalCard part={{ ...part(), status }} />); expect(screen.queryByRole("button")).toBeNull();
  });
  it("does not offer approval when the agent has finished", () => {
    useApp.setState({ running: false }); render(<ApprovalCard part={part()} />);
    expect(screen.queryByRole("button")).toBeNull();
  });
  it("displays the automatic reviewer result and reason", () => {
    render(<ApprovalReview part={{ kind: "approvalReview", id: "r", status: "denied", detail: "Outside scope" }} />);
    expect(screen.getByText("Automatic review · Denied")).toBeTruthy(); expect(screen.getByText("Outside scope")).toBeTruthy();
  });
});
