import { ask } from "@tauri-apps/plugin-dialog";
import { ChevronDown, Shield } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { PERMISSION_MODES, type ApprovalDecision, type PermissionMode } from "../lib/permissions";
import { useApp, type ChatPart } from "../store";

export function PermissionPicker() {
  const provider = useApp((s) => s.selection.provider);
  const deckId = useApp((s) => s.deck?.id);
  const running = useApp((s) => s.running);
  const mode = useApp((s) => s.permissionMode);
  const [open, setOpen] = useState(false);
  const [modes, setModes] = useState<PermissionMode[]>([]);
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (provider !== "codex" || !deckId) return;
    let active = true;
    setModes([]);
    setError(null);
    void api.codexPermissionModes(deckId).then((next) => { if (active) setModes(next); }).catch((e) => { if (active) setError(errorMessage(e)); });
    return () => { active = false; };
  }, [provider, deckId]);
  useEffect(() => {
    if (!open) return;
    const dismiss = (e: PointerEvent) => { if (!root.current?.contains(e.target as Node)) setOpen(false); };
    const escape = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", dismiss); document.removeEventListener("keydown", escape); };
  }, [open]);
  useEffect(() => { if (running || provider !== "codex") setOpen(false); }, [running, provider]);
  if (provider !== "codex") return null;
  const choose = async (next: PermissionMode) => {
    try {
      if (next === "fullAccess" && mode !== next && !(await ask(
        "Codex will be able to change files and run commands beyond the deck workspace without asking for approval.",
        { title: "Enable Full access?", kind: "warning", okLabel: "Enable Full access", cancelLabel: "Cancel" },
      ))) return;
    } catch (e) { setError(errorMessage(e)); return; }
    // The turn may have started while the confirmation was open; the store checks again.
    useApp.getState().setPermissionMode(next);
    setOpen(false);
  };
  return <div ref={root} className="min-w-0">
    <button type="button" disabled={running} aria-label="Codex permissions" aria-expanded={open} onClick={() => setOpen(!open)}
      className="flex h-7 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50">
      <Shield className="size-3.5 shrink-0" /><span>{PERMISSION_MODES[mode].label}</span><ChevronDown className="size-3" />
    </button>
    {open && <div role="group" aria-label="Permission modes" className="absolute bottom-full left-2 z-50 mb-2 w-64 max-w-[calc(100vw-2rem)] rounded-xl border bg-card p-1.5 text-foreground shadow-lg">
      {error ? <p role="alert" className="p-2 text-xs text-destructive">{error}</p> : modes.length === 0 ? <p className="p-2 text-xs text-muted-foreground">Checking Codex permissions…</p> : modes.map((m) => <button key={m} type="button" aria-pressed={mode === m} onClick={() => void choose(m)} className="block w-full rounded-lg px-2.5 py-2 text-left text-xs hover:bg-accent aria-pressed:bg-accent">
        <span className="font-medium">{PERMISSION_MODES[m].label}</span><span className="mt-0.5 block text-muted-foreground">{PERMISSION_MODES[m].description}</span>
      </button>)}
      {modes.length > 0 && !modes.includes(mode) && <p role="alert" className="p-2 text-xs text-destructive">The saved mode is unavailable. Select another mode.</p>}
    </div>}
  </div>;
}

export function ApprovalCard({ part }: { part: Extract<ChatPart, { kind: "approval" }> }) {
  const deckId = useApp((s) => s.deck?.id);
  const running = useApp((s) => s.running);
  const [submitted, setSubmitted] = useState<ApprovalDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);
  const active = part.status === "pending" && running && !submitted;
  const respond = async (decision: ApprovalDecision) => {
    if (!deckId || !active || sending.current) return;
    sending.current = true;
    setSubmitted(decision);
    setError(null);
    try { await api.respondApproval(deckId, part.approval.id, decision); }
    catch (e) { setError(errorMessage(e)); setSubmitted(null); }
    finally { sending.current = false; }
  };
  const labels: Record<ApprovalDecision, string> = { accept: part.approval.acceptLabel, acceptForSession: "Allow for session", decline: "Deny" };
  return <div className="rounded-lg border p-3 text-xs" aria-label="Codex approval request">
    <p className="flex items-center gap-1.5 font-medium"><Shield className="size-3.5" />{part.approval.title}</p>
    {part.approval.reason && <p className="mt-2 whitespace-pre-wrap text-muted-foreground">{part.approval.reason}</p>}
    <pre className="selectable my-2 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted p-2 font-mono text-xs">{part.approval.details}</pre>
    {active ? <div className="flex flex-wrap gap-2">
      {part.approval.decisions.map((d) => <button key={d} type="button" onClick={() => void respond(d)} className="rounded-md border px-2.5 py-1.5 hover:bg-accent">{labels[d]}</button>)}
      <button type="button" onClick={() => useApp.getState().interrupt()} className="rounded-md border px-2.5 py-1.5 hover:bg-accent">Stop</button>
    </div> : <p aria-live="polite" className="text-muted-foreground">{part.status === "expired" || !running && !submitted ? "Request closed" : submitted ? `${labels[submitted]} · response sent` : "Request resolved"}</p>}
    {error && <p role="alert" className="mt-2 text-destructive">{error}</p>}
  </div>;
}

export function ApprovalReview({ part }: { part: Extract<ChatPart, { kind: "approvalReview" }> }) {
  const status = ({ inProgress: "Reviewing", reviewing: "Reviewing", approved: "Approved", denied: "Denied", aborted: "Aborted", timedOut: "Timed out" } as Record<string, string>)[part.status] ?? part.status;
  return <div className="rounded-lg border p-2.5 text-xs" aria-live="polite">
    <p className="font-medium">Automatic review · {status}</p>
    {part.detail && <p className="mt-1 text-muted-foreground">{part.detail}</p>}
  </div>;
}
