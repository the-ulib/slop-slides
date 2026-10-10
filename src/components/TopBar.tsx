import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  AudioLines,
  CheckCircle2,
  ChevronDown,
  ChevronLeft,
  Code2,
  FileCode2,
  FolderOpen,
  Images,
  Loader2,
  PanelRightClose,
  PanelRightOpen,
  Play,
  Presentation,
  Share,
  XCircle,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useVideo } from "../videoStore";
import { api, errorMessage } from "../lib/api";
import { cn, isMac } from "../lib/utils";
import { lintFixPrompt, useApp, type StageView } from "../store";

export function TopBar() {
  const deck = useApp((s) => s.deck);
  const [title, setTitle] = useState(deck?.title ?? "");
  // Escape blurs the field, and blur commits; this keeps that blur from saving the edit.
  const cancelled = useRef(false);

  useEffect(() => setTitle(deck?.title ?? ""), [deck?.title]);
  if (!deck) return null;

  const commitTitle = async () => {
    if (cancelled.current) {
      cancelled.current = false;
      return;
    }
    const next = title.trim();
    if (!next || next === deck.title) return setTitle(deck.title);
    try {
      useApp.getState().setDeck(await api.renameDeck(deck.id, next));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const exportDeck = async () => {
    const name = deck.title.replace(/[\\/:*?"<>|]+/g, "").trim() || "presentation";
    const dest = await save({
      title: "Export presentation",
      defaultPath: `${name}.html`,
      filters: [{ name: "HTML presentation", extensions: ["html"] }],
    });
    if (!dest) return;
    try {
      await api.exportDeck(deck.id, dest);
      await revealItemInDir(dest);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const exportImages = async () => {
    const parent = await open({ directory: true, title: "Choose where to save the slide images" });
    if (!parent || Array.isArray(parent)) return;
    try {
      useApp.getState().startImageExport(await api.createImageExportDir(deck.id, parent));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  return (
    <header
      data-tauri-drag-region
      className={cn(
        "flex h-12 shrink-0 items-center gap-2 border-b bg-background pr-3",
        isMac ? "pl-[84px]" : "pl-3",
      )}
    >
      <button
        type="button"
        onClick={() => void useApp.getState().closeDeck()}
        title="All decks"
        className="flex items-center gap-1 rounded-md px-1.5 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <ChevronLeft className="size-4" />
        Decks
      </button>
      <span className="text-muted-foreground/40">/</span>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            cancelled.current = true;
            setTitle(deck.title);
            e.currentTarget.blur();
          }
        }}
        className="min-w-0 max-w-md flex-1 truncate rounded-md bg-transparent px-1.5 py-1 text-sm font-medium outline-none hover:bg-accent focus:bg-accent"
      />
      <div data-tauri-drag-region className="flex-1 self-stretch" />
      <LintStatus />
      <ViewToggle />
      <button
        type="button"
        onClick={() => void revealItemInDir(`${deck.path}/deck.html`)}
        title="Show deck folder"
        className="rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <FolderOpen className="size-4" />
      </button>
      <ExportMenu
        disabled={deck.slides.length === 0}
        onHtml={() => void exportDeck()}
        onImages={() => void exportImages()}
        onVideo={() => void useVideo.getState().open(deck.id, true)}
      />
      <NarrationButton />
      <button
        type="button"
        disabled={deck.slides.length === 0}
        onClick={() => useApp.getState().setPresenting(true)}
        className="flex items-center gap-1.5 rounded-md bg-primary px-2.5 py-1.5 text-xs font-medium text-primary-foreground shadow-sm hover:opacity-90 disabled:opacity-40"
      >
        <Play className="size-3.5 fill-current" />
        Present
      </button>
      <SidebarToggle />
    </header>
  );
}

/** Direct entry remains visible even when the right sidebar is closed. */
function NarrationButton() {
  const open = useApp((s) => s.chatOpen && s.sidebarTab === "narration");
  return <button
    type="button"
    title="Open narration scripts and audio"
    aria-label="Open narration"
    aria-pressed={open}
    onClick={() => useApp.getState().setSidebarTab("narration")}
    className={cn("flex shrink-0 items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium hover:bg-accent", open && "bg-accent")}
  ><AudioLines className="size-3.5" />Narration</button>;
}

/** Shows or hides both Chat and Narration in the right sidebar. */
function SidebarToggle() {
  const chatOpen = useApp((s) => s.chatOpen);
  const Icon = chatOpen ? PanelRightClose : PanelRightOpen;
  const label = chatOpen ? "Hide sidebar" : "Show sidebar";
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={chatOpen}
      onClick={() => useApp.getState().setChatOpen(!chatOpen)}
      className="rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      <Icon className="size-4" />
    </button>
  );
}

/** The Export button and its choices: one shareable HTML file, or a PNG per slide. */
function ExportMenu(props: { disabled: boolean; onHtml: () => void; onImages: () => void; onVideo: () => void }) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const choose = (action: () => void) => () => {
    setOpen(false);
    action();
  };

  return (
    <div ref={menuRef} className="relative">
      <button
        type="button"
        disabled={props.disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-40"
      >
        <Share className="size-3.5" />
        Export
        <ChevronDown className="size-3 text-muted-foreground" />
      </button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 top-full z-20 mt-1 w-64 rounded-lg border bg-card p-1 shadow-lg"
        >
          <ExportItem icon={FileCode2} label="HTML file" hint="One self-contained file to share" onClick={choose(props.onHtml)} />
          <ExportItem icon={Presentation} label="Narrated MP4" hint="1080p video with saved narration" onClick={choose(props.onVideo)} />
          <ExportItem icon={Images} label="PNG images" hint="One image per slide, in a new folder" onClick={choose(props.onImages)} />
        </div>
      )}
    </div>
  );
}

function ExportItem(props: { icon: typeof Share; label: string; hint: string; onClick: () => void }) {
  const Icon = props.icon;
  return (
    <button
      type="button"
      role="menuitem"
      onClick={props.onClick}
      className="flex w-full items-start gap-2.5 rounded-md px-2 py-1.5 text-left hover:bg-accent"
    >
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <span className="flex flex-col">
        <span className="text-xs font-medium">{props.label}</span>
        <span className="text-2xs text-muted-foreground">{props.hint}</span>
      </span>
    </button>
  );
}

const VIEWS: { id: StageView; label: string; title: string; icon: typeof Code2 }[] = [
  { id: "slides", label: "Slides", title: "Show the rendered slide", icon: Presentation },
  { id: "code", label: "HTML", title: "Show deck.html, scrolled to the selected slide", icon: Code2 },
];

function ViewToggle() {
  const view = useApp((s) => s.view);
  const codeDirty = useApp((s) => s.codeDirty);
  return (
    <div className="flex items-center rounded-md border bg-muted p-0.5">
      {VIEWS.map(({ id, label, title, icon: Icon }) => (
        <button
          key={id}
          type="button"
          title={title}
          aria-pressed={view === id}
          onClick={() => useApp.getState().setView(id)}
          className={cn(
            "flex items-center gap-1.5 rounded px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground",
            view === id && "bg-background text-foreground shadow-sm ring-1 ring-border",
          )}
        >
          <Icon className="size-3.5" />
          {label}
          {id === "code" && codeDirty && (
            <span title="Unsaved changes" className="size-1.5 rounded-full bg-primary" />
          )}
        </button>
      ))}
    </div>
  );
}

/** Lint result for deck.html. When it has issues, clicking puts fix instructions in the chat. */
export function LintStatus() {
  const lint = useApp((s) => s.lint);
  const running = useApp((s) => s.running);

  if (lint === null) {
    return (
      <span
        title="Checking deck.html…"
        className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground"
      >
        <Loader2 className="size-3.5 animate-spin" />
        Lint
      </span>
    );
  }

  const errors = lint.filter((i) => i.severity === "error").length;
  const warnings = lint.length - errors;
  if (lint.length === 0) {
    return (
      <button
        type="button"
        title="deck.html passes lint. Click to check again."
        onClick={() => void useApp.getState().refreshLint()}
        className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium text-emerald-600 hover:bg-accent"
      >
        <CheckCircle2 className="size-3.5" />
        Lint OK
      </button>
    );
  }

  const Icon = errors > 0 ? XCircle : AlertTriangle;
  const summary = [
    errors > 0 && `${errors} ${errors === 1 ? "error" : "errors"}`,
    warnings > 0 && `${warnings} ${warnings === 1 ? "warning" : "warnings"}`,
  ]
    .filter(Boolean)
    .join(", ");
  const details = lint
    .slice(0, 8)
    .map((i) => `Line ${i.line}: ${i.message}`)
    .concat(lint.length > 8 ? [`…and ${lint.length - 8} more`] : [])
    .join("\n");
  return (
    <button
      type="button"
      disabled={running}
      title={`${details}\n\nClick to ask the agent to fix ${lint.length === 1 ? "it" : "them"}.`}
      onClick={() => useApp.getState().fillComposer(lintFixPrompt(lint))}
      className={cn(
        "flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50",
        errors > 0 ? "text-destructive" : "text-amber-600",
      )}
    >
      <Icon className="size-3.5" />
      Lint: {summary}
    </button>
  );
}
