import { useNarration } from "../narrationStore";
import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { restrictToVerticalAxis } from "@dnd-kit/modifiers";
import { SortableContext, arrayMove, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Copy, Eye, EyeOff, Lock, LockOpen, Palette, Pencil, Plus, SquareSplitVertical, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { api, errorMessage, type Section, type Slide } from "../lib/api";
import { applyOrder, railItems, startsSection } from "../lib/sections";
import { cn } from "../lib/utils";
import { useApp } from "../store";
import { SlideFrame, useSlideVersion } from "./SlideFrame";
import { LayoutPicker, Popover, StylePicker } from "./Templates";

export function SlideRail() {
  const deck = useApp((s) => s.deck);
  const selected = useApp((s) => s.selected);
  const [editingSection, setEditingSection] = useState<number | null>(null);
  const [picker, setPicker] = useState<"style" | "slide" | null>(null);
  const styleButton = useRef<HTMLButtonElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const templates = useApp((s) => s.templates);
  const styleTitle = templates?.find((t) => t.id === deck?.template)?.title;
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));
  if (!deck) return null;

  const items = railItems(deck);
  const keys = items.map((item) => item.key);
  const selectedIndex = deck.slides.findIndex((s) => s.id === selected);
  const canAddSection = selectedIndex >= 0 && !startsSection(deck, selectedIndex);

  const onDragEnd = async ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id) return;
    const order = arrayMove(keys, keys.indexOf(String(active.id)), keys.indexOf(String(over.id)));
    useApp.getState().setDeck(applyOrder(deck, order));
    try {
      useApp.getState().setDeck(await api.reorderSlides(deck.id, order));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
      useApp.getState().setDeck(await api.loadDeck(deck.id));
    }
  };

  const addSlide = async () => {
    const { selected } = useApp.getState();
    try {
      const created = await api.addSlide(deck.id, selected);
      useApp.getState().setDeck(created.deck);
      useApp.getState().select(created.slide);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const addSection = async () => {
    try {
      const next = await api.addSection(deck.id, selected, "New section");
      useApp.getState().setDeck(next);
      const created = next.sections.filter((s) => s.before === selectedIndex).at(-1);
      setEditingSection(created?.index ?? null);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  return (
    <div className="flex h-full flex-col bg-sidebar">
      <div className="flex h-10 shrink-0 items-center justify-between px-3">
        <span className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          Slides · {deck.slides.length}
        </span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={addSection}
            disabled={!canAddSection}
            title={
              selectedIndex >= 0 && !canAddSection
                ? "This slide already starts a section"
                : "Start a section at this slide"
            }
            className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <SquareSplitVertical className="size-4" />
          </button>
          <button
            ref={styleButton}
            type="button"
            onClick={() => setPicker((p) => (p === "style" ? null : "style"))}
            aria-expanded={picker === "style"}
            title={styleTitle ? `Style: ${styleTitle}. Pick another, or save the deck as a template` : "Pick a style for the deck, or save it as a template"}
            className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <Palette className="size-4" />
          </button>
          <button
            ref={addButton}
            type="button"
            onClick={() => setPicker((p) => (p === "slide" ? null : "slide"))}
            aria-expanded={picker === "slide"}
            title="New slide"
            className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <Plus className="size-4" />
          </button>
        </div>
      </div>
      {picker === "style" && (
        <Popover anchor={styleButton} placement="below" width={600} label="Deck style" onClose={() => setPicker(null)}>
          <StylePicker onDone={() => setPicker(null)} />
        </Popover>
      )}
      {picker === "slide" && (
        <Popover anchor={addButton} placement="below" width={560} label="New slide" onClose={() => setPicker(null)}>
          <LayoutPicker mode="add" onBlank={() => void addSlide()} onDone={() => setPicker(null)} />
        </Popover>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-4 pt-1.5">
        {deck.slides.length === 0 ? (
          <p className="px-1 pt-2 text-xs leading-relaxed text-muted-foreground">
            No slides yet. Describe your presentation in the chat, or add a blank slide.
          </p>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={onDragEnd}
          >
            <SortableContext items={keys} strategy={verticalListSortingStrategy}>
              <ol className="flex flex-col gap-3">
                {items.map((item) =>
                  item.kind === "slide" ? (
                    <Thumbnail key={item.key} deckId={deck.id} slide={item.slide} index={item.index} />
                  ) : (
                    <SectionHeader
                      key={item.key}
                      itemKey={item.key}
                      deckId={deck.id}
                      section={item.section}
                      editing={editingSection === item.section.index}
                      onEditing={(editing) => setEditingSection(editing ? item.section.index : null)}
                    />
                  ),
                )}
              </ol>
            </SortableContext>
          </DndContext>
        )}
      </div>
    </div>
  );
}

/** Divider row that names a section. Only the editor shows it; the player never does. */
function SectionHeader(props: {
  itemKey: string;
  deckId: string;
  section: Section;
  editing: boolean;
  onEditing: (editing: boolean) => void;
}) {
  const { deckId, section, editing, onEditing } = props;
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: props.itemKey,
  });

  const commit = async (title: string) => {
    onEditing(false);
    const next = title.trim();
    if (!next || next === section.title) return;
    try {
      useApp.getState().setDeck(await api.renameSection(deckId, section.index, next));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const remove = async () => {
    try {
      useApp.getState().setDeck(await api.deleteSection(deckId, section.index));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  return (
    <li
      ref={setNodeRef}
      data-testid="section-header"
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn("group flex items-center gap-2 pt-1", isDragging && "z-10 opacity-80")}
      {...attributes}
      {...listeners}
    >
      <span className="w-4 shrink-0" />
      {editing ? (
        <input
          autoFocus
          aria-label="Section title"
          defaultValue={section.title}
          onFocus={(e) => e.currentTarget.select()}
          onPointerDown={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") e.currentTarget.blur();
            else if (e.key === "Escape") {
              e.currentTarget.value = section.title;
              e.currentTarget.blur();
            }
          }}
          onBlur={(e) => void commit(e.currentTarget.value)}
          className="min-w-0 flex-1 rounded border border-input bg-background px-1.5 py-0.5 text-2xs font-medium uppercase tracking-wide text-foreground outline-none focus:ring-1 focus:ring-primary"
        />
      ) : (
        <>
          <span
            title="Double-click to rename"
            onDoubleClick={() => onEditing(true)}
            className={cn(
              "min-w-0 truncate text-2xs font-medium uppercase tracking-wide text-muted-foreground",
              !section.title && "italic",
            )}
          >
            {section.title || "Untitled section"}
          </span>
          <span className="h-px flex-1 bg-border" />
          <div className="hidden shrink-0 gap-0.5 group-hover:flex">
            <SectionAction title="Rename section" onClick={() => onEditing(true)}>
              <Pencil className="size-3" />
            </SectionAction>
            <SectionAction title="Remove section" onClick={remove}>
              <X className="size-3" />
            </SectionAction>
          </div>
        </>
      )}
    </li>
  );
}

function SectionAction(props: { title: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={props.title}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        props.onClick();
      }}
      className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      {props.children}
    </button>
  );
}

function Thumbnail({ deckId, slide, index }: { deckId: string; slide: Slide; index: number }) {
  const selected = useApp((s) => s.selected === slide.id);
  const version = useSlideVersion(slide);
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: slide.id,
  });
  const itemRef = useRef<HTMLLIElement | null>(null);

  useEffect(() => {
    if (selected) itemRef.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [selected]);

  const duplicate = async () => {
    try {
      if (!(await useNarration.getState().save())) { useApp.getState().setSidebarTab("narration"); return; }
      const created = await api.duplicateSlide(deckId, slide.id);
      await useNarration.getState().refresh();
      useApp.getState().setDeck(created.deck);
      useApp.getState().select(created.slide);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const toggleHidden = async () => {
    try {
      useApp.getState().setDeck(await api.setSlideHidden(deckId, slide.id, !slide.hidden));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const toggleLocked = async () => {
    try {
      useApp.getState().setDeck(await api.setSlideLocked(deckId, slide.id, !slide.locked));
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  const remove = async () => {
    try {
      const before = useApp.getState().deck?.slides ?? [];
      const neighbor = (before[index + 1] ?? before[index - 1])?.id ?? null;
      if (!(await useNarration.getState().save())) { useApp.getState().setSidebarTab("narration"); return; }
      const next = await api.deleteSlide(deckId, slide.id);
      // deck.html drops the slide's review marks with it; forget them here too.
      useApp.getState().clearSketch(slide.id);
      if (selected) useApp.getState().select(neighbor);
      useApp.getState().setDeck(next);
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  return (
    <li
      ref={(node) => {
        setNodeRef(node);
        itemRef.current = node;
      }}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn("group flex gap-2", isDragging && "z-10 opacity-80")}
      {...attributes}
      {...listeners}
    >
      <span
        className={cn(
          "w-4 shrink-0 pt-0.5 text-right text-2xs tabular-nums text-muted-foreground",
          selected && "font-semibold text-foreground",
          slide.hidden && "line-through opacity-60",
        )}
      >
        {index + 1}
      </span>
      <div className="relative min-w-0 flex-1">
        <button
          type="button"
          onClick={() => useApp.getState().select(slide.id)}
          className={cn(
            "block w-full overflow-hidden rounded-md ring-1 ring-border transition-shadow",
            selected ? "ring-2 ring-primary" : "hover:ring-input",
          )}
        >
          <SlideFrame
            deckId={deckId}
            slideId={slide.id}
            version={version}
            thumbnail
            className={cn(slide.hidden && "opacity-35 grayscale")}
          />
        </button>
        {slide.hidden && <HiddenMark />}
        {slide.locked && (
          <span
            data-testid="locked-mark"
            title="Locked: neither you nor the agent can change it"
            className="pointer-events-none absolute bottom-1 left-1 rounded bg-black/60 p-1 text-white backdrop-blur"
          >
            <Lock className="size-3" aria-label="Locked slide" />
          </span>
        )}
        <div className="absolute right-1 top-1 hidden gap-0.5 group-hover:flex">
          <RailAction title={slide.locked ? "Unlock slide" : "Lock slide"} onClick={toggleLocked}>
            {slide.locked ? <LockOpen className="size-3" /> : <Lock className="size-3" />}
          </RailAction>
          <RailAction title={slide.hidden ? "Show slide" : "Hide slide"} onClick={toggleHidden}>
            {slide.hidden ? <Eye className="size-3" /> : <EyeOff className="size-3" />}
          </RailAction>
          <RailAction title="Duplicate" onClick={duplicate}>
            <Copy className="size-3" />
          </RailAction>
          {!slide.locked && (
            <RailAction title="Delete" onClick={remove}>
              <Trash2 className="size-3" />
            </RailAction>
          )}
        </div>
      </div>
    </li>
  );
}

/** Diagonal strike across a hidden slide's thumbnail. */
function HiddenMark() {
  return (
    <svg
      data-testid="hidden-mark"
      aria-label="Hidden slide"
      role="img"
      viewBox="0 0 16 9"
      preserveAspectRatio="none"
      className="pointer-events-none absolute inset-0 size-full overflow-hidden rounded-md text-muted-foreground"
    >
      <line
        x1="0"
        y1="9"
        x2="16"
        y2="0"
        stroke="currentColor"
        strokeWidth="1.5"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

function RailAction(props: { title: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={props.title}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        props.onClick();
      }}
      className="rounded bg-black/60 p-1 text-white backdrop-blur hover:bg-black/80"
    >
      {props.children}
    </button>
  );
}
