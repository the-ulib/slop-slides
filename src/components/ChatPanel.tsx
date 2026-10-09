import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  AlertTriangle,
  ArrowUp,
  Check,
  FileText,
  FoldVertical,
  Globe,
  Loader2,
  PanelRightClose,
  Paperclip,
  Pencil,
  PenLine,
  Presentation,
  RotateCcw,
  Search,
  Square,
  SquareTerminal,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { api, errorMessage } from "../lib/api";
import { COMPACT_THRESHOLD, contextPercent, formatTokens, latestContext, windowTokens } from "../lib/context";
import { cn, deckFileUrl } from "../lib/utils";
import { PROVIDERS, type Provider } from "../lib/models";
import { useNarration } from "../narrationStore";
import { useApp, type AssistantMessage, type ChatMessage, type ChatPart, type UserMessage } from "../store";
import { ApprovalCard, PermissionPicker, ApprovalReview } from "./Permissions";
import { EffortPicker, ModelPicker, useDismiss } from "./ModelPicker";

const SUGGESTIONS = [
  "A 6-slide pitch for a neighborhood tool-sharing app, bold and warm",
  "Teach the basics of compound interest to teenagers in 8 slides",
  "Quarterly engineering update: shipped, in progress, risks, asks",
];

export function ChatPanel({ embedded = false }: { embedded?: boolean }) {
  const hasNarration = useNarration((s) => !!s.document && Object.keys(s.document.manifest.slides).length > 0);
  const messages = useApp((s) => s.messages);
  const running = useApp((s) => s.running);
  const provider = useApp((s) => s.selection.provider);
  const cliMissing = useApp(
    (s) => s.providers?.find((p) => p.id === s.selection.provider)?.installed === false,
  );
  const deckEmpty = useApp((s) => (s.deck?.slides.length ?? 0) === 0);
  const composerFill = useApp((s) => s.composerFill);
  const [draft, setDraft] = useState("");

  // Other parts of the app (e.g. the lint status) can hand the composer a prepared message.
  useEffect(() => {
    if (composerFill) setDraft(composerFill.text);
  }, [composerFill]);

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center justify-between px-3">
        <span className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          {embedded ? "Conversation" : "Chat"}
        </span>
        <div className="flex items-center gap-1">
          {messages.length > 0 && (
            <button
              type="button"
              title="New conversation (slides are kept)"
              onClick={() => void useApp.getState().resetChat()}
              className="flex items-center gap-1 rounded-md px-1.5 py-1 text-2xs text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <RotateCcw className="size-3" />
              New chat
            </button>
          )}
          {!embedded && <button
            type="button"
            title="Hide chat"
            aria-label="Hide chat"
            onClick={() => useApp.getState().setChatOpen(false)}
            className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <PanelRightClose className="size-3.5" />
          </button>}
        </div>
      </div>
      {hasNarration && <button className="mx-3 mb-2 rounded-md border border-border px-2 py-1.5 text-xs text-muted-foreground hover:bg-accent" onClick={() => useApp.getState().reviewNarration()}>Review narration</button>}
      {cliMissing && <MissingCli provider={provider} />}
      <MessageList messages={messages} running={running}>
        {messages.length === 0 && (
          <EmptyChat deckEmpty={deckEmpty} onPick={(text) => setDraft(text)} />
        )}
      </MessageList>
      <Composer draft={draft} setDraft={setDraft} />
    </div>
  );
}

function MissingCli({ provider }: { provider: Provider }) {
  const { cli, install } = PROVIDERS[provider];
  return (
    <div className="mx-3 mb-2 flex gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-2.5 text-xs leading-relaxed text-amber-800 dark:text-amber-200">
      <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
      <span>
        {cli} is not installed. {install} Or pick another model below.
      </span>
    </div>
  );
}

