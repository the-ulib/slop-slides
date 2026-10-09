import { AlertTriangle, Check, ChevronDown, Loader2, RotateCw, Search, Star } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";

import {
  contextWindowLabel,
  effortLabel,
  modelKey,
  PROVIDER_IDS,
  PROVIDERS,
  type Provider,
  type ProviderInfo,
  type ProviderModel,
} from "../lib/models";
import { cn, isMac } from "../lib/utils";
import { useApp } from "../store";
import { ProviderIcon } from "./ProviderIcon";

type Filter = "favorites" | Provider;

type Row = ProviderModel & { provider: Provider };

/** Closes a popover on outside pointer-down or Escape. */
export function useDismiss(ref: RefObject<HTMLElement | null>, open: boolean, close: () => void) {
  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open, close, ref]);
}

const triggerClass =
  "flex h-7 items-center gap-1.5 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground";

export function ModelPicker() {
  const selection = useApp((s) => s.selection);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const close = useMemo(() => () => setOpen(false), []);
  useDismiss(rootRef, open, close);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={triggerClass}
      >
        <ProviderIcon provider={selection.provider} className="size-3.5" />
        <span className="font-medium text-foreground/90">{selection.label}</span>
        <ChevronDown className="size-3 opacity-60" />
      </button>
      {open && <ModelMenu onClose={close} />}
    </div>
  );
}

