import { ask } from "@tauri-apps/plugin-dialog";
import { Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { api, errorMessage, type DeckSummary } from "../lib/api";
import { cn, isMac, relativeTime } from "../lib/utils";
import { useApp } from "../store";
import { SlideFrame } from "./SlideFrame";
import { TemplateSelect } from "./Templates";

export function Home() {
  const [decks, setDecks] = useState<DeckSummary[] | null>(null);
  const [title, setTitle] = useState("");
  const [template, setTemplate] = useState<string | null>(null);
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = async () => {
    setLoading(true);
    try {
      setDecks(await api.listDecks());
      setLibraryError(null);
    } catch (error) {
      const message = errorMessage(error);
      setLibraryError(message);
      useApp.getState().setError(message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const create = async (event: React.FormEvent) => {
    event.preventDefault();
    await useApp.getState().createDeck(title.trim() || "Untitled deck", template);
  };

  const remove = async (deck: DeckSummary) => {
    const confirmed = await ask(`Delete “${deck.title}”? Its folder and slides are removed.`, {
      title: "Delete deck",
      kind: "warning",
      okLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await api.deleteDeck(deck.id);
      await refresh();
    } catch (error) {
      useApp.getState().setError(errorMessage(error));
    }
  };

  return (
    <div className="flex h-full flex-col">
      <header data-tauri-drag-region className={cn("flex h-12 shrink-0 items-center", isMac ? "pl-[84px]" : "pl-4")}>
        <span data-tauri-drag-region className="text-sm font-semibold tracking-tight">
          SlopSlide
        </span>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-5xl flex-col gap-8 px-8 py-10">
          <form onSubmit={create} className="flex flex-col gap-3">
            <h1 className="text-2xl font-semibold tracking-tight">What are you presenting?</h1>
            <div className="flex gap-2">
              <input
                autoFocus
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Deck title, e.g. Series A pitch"
                className="h-10 flex-1 rounded-lg border bg-card px-3 text-sm outline-none focus:border-input focus:ring-2 focus:ring-primary/20"
              />
              <TemplateSelect value={template} onChange={setTemplate} />
              <button
                type="submit"
                className="flex h-10 items-center gap-1.5 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground hover:opacity-90"
              >
                <Plus className="size-4" />
                New deck
              </button>
            </div>
          </form>

          {libraryError && (
            <div className="flex items-center justify-between gap-4 rounded-lg border p-4 text-sm">
              <p>Couldn’t open the deck library: {libraryError}</p>
              <button type="button" disabled={loading} onClick={() => void refresh()} className="shrink-0 rounded-md border px-3 py-1.5 disabled:opacity-50">
                {loading ? "Opening…" : "Retry opening library"}
              </button>
            </div>
          )}

          {decks && decks.length > 0 && (
            <section className="flex flex-col gap-3">
              <h2 className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                Recent decks
              </h2>
              <ul className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-5">
                {decks.map((deck) => (
                  <li key={deck.id} className="group relative">
                    <button
                      type="button"
                      onClick={() => void useApp.getState().openDeck(deck.id)}
                      className="flex w-full flex-col gap-2 text-left"
                    >
                      <div className="w-full overflow-hidden rounded-lg ring-1 ring-border transition group-hover:ring-input group-hover:shadow-md">
                        {deck.firstSlide ? (
                          <SlideFrame deckId={deck.id} slideId={deck.firstSlide} version={String(deck.updatedMs)} thumbnail />
                        ) : (
                          <div className="flex aspect-video items-center justify-center bg-muted text-xs text-muted-foreground">
                            Empty deck
                          </div>
                        )}
                      </div>
                      <div className="flex flex-col px-0.5">
                        <span className="truncate text-sm font-medium">{deck.title}</span>
                        <span className="text-xs text-muted-foreground">
                          {deck.slideCount} {deck.slideCount === 1 ? "slide" : "slides"} ·{" "}
                          {relativeTime(deck.updatedMs)}
                        </span>
                      </div>
                    </button>
                    <button
                      type="button"
                      title="Delete deck"
                      onClick={() => void remove(deck)}
                      className="absolute right-2 top-2 hidden rounded-md bg-black/60 p-1.5 text-white backdrop-blur hover:bg-black/80 group-hover:block"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      </main>
    </div>
  );
}
