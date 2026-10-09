import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Check, FolderOpen, Loader2, Plus, Save } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import type { TemplateSummary } from "../lib/api";
import { cn, layoutLabel, templateSlideUrl } from "../lib/utils";
import { useApp } from "../store";
import { SlideFrame } from "./SlideFrame";

const LAST_TEMPLATE_KEY = "slopslide.layoutTemplate";

/** Loads the templates once, for whichever picker needs them first. */
export function useTemplates(): TemplateSummary[] | undefined {
  const templates = useApp((s) => s.templates);
  useEffect(() => {
    if (useApp.getState().templates === undefined) void useApp.getState().refreshTemplates();
  }, []);
  return templates;
}

/** A still preview of one of a template's slides. */
export function TemplateSlide(props: { template: string; slide: string; className?: string }) {
  return (
    <SlideFrame
      deckId={props.template}
      slideId={props.slide}
      version=""
      thumbnail
      url={templateSlideUrl(props.template, props.slide)}
      className={props.className}
    />
  );
}

/**
 * A panel floating next to `anchor`, kept on screen. It sits on top of everything (the rail
 * and the stage bar would clip it) and closes on Escape or a click elsewhere.
 */
export function Popover(props: {
  anchor: RefObject<HTMLElement | null>;
  placement: "below" | "above";
  width: number;
  label: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const { anchor, placement, width, onClose } = props;
  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<React.CSSProperties>({ visibility: "hidden" });

  useLayoutEffect(() => {
    const rect = anchor.current?.getBoundingClientRect();
    if (!rect) return;
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    setPosition(
      placement === "below"
        ? { left, top: rect.bottom + 6, maxHeight: window.innerHeight - rect.bottom - 16 }
        : { left, bottom: window.innerHeight - rect.top + 6, maxHeight: rect.top - 16 },
    );
  }, [anchor, placement, width]);

  useEffect(() => {
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!panelRef.current?.contains(target) && !anchor.current?.contains(target)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [anchor, onClose]);

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={props.label}
      style={{ position: "fixed", width, ...position }}
      className="z-50 flex flex-col overflow-hidden rounded-xl border bg-card text-foreground shadow-lg"
    >
      {props.children}
    </div>
  );
}

/** The template a picker starts on: the deck's own, else the one picked last, else the first. */
function initialTemplate(templates: TemplateSummary[], deckTemplate: string | null | undefined): string | null {
  const known = (id: string | null | undefined) => (id && templates.some((t) => t.id === id) ? id : null);
  return known(deckTemplate) ?? known(localStorage.getItem(LAST_TEMPLATE_KEY)) ?? templates[0]?.id ?? null;
}

/**
 * Template layouts as thumbnails, to add a slide on one (`add`) or rebuild the selected slide
 * on one (`change`).
 */
export function LayoutPicker(props: { mode: "add" | "change"; onDone: () => void; onBlank?: () => void }) {
  const templates = useTemplates();
  const deckTemplate = useApp((s) => s.deck?.template);
  const [picked, setPicked] = useState<string | null>(null);
  const current = picked ?? (templates ? initialTemplate(templates, deckTemplate) : null);
  const switchable = props.mode === "change";
  const template = templates?.find((t) => t.id === current);
  const own = templates?.filter((t) => !t.builtin) ?? [];
  const builtin = templates?.filter((t) => t.builtin) ?? [];
  const sameStyle = !!template && template.id === deckTemplate;

  const choose = (slide: string) => {
    if (!template) return;
    localStorage.setItem(LAST_TEMPLATE_KEY, template.id);
    const app = useApp.getState();
    void (props.mode === "add" ? app.addLayoutSlide(template.id, slide) : app.changeLayout(template.id, slide));
    props.onDone();
  };

  const hint =
    props.mode === "add"
      ? sameStyle
        ? "Adds a copy of the layout with placeholder text after the selected slide."
        : "This deck has another style: the chat gets a request for the agent to recreate the layout in it."
      : "The chat gets a request for the agent to rebuild this slide on the layout, keeping its content.";

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="text-xs font-medium">{props.mode === "add" ? "New slide" : "Change layout"}</span>
        <div className="flex-1" />
        {switchable && templates && templates.length > 0 && (
          <select
            aria-label="Template"
            value={current ?? ""}
            onChange={(e) => setPicked(e.target.value)}
            className="max-w-56 rounded-md border bg-background px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-primary"
          >
            {own.length > 0 && (
              <optgroup label="Your templates">
                {own.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.title}
                  </option>
                ))}
              </optgroup>
            )}
            <optgroup label="Built-in">
              {builtin.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.title}
                  {t.id === deckTemplate ? " (this deck)" : ""}
                </option>
              ))}
            </optgroup>
          </select>
        )}
      </div>
      <div className="min-h-0 overflow-y-auto p-3">
        {templates === undefined ? (
          <Loading />
        ) : !template ? (
          <p className="text-xs text-muted-foreground">No templates found.</p>
        ) : (
          <ul className="grid grid-cols-3 gap-3">
            {props.onBlank && (
              <li>
                <button
                  type="button"
                  aria-label="Blank slide"
                  title="Add blank slide"
                  onClick={() => {
                    props.onBlank?.();
                    props.onDone();
                  }}
                  className="group flex w-full flex-col gap-1 text-left"
                >
                  <div className="flex aspect-video w-full items-center justify-center rounded-md border border-dashed text-muted-foreground transition group-hover:border-primary group-hover:text-foreground">
                    <Plus className="size-5" />
                  </div>
                  <span className="truncate text-2xs text-muted-foreground group-hover:text-foreground">Blank</span>
                </button>
              </li>
            )}
            {template.slides.map((slide) => (
              <li key={`${template.id}/${slide}`}>
                <button
                  type="button"
                  aria-label={`${layoutLabel(slide)} layout`}
                  onClick={() => choose(slide)}
                  className="group flex w-full flex-col gap-1 text-left"
                >
                  <div className="w-full overflow-hidden rounded-md ring-1 ring-border transition group-hover:ring-2 group-hover:ring-primary">
                    <TemplateSlide template={template.id} slide={slide} />
                  </div>
                  <span className="truncate text-2xs text-muted-foreground group-hover:text-foreground">{layoutLabel(slide)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <p className="border-t px-3 py-2 text-2xs leading-relaxed text-muted-foreground">{hint}</p>
    </div>
  );
}

/** Restyles the deck like a template, or saves the deck as a template. */
export function StylePicker(props: { onDone: () => void }) {
  const deck = useApp((s) => s.deck);
  const running = useApp((s) => s.running);
  const templates = useTemplates();
  if (!deck) return null;
  return (
    <StylePanel
      templates={templates}
      current={deck.template ?? null}
      empty={deck.slides.length === 0}
      busy={running}
      onPicked={props.onDone}
    />
  );
}

function StylePanel(props: {
  templates: TemplateSummary[] | undefined;
  current: string | null;
  empty: boolean;
  busy: boolean;
  onPicked: () => void;
}) {
  const { templates, current } = props;
  const pick = (template: TemplateSummary) => {
    void useApp.getState().applyStyle(template.id);
    props.onPicked();
  };

  return (
    <>
      <div className="border-b px-3 py-2">
        <span className="text-xs font-medium">Style</span>
        <p className="text-2xs text-muted-foreground">
          {props.empty
            ? "The deck takes the style's fonts, colors, and layouts."
            : "The chat gets a request for the agent to restyle every slide, keeping the content."}
        </p>
      </div>
      <div className="min-h-0 overflow-y-auto p-3">
        {templates === undefined ? (
          <Loading />
        ) : (
          <ul className="grid grid-cols-3 gap-3">
            {templates.map((t) => (
              <li key={t.id}>
                <button
                  type="button"
                  disabled={props.busy && !props.empty}
                  aria-label={`${t.title} style`}
                  aria-current={t.id === current || undefined}
                  onClick={() => pick(t)}
                  className="group flex w-full flex-col gap-1 text-left disabled:opacity-50"
                >
                  <div
                    className={cn(
                      "w-full overflow-hidden rounded-md ring-1 ring-border transition group-hover:ring-2 group-hover:ring-input",
                      t.id === current && "ring-2 ring-primary group-hover:ring-primary",
                    )}
                  >
                    {t.slides[0] && <TemplateSlide template={t.id} slide={t.slides[0]} />}
                  </div>
                  <span className="flex items-center gap-1 text-2xs">
                    <span className="truncate">{t.title}</span>
                    {!t.builtin && <span className="rounded bg-muted px-1 text-muted-foreground">Yours</span>}
                    {t.id === current && <Check className="size-3 text-primary" />}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {!props.empty && <SaveAsTemplate />}
    </>
  );
}

/** Saves the deck as a template: its design and layouts, with placeholder text. */
function SaveAsTemplate() {
  const title = useApp((s) => s.deck?.title ?? "");
  const [name, setName] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<TemplateSummary | null>(null);

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    if (name === null || saving) return;
    setSaving(true);
    const created = await useApp.getState().saveAsTemplate(name.trim() || title);
    setSaving(false);
    if (created) {
      setSaved(created);
      setName(null);
    }
  };

  return (
    <div className="border-t px-3 py-2">
      {name === null ? (
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => {
              setSaved(null);
              setName(title);
            }}
            className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs font-medium hover:bg-accent"
          >
            <Save className="size-3.5" />
            Save deck as template…
          </button>
          {saved && (
            <span className="flex min-w-0 items-center gap-1 text-2xs text-muted-foreground">
              <span className="truncate">Saved “{saved.title}”.</span>
              {saved.path && (
                <button
                  type="button"
                  title="Show the template's folder"
                  onClick={() => void revealItemInDir(`${saved.path}/deck.html`)}
                  className="rounded p-0.5 hover:bg-accent hover:text-foreground"
                >
                  <FolderOpen className="size-3.5" />
                </button>
              )}
            </span>
          )}
        </div>
      ) : (
        <form onSubmit={save} className="flex items-center gap-2">
          <input
            autoFocus
            aria-label="Template name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                setName(null);
              }
            }}
            className="min-w-0 flex-1 rounded-md border bg-background px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-primary"
          />
          <button
            type="submit"
            disabled={saving}
            className="flex items-center gap-1 rounded-md bg-primary px-2 py-1 text-xs font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {saving && <Loader2 className="size-3 animate-spin" />}
            Save
          </button>
        </form>
      )}
      <p className="mt-1 text-2xs text-muted-foreground">
        Copies the deck to ~/.slopslides/templates with placeholder text in place of its content.
      </p>
    </div>
  );
}

/** Style choice for a new deck. */
export function TemplateSelect(props: { value: string | null; onChange: (template: string | null) => void }) {
  const templates = useTemplates();
  return (
    <select
      aria-label="Style"
      value={props.value ?? ""}
      onChange={(e) => props.onChange(e.target.value || null)}
      className="h-10 rounded-lg border bg-card px-2 text-sm outline-none focus:border-input focus:ring-2 focus:ring-primary/20"
    >
      <option value="">Any style</option>
      {(templates ?? []).map((t) => (
        <option key={t.id} value={t.id}>
          {t.title}
          {t.builtin ? "" : " (yours)"}
        </option>
      ))}
    </select>
  );
}

function Loading() {
  return (
    <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Loader2 className="size-3.5 animate-spin" />
      Loading templates…
    </p>
  );
}