function ModelMenu({ onClose }: { onClose: () => void }) {
  const selection = useApp((s) => s.selection);
  const favorites = useApp((s) => s.favoriteModels);
  const providers = useApp((s) => s.providers);
  const [filter, setFilter] = useState<Filter>(selection.provider);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const info = (provider: Provider) => providers?.find((p) => p.id === provider);

  const allRows = useMemo<Row[]>(
    () => (providers ?? []).flatMap((p) => p.models.map((m) => ({ ...m, provider: p.id }))),
    [providers],
  );

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q) {
      return allRows.filter((m) =>
        `${m.label} ${m.id} ${PROVIDERS[m.provider].label}`.toLowerCase().includes(q),
      );
    }
    if (filter === "favorites") return allRows.filter((m) => favorites.includes(modelKey(m.provider, m.id)));
    return allRows.filter((m) => m.provider === filter);
  }, [allRows, filter, query, favorites]);

  // Start on the selected model whenever the list changes.
  useEffect(() => {
    const index = rows.findIndex((m) => m.provider === selection.provider && m.id === selection.model);
    setActive(Math.max(0, index));
  }, [rows]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const choose = (row: Row | undefined) => {
    if (!row) return;
    useApp.getState().setModel(row.provider, row.id);
    onClose();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(rows.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(rows[active]);
    } else if ((isMac ? e.metaKey : e.ctrlKey) && /^[1-9]$/.test(e.key)) {
      e.preventDefault();
      choose(rows[Number(e.key) - 1]);
    }
  };

  const rail: { id: Filter; title: string; icon: React.ReactNode }[] = [
    { id: "favorites", title: "Favorites", icon: <Star className="size-4" /> },
    ...PROVIDER_IDS.map((p) => {
      const missing = info(p)?.installed === false;
      return {
        id: p,
        title: missing ? `${PROVIDERS[p].cli} is not installed` : PROVIDERS[p].label,
        icon: (
          <span className="relative">
            <ProviderIcon provider={p} className={cn("size-4", missing && "opacity-40 grayscale")} />
            {missing && (
              <span className="absolute -right-1 -bottom-1 size-2 rounded-full border border-card bg-muted-foreground" />
            )}
          </span>
        ),
      };
    }),
  ];

  // A provider tab that has no models to show explains why instead.
  const status = !query && filter !== "favorites" ? info(filter) : undefined;

  return (
    <div
      onKeyDown={onKeyDown}
      className="absolute bottom-full left-0 z-50 mb-2 flex w-[22rem] overflow-hidden rounded-xl border bg-card shadow-xl shadow-black/20"
    >
      <div className="flex w-12 shrink-0 flex-col items-center gap-1 border-r py-2">
        {rail.map((item) => {
          const selected = !query && filter === item.id;
          return (
            <button
              key={item.id}
              type="button"
              title={item.title}
              onClick={() => {
                setFilter(item.id);
                setQuery("");
              }}
              className={cn(
                "relative flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground",
                selected && "bg-accent text-foreground",
              )}
            >
              {selected && (
                <span className="absolute -left-2 top-1.5 bottom-1.5 w-0.5 rounded-full bg-primary" />
              )}
              {item.icon}
            </button>
          );
        })}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <label className="flex items-center gap-2 border-b px-3 focus-within:border-primary">
          <Search className="size-3.5 shrink-0 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search models..."
            className="h-10 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/70"
          />
        </label>
        <div ref={listRef} className="max-h-72 min-h-32 overflow-y-auto p-1.5">
          {providers === undefined ? (
            <p className="flex items-center justify-center gap-2 px-2 py-8 text-xs text-muted-foreground">
              <Loader2 className="size-3.5 animate-spin" />
              Looking for installed agents…
            </p>
          ) : status && status.models.length === 0 ? (
            <ProviderStatus info={status} />
          ) : rows.length === 0 ? (
            <p className="px-2 py-8 text-center text-xs text-muted-foreground">
              {filter === "favorites" && !query ? "Star a model to pin it here." : "No models match."}
            </p>
          ) : (
            rows.map((row, index) => {
              const key = modelKey(row.provider, row.id);
              const isFavorite = favorites.includes(key);
              return (
                <div
                  key={key}
                  data-index={index}
                  role="option"
                  aria-selected={index === active}
                  onMouseMove={() => setActive(index)}
                  onClick={() => choose(row)}
                  className={cn(
                    "group flex cursor-default items-center gap-2 rounded-lg px-2.5 py-2",
                    index === active && "bg-accent",
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 text-sm text-foreground">
                      <span className="truncate">{row.label}</span>
                      {row.isDefault && (
                        <span className="rounded border border-primary/40 bg-primary/10 px-1 text-[0.625rem] font-semibold leading-4 text-primary">
                          DEFAULT
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 flex items-center gap-1 text-2xs text-muted-foreground">
                      <ProviderIcon provider={row.provider} className="size-3" />
                      {PROVIDERS[row.provider].label}
                    </div>
                  </div>
                  {row.provider === selection.provider && row.id === selection.model && (
                    <Check className="size-3.5 shrink-0 text-primary" />
                  )}
                  {index < 9 && (
                    <kbd className="shrink-0 font-sans text-2xs text-muted-foreground/80">
                      {isMac ? "⌘" : "Ctrl+"}
                      {index + 1}
                    </kbd>
                  )}
                  <button
                    type="button"
                    title={isFavorite ? "Remove from favorites" : "Add to favorites"}
                    onClick={(e) => {
                      e.stopPropagation();
                      useApp.getState().toggleFavoriteModel(key);
                    }}
                    className={cn(
                      "shrink-0 rounded p-0.5 text-muted-foreground/60 hover:text-foreground",
                      isFavorite && "text-amber-500 hover:text-amber-400",
                    )}
                  >
                    <Star className={cn("size-3.5", isFavorite && "fill-current")} />
                  </button>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

function ProviderStatus({ info }: { info: ProviderInfo }) {
  const { cli, install } = PROVIDERS[info.id];
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-6 text-center">
      <ProviderIcon provider={info.id} className="size-6 opacity-40 grayscale" />
      {info.installed ? (
        <>
          <p className="flex items-center gap-1.5 text-sm text-foreground">
            <AlertTriangle className="size-3.5 text-amber-500" />
            Could not list {cli} models
          </p>
          <p className="selectable text-xs leading-relaxed text-muted-foreground">{info.error}</p>
        </>
      ) : (
        <>
          <p className="text-sm text-foreground">{cli} is not installed</p>
          <p className="text-xs leading-relaxed text-muted-foreground">{install}</p>
        </>
      )}
      <button
        type="button"
        onClick={() => void useApp.getState().refreshProviders()}
        className="mt-1 flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <RotateCw className="size-3" />
        Check again
      </button>
    </div>
  );
}

/** Reasoning effort and context window for the selected model, like T3 Code's traits menu. */
export function EffortPicker() {
  const selection = useApp((s) => s.selection);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const close = useMemo(() => () => setOpen(false), []);
  useDismiss(rootRef, open, close);

  const model = useApp((s) =>
    s.providers
      ?.find((p) => p.id === s.selection.provider)
      ?.models.find((m) => m.id === s.selection.model),
  );
  const efforts = model?.efforts ?? [];
  const contextWindows = model?.contextWindows ?? [];
  if (efforts.length === 0 && contextWindows.length === 0) return null;

  const showContextWindow = contextWindows.length > 0 && selection.contextWindow !== null;
  const label = [
    efforts.length > 0 ? effortLabel(selection.effort) : null,
    showContextWindow ? contextWindowLabel(selection.contextWindow!) : null,
  ]
    .filter(Boolean)
    .join(" · ");

  const sections = [
    {
      title: "Effort",
      options: efforts.map((id) => ({ id, label: effortLabel(id) })),
      current: selection.effort,
      choose: (id: string) => useApp.getState().setEffort(id),
    },
    {
      title: "Context Window",
      options: contextWindows.map((id) => ({ id, label: contextWindowLabel(id) })),
      current: selection.contextWindow,
      choose: (id: string) => useApp.getState().setContextWindow(id),
    },
  ].filter((section) => section.options.length > 0);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-expanded={open}
        title={showContextWindow ? "Reasoning effort and context window" : "Reasoning effort"}
        onClick={() => setOpen((v) => !v)}
        className={triggerClass}
      >
        <span>{label}</span>
        <ChevronDown className="size-3 opacity-60" />
      </button>
      {open && (
        <div className="absolute bottom-full left-0 z-50 mb-2 w-40 rounded-xl border bg-card p-1.5 shadow-xl shadow-black/20">
          {sections.map((section, index) => (
            <div key={section.title} className={cn(index > 0 && "mt-1.5 border-t pt-1.5")}>
              <div className="px-2 pt-1 pb-1.5 text-2xs font-medium text-muted-foreground">
                {section.title}
              </div>
              {section.options.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => {
                    section.choose(option.id);
                    close();
                  }}
                  className="flex w-full items-center justify-between rounded-lg px-2 py-1.5 text-left text-sm hover:bg-accent"
                >
                  {option.label}
                  {option.id === section.current && <Check className="size-3.5 text-primary" />}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