function EmptyChat({ deckEmpty, onPick }: { deckEmpty: boolean; onPick: (text: string) => void }) {
  return (
    <div className="flex flex-col gap-3 px-1 pt-6">
      <p className="text-sm text-muted-foreground">
        {deckEmpty
          ? "Describe the presentation you want. The agent writes the theme and slides; you can steer from there."
          : "Ask for changes to the current slide or the whole deck."}
      </p>
      {deckEmpty && (
        <div className="flex flex-col gap-1.5">
          {SUGGESTIONS.map((text) => (
            <button
              key={text}
              type="button"
              onClick={() => onPick(text)}
              className="rounded-lg border px-3 py-2 text-left text-xs leading-relaxed text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              {text}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function MessageList(props: { messages: ChatMessage[]; running: boolean; children?: React.ReactNode }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  });

  return (
    <div
      ref={scrollRef}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      className="min-h-0 flex-1 overflow-y-auto px-3"
    >
      <div className="flex flex-col gap-4 pb-4">
        {props.children}
        {props.messages.map((message) =>
          message.role === "user" ? (
            <UserBubble key={message.id} message={message} />
          ) : (
            <AssistantBlock key={message.id} message={message} />
          ),
        )}
      </div>
    </div>
  );
}

function UserBubble({ message }: { message: UserMessage }) {
  const deck = useApp((s) => s.deck);
  const slideNumber = message.slide && deck ? deck.slides.findIndex((s) => s.id === message.slide) + 1 : 0;
  return (
    <div className="flex flex-col items-end gap-1">
      <div className="selectable max-w-[90%] whitespace-pre-wrap rounded-xl rounded-br-sm bg-accent px-3 py-2 text-sm leading-relaxed">
        {message.text}
      </div>
      {(slideNumber > 0 || message.attachments.length > 0 || message.screenshot) && (
        <div className="flex flex-wrap justify-end gap-1 text-2xs text-muted-foreground">
          {slideNumber > 0 && <span>on slide {slideNumber}</span>}
          {message.sketch && <span>· with sketch</span>}
          {message.screenshot && <span>· with screenshot</span>}
          {message.attachments.map((a) => (
            <span key={a}>· {a.replace(/^assets\//, "")}</span>
          ))}
        </div>
      )}
    </div>
  );
}

function AssistantBlock({ message }: { message: AssistantMessage }) {
  const streaming = message.status === "streaming";
  return (
    <div className="flex flex-col gap-2">
      {message.parts.map((part, i) =>
        part.kind === "text" ? (
          part.text.trim() && (
            <div key={i} className="markdown selectable">
              <Markdown remarkPlugins={[remarkGfm]}>{part.text}</Markdown>
            </div>
          )
        ) : part.kind === "approval" ? (
          <ApprovalCard key={part.approval.id} part={part} />
        ) : part.kind === "approvalReview" ? (
          <ApprovalReview key={part.id} part={part} />
        ) : (
          <ToolRow key={part.id} part={part} />
        ),
      )}
      {streaming && message.compacting && (
        <span className="shimmer-text text-xs">Compacting the conversation…</span>
      )}
      {message.compacted && (
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <FoldVertical className="size-3.5 shrink-0" />
          Conversation compacted
        </span>
      )}
      {streaming &&
        !message.compacting &&
        (message.thinking || (message.parts.length === 0 && !message.compacted)) && (
          <span className="shimmer-text text-xs">Thinking…</span>
        )}
      {streaming && !message.thinking && message.parts.at(-1)?.kind === "tool" && (
        <span className="shimmer-text text-xs">Working…</span>
      )}
      {message.error && (
        <div className="selectable flex gap-2 rounded-lg border border-destructive/30 bg-destructive/10 p-2.5 text-xs leading-relaxed text-destructive">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          <span className="whitespace-pre-wrap break-words">{message.error}</span>
        </div>
      )}
      {message.status === "interrupted" && (
        <span className="text-2xs text-muted-foreground">Stopped</span>
      )}
      {message.status === "done" && message.durationMs !== null && (
        <span className="text-2xs text-muted-foreground/70">
          {(message.durationMs / 1000).toFixed(1)}s
          {message.costUsd ? ` · $${message.costUsd.toFixed(3)}` : ""}
        </span>
      )}
    </div>
  );
}

function ToolRow({ part }: { part: Extract<ChatPart, { kind: "tool" }> }) {
  const deck = useApp((s) => s.deck);
  const { icon: Icon, label, target } = describeTool(part, deck?.id ?? "");
  const slide = target === "deck.html" ? editedSlide(part, deck?.slides.map((s) => s.id) ?? []) : null;
  return (
    <button
      type="button"
      disabled={!slide}
      onClick={() => slide && useApp.getState().select(slide)}
      className="flex min-w-0 items-center gap-2 rounded-md text-left text-xs text-muted-foreground enabled:hover:text-foreground"
    >
      {part.status === "running" ? (
        <Loader2 className="size-3.5 shrink-0 animate-spin" />
      ) : part.status === "error" ? (
        <X className="size-3.5 shrink-0 text-destructive" />
      ) : (
        <Icon className="size-3.5 shrink-0" />
      )}
      <span className="truncate">
        {label}
        {target && <span className="font-mono text-[0.95em] text-foreground/80"> {target}</span>}
        {slide && <span className="font-mono text-[0.95em] text-foreground/80"> · #{slide}</span>}
      </span>
      {part.status === "done" && slide && <Check className="size-3 shrink-0 text-emerald-500" />}
    </button>
  );
}

/** The slide an Edit of deck.html touched, found via an `id="…"` in the edited text. */
function editedSlide(part: Extract<ChatPart, { kind: "tool" }>, slideIds: string[]): string | null {
  for (const key of ["new_string", "old_string"]) {
    const text = part.input[key];
    if (typeof text !== "string") continue;
    for (const match of text.matchAll(/\bid=["']([^"']+)["']/g)) {
      if (match[1] && slideIds.includes(match[1])) return match[1];
    }
  }
  return null;
}

function describeTool(part: Extract<ChatPart, { kind: "tool" }>, deckId: string) {
  const input = part.input;
  const str = (key: string) => (typeof input[key] === "string" ? (input[key] as string) : "");
  // Tool paths are absolute inside the deck folder; show them deck-relative.
  const relative = (path: string) => {
    const normalized = path.replaceAll("\\", "/");
    const marker = `/${deckId}/`;
    const index = normalized.lastIndexOf(marker);
    return index >= 0 ? normalized.slice(index + marker.length) : normalized;
  };
  switch (part.name) {
    case "Write":
      return { icon: FileText, label: "Wrote", target: relative(str("file_path")) };
    case "Edit":
    case "MultiEdit":
      return { icon: Pencil, label: "Edited", target: relative(str("file_path")) };
    case "Read":
      return { icon: FileText, label: "Read", target: relative(str("file_path")) };
    case "Glob":
    case "Grep":
      return { icon: Search, label: "Searched", target: str("pattern") };
    case "WebSearch":
      return { icon: Globe, label: "Searched the web for", target: str("query") };
    case "WebFetch":
      return { icon: Globe, label: "Fetched", target: str("url") };
    case "Bash":
      return { icon: SquareTerminal, label: "Ran", target: str("command") };
    default:
      return { icon: FileText, label: part.name, target: "" };
  }
}

function Composer(props: { draft: string; setDraft: (text: string) => void }) {
  const { draft, setDraft } = props;
  const deck = useApp((s) => s.deck);
  const selected = useApp((s) => s.selected);
  const running = useApp((s) => s.running);
  const [includeSlide, setIncludeSlide] = useState(true);
  const [attachments, setAttachments] = useState<string[]>([]);
  // A slide screenshot handed over with a prepared message (Tidy layout), shown with the images.
  const [screenshot, setScreenshot] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [gridOpen, setGridOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [inputHeight, setInputHeight] = useState<number | null>(null);
  const resizeStart = useRef<{ y: number; height: number } | null>(null);
  const maxInputHeight = Math.max(64, Math.min(240, window.innerHeight * 0.4));
  const resizeInput = (height: number) => setInputHeight(Math.max(64, Math.min(maxInputHeight, height)));

  const slideNumber = selected && deck ? deck.slides.findIndex((s) => s.id === selected) + 1 : 0;
  // Images show as previews in a fan behind the composer; other files stay chips.
  const images = [...(screenshot ? [screenshot] : []), ...attachments.filter(isImage)];
  const files = attachments.filter((a) => !isImage(a));
  const removeAttachment = (path: string) => {
    if (path === screenshot) setScreenshot(null);
    else setAttachments((prev) => prev.filter((p) => p !== path));
  };
  // Marks go out once while on show (see `send`); they stay on the slide as a review.
  const unsentSketch = useApp((s) => {
    const marks = selected ? s.sketches[selected] : undefined;
    return !!selected && !!marks?.length && s.reviewVisible && marks !== s.sketchesSent[selected];
  });
  const sendsSketch = unsentSketch && includeSlide && slideNumber > 0;

  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    if (inputHeight !== null) {
      el.style.height = `${inputHeight}px`;
      return;
    }
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, [draft, inputHeight]);

  useEffect(() => {
    textareaRef.current?.focus();
  }, [draft === ""]); // eslint-disable-line react-hooks/exhaustive-deps

  const composerFill = useApp((s) => s.composerFill);
  useEffect(() => {
    if (composerFill) setScreenshot(composerFill.screenshot ?? null);
    textareaRef.current?.focus();
  }, [composerFill]);

  const importPaths = async (paths: string[]) => {
    if (!deck || paths.length === 0) return;
    try {
      const imported = await api.importAssets(deck.id, paths);
      setAttachments((prev) => [...prev, ...imported.filter((a) => !prev.includes(a))]);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  // Images pasted into the composer are saved as assets and attached.
  const pasteImages = async (files: File[]) => {
    if (!deck) return;
    try {
      const saved: string[] = [];
      for (const file of files) {
        saved.push(await api.saveAsset(deck.id, pastedName(file), await readBase64(file)));
      }
      setAttachments((prev) => [...prev, ...saved.filter((a) => !prev.includes(a))]);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const onPaste = (e: React.ClipboardEvent) => {
    const images = Array.from(e.clipboardData.files).filter((f) => f.type.startsWith("image/"));
    if (images.length === 0) return;
    // Keep any text that came along (e.g. an image copied from a web page has none worth pasting).
    if (!e.clipboardData.getData("text/plain")) e.preventDefault();
    void pasteImages(images);
  };

  // Files dropped anywhere on the window become attachments.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over" || event.payload.type === "enter") setDragging(true);
        else if (event.payload.type === "leave") setDragging(false);
        else if (event.payload.type === "drop") {
          setDragging(false);
          void importPaths(event.payload.paths);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [deck?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const pickFiles = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Images & media", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "mp4", "webm", "pdf", "csv", "md", "txt"] }],
    });
    if (picked) await importPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const submit = () => {
    const text = draft.trim();
    if (!text || running) return;
    void useApp.getState().send(text, { includeSlide: includeSlide && slideNumber > 0, attachments, ...(screenshot && { screenshot }) });
    setDraft("");
    setAttachments([]);
    setScreenshot(null);
    setGridOpen(false);
  };

  return (
    <div className="relative shrink-0 px-3 pb-3">
      {deck && images.length > 0 && (
        gridOpen ? (
          <ImageGrid
            deckId={deck.id}
            images={images}
            onRemove={(path) => {
              removeAttachment(path);
              if (images.length === 1) setGridOpen(false);
            }}
            onClose={() => setGridOpen(false)}
          />
        ) : (
          <ImageFan deckId={deck.id} images={images} onOpen={() => setGridOpen(true)} />
        )
      )}
      <ContextMeter />
      <div
        onClick={(e) => {
          if (e.target === e.currentTarget) textareaRef.current?.focus();
        }}
        className={cn(
          "relative z-10 rounded-2xl border bg-card shadow-composer transition-colors focus-within:border-input",
          dragging && "border-primary ring-2 ring-primary/30",
        )}
      >
        <div
          role="separator"
          aria-label="Resize message input"
          aria-orientation="horizontal"
          aria-valuemin={64}
          aria-valuemax={maxInputHeight}
          aria-valuenow={inputHeight ?? 64}
          tabIndex={0}
          title="Drag up to make the message input taller"
          className="group flex h-4 w-full cursor-ns-resize touch-none items-center justify-center rounded-t-2xl outline-none focus-visible:ring-2 focus-visible:ring-primary"
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            resizeStart.current = { y: e.clientY, height: textareaRef.current?.getBoundingClientRect().height ?? 64 };
            e.currentTarget.setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            const start = resizeStart.current;
            if (start) resizeInput(start.height + start.y - e.clientY);
          }}
          onPointerUp={(e) => {
            resizeStart.current = null;
            if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
          }}
          onPointerCancel={() => { resizeStart.current = null; }}
          onLostPointerCapture={() => { resizeStart.current = null; }}
          onKeyDown={(e) => {
            const height = textareaRef.current?.getBoundingClientRect().height ?? 64;
            if (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "Home" || e.key === "End") {
              e.preventDefault();
              resizeInput(e.key === "Home" ? 64 : e.key === "End" ? maxInputHeight : height + (e.key === "ArrowUp" ? 16 : -16));
            }
          }}
        >
          <span className="h-1 w-8 rounded-full bg-border group-hover:bg-muted-foreground group-focus-visible:bg-primary" />
        </div>
        {(files.length > 0 || slideNumber > 0) && (
          <div className="flex flex-wrap gap-1 px-3.5 pt-3">
            {slideNumber > 0 && (
              <button
                type="button"
                onClick={() => setIncludeSlide((v) => !v)}
                title={includeSlide ? "The agent will know which slide you are on" : "Not referencing the current slide"}
                className={cn(
                  "flex h-5 items-center gap-1 rounded-md border px-1.5 text-xs",
                  includeSlide
                    ? "border-primary/30 bg-primary/10 text-primary"
                    : "text-muted-foreground line-through",
                )}
              >
                <Presentation className="size-3" />
                Slide {slideNumber}
              </button>
            )}
            {sendsSketch && (
              <span
                title="A screenshot of the slide with your marks is sent along; the marks stay on the slide as a review"
                className="flex h-5 items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-1.5 text-xs text-primary"
              >
                <PenLine className="size-3" />
                Sketch
                <button
                  type="button"
                  aria-label="Don't send sketch"
                  title="Don't send these marks; they stay on the slide"
                  onClick={() => selected && useApp.getState().skipSketch(selected)}
                  className="hover:text-foreground"
                >
                  <X className="size-3" />
                </button>
              </span>
            )}
            {files.map((a) => (
              <span key={a} className="flex h-5 items-center gap-1 rounded-md border px-1.5 text-xs text-muted-foreground">
                <FileText className="size-3" />
                {a.replace(/^assets\//, "")}
                <button
                  type="button"
                  aria-label={`Remove ${a.replace(/^assets\//, "")}`}
                  onClick={() => removeAttachment(a)}
                  className="hover:text-foreground"
                >
                  <X className="size-3" />
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          ref={textareaRef}
          value={draft}
          rows={2}
          onChange={(e) => setDraft(e.target.value)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }}
          placeholder={running ? "The agent is working…" : "Ask for slides or changes…"}
          className="block max-h-[max(64px,min(240px,40vh))] min-h-16 w-full resize-none bg-transparent px-3.5 pt-3 text-sm leading-relaxed outline-none placeholder:text-muted-foreground/70"
        />
        <div className="relative flex flex-wrap items-center gap-1 px-2 pt-1 pb-2">
          <ModelPicker />
          <div className="mx-0.5 h-4 w-px bg-border" />
          <EffortPicker />
          <PermissionPicker />
          <div className="flex-1" />
          <button
            type="button"
            onClick={pickFiles}
            title="Attach images or files"
            className="flex size-8 items-center justify-center rounded-full text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <Paperclip className="size-4" />
          </button>
          {running ? (
            <button
              type="button"
              onClick={() => useApp.getState().interrupt()}
              title="Stop"
              className="flex size-8 items-center justify-center rounded-full bg-foreground text-background hover:opacity-80"
            >
              <Square className="size-3 fill-current" />
            </button>
          ) : (
            <button
              type="button"
              onClick={submit}
              disabled={!draft.trim()}
              title="Send"
              className="flex size-8 items-center justify-center rounded-full bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-30"
            >
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

const IMAGE_EXTENSIONS = /\.(png|jpe?g|gif|webp|svg|avif)$/i;
const isImage = (path: string) => IMAGE_EXTENSIONS.test(path);
const assetName = (path: string) => (path.startsWith(".slopslide/") ? "Slide screenshot" : path.replace(/^assets\//, ""));

/** How many previews the fan shows; the rest are counted on a badge. */
const FAN_SIZE = 4;

/**
 * Attached images as a fan of cards tucked behind the composer, peeking up above its top
 * edge. Clicking it opens {@link ImageGrid}.
 */
function ImageFan(props: { deckId: string; images: string[]; onOpen: () => void }) {
  const { deckId, images } = props;
  const shown = images.slice(-FAN_SIZE);
  const mid = (shown.length - 1) / 2;
  const label = `${images.length} attached image${images.length === 1 ? "" : "s"}`;
  return (
    <div className="relative h-10">
      <button
        type="button"
        onClick={props.onOpen}
        title={`${label} · click to review`}
        aria-label={`Show ${label}`}
        className="group absolute right-10 top-0 h-16 w-32"
      >
        {shown.map((path, i) => {
          const offset = i - mid;
          return (
            <img
              key={path}
              src={deckFileUrl(deckId, path)}
              alt={assetName(path)}
              draggable={false}
              style={{ "--x": `${offset * 18}px`, "--r": `${offset * 8}deg` } as React.CSSProperties}
              className={cn(
                "absolute left-1/2 top-0 size-14 -ml-7 rounded-lg border-2 border-card bg-muted object-cover shadow-md transition-transform duration-200",
                "origin-bottom [transform:translateX(var(--x))_rotate(var(--r))]",
                "group-hover:[transform:translateX(calc(var(--x)*1.6))_translateY(-6px)_rotate(calc(var(--r)*1.5))]",
              )}
            />
          );
        })}
        {images.length > shown.length && (
          <span className="absolute -right-1 top-0 z-10 rounded-full bg-foreground px-1.5 text-2xs font-medium text-background tabular-nums">
            +{images.length - shown.length}
          </span>
        )}
      </button>
    </div>
  );
}

/** Every attached image as a preview, each with a way to remove it. */
function ImageGrid(props: { deckId: string; images: string[]; onRemove: (path: string) => void; onClose: () => void }) {
  const { deckId, images, onClose } = props;
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, true, onClose);
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label="Attached images"
      className="absolute bottom-full left-3 right-3 z-20 mb-1 rounded-xl border bg-card p-2 shadow-composer"
    >
      <div className="flex items-center justify-between px-1 pb-2">
        <span className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          {images.length} image{images.length === 1 ? "" : "s"}
        </span>
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="rounded-md p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      </div>
      <div className="grid max-h-72 grid-cols-3 gap-2 overflow-y-auto">
        {images.map((path) => (
          <figure key={path} className="group relative m-0">
            <img
              src={deckFileUrl(deckId, path)}
              alt={assetName(path)}
              draggable={false}
              className="aspect-square w-full rounded-lg border bg-muted object-cover"
            />
            <figcaption className="truncate pt-0.5 text-2xs text-muted-foreground">{assetName(path)}</figcaption>
            <button
              type="button"
              aria-label={`Remove ${assetName(path)}`}
              title="Remove"
              onClick={() => props.onRemove(path)}
              className="absolute right-1 top-1 flex size-5 items-center justify-center rounded-full bg-black/60 text-white opacity-80 hover:opacity-100 group-hover:opacity-100"
            >
              <X className="size-3" />
            </button>
          </figure>
        ))}
      </div>
    </div>
  );
}

/** Clipboard images usually arrive as a generic "image.png"; name them after the paste. */
function pastedName(file: File): string {
  const ext = file.type.split("/")[1]?.split("+")[0] || "png";
  const generic = !file.name || /^image\.\w+$/i.test(file.name);
  return generic ? `pasted-image.${ext === "jpeg" ? "jpg" : ext}` : file.name;
}

function readBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).replace(/^data:[^,]*,/, ""));
    reader.onerror = () => reject(reader.error ?? new Error("Could not read the pasted image"));
    reader.readAsDataURL(file);
  });
}

/**
 * How much of the agent's context window the conversation fills, with a way to compact it
 * once it gets large. Hidden until the selected provider reports its usage.
 */
function ContextMeter() {
  const provider = useApp((s) => s.selection.provider);
  const selectedWindow = useApp((s) => s.selection.contextWindow);
  const context = useApp((s) => latestContext(s.messages, s.selection.provider));
  const running = useApp((s) => s.running);
  if (!context) return null;

  // Claude runs the next turn with the chosen window, which may differ from the last one.
  const window = (provider === "claude" && windowTokens(selectedWindow)) || context.window;
  const { tokens } = context;
  const percent = contextPercent(tokens, window);
  const title =
    tokens === null
      ? "The conversation was compacted. Its new size shows after the next message."
      : `The conversation holds ${tokens.toLocaleString("en-US")} tokens` +
        (window ? ` of the ${window.toLocaleString("en-US")}-token context window` : "");

  // A tab resting on the composer's top edge, opaque so the image fan tucks behind it.
  return (
    <div
      title={title}
      className="relative z-10 mx-3 flex h-8 items-center gap-2.5 rounded-t-xl border border-b-0 bg-card bg-[linear-gradient(var(--muted),var(--muted))] px-3 text-xs"
    >
      <div
        role="progressbar"
        aria-label="Context used"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent === null ? undefined : Math.round(percent)}
        className="h-1.5 w-12 shrink-0 overflow-hidden rounded-full bg-border"
      >
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-500",
            percent === null || percent < 70
              ? "bg-primary/70"
              : percent < 90
                ? "bg-amber-500"
                : "bg-destructive",
          )}
          style={{ width: `${percent ?? 0}%` }}
        />
      </div>
      <span className="flex min-w-0 items-baseline gap-1.5">
        <span className="shrink-0 font-medium tabular-nums text-foreground">
          {tokens === null ? "Compacted" : `${formatTokens(tokens)} tokens`}
        </span>
        <span className="truncate tabular-nums text-muted-foreground">
          {tokens === null
            ? "New size shows after the next message"
            : percent !== null && window
              ? `${Math.round(percent)}% of ${formatTokens(window)} context used`
              : "in context"}
        </span>
      </span>
      <div className="flex-1" />
      {percent !== null && percent > COMPACT_THRESHOLD && (
        <button
          type="button"
          disabled={running}
          onClick={() => void useApp.getState().compact()}
          title="Summarize the conversation so far to free up context (/compact)"
          className="shrink-0 font-medium text-foreground hover:opacity-70 disabled:opacity-40"
        >
          Compact
        </button>
      )}
    </div>
  );
}
