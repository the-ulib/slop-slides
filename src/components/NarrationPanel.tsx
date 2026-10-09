import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useApp } from "../store";
import { useNarration } from "../narrationStore";
import { SpeechControls } from "./SpeechControls";
import { editedManifest, emptyScript, slideReviewHash, type NarrationLanguage } from "../lib/narration";

const field = "w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs outline-none focus:border-primary disabled:opacity-50";
export function NarrationPanel() {
  const deck = useApp((s) => s.deck);
  const selected = useApp((s) => s.selected);
  const running = useApp((s) => s.running);
  const state = useNarration();
  const [scope, setScope] = useState<"slide" | "deck">("slide");
  const [audience, setAudience] = useState("");
  const [minutes, setMinutes] = useState("");
  if (!deck) return null;
  const manifest = state.deckId === deck.id && state.document ? editedManifest(state.document.manifest, state.edits, state.languageEdit, state.settingsEdits) : null;
  const slide = deck.slides.find((s) => s.id === selected);
  const index = deck.slides.findIndex((s) => s.id === selected);
  const script = (selected && manifest?.slides[selected]) || emptyScript();
  const hash = selected ? slideReviewHash(deck, selected) : null;
  const editable = !!manifest && !!slide && !slide.id.startsWith("#");
  const words = script.text.trim().split(/\s+/).filter(Boolean).length;
  const needsReview = !!words && script.reviewedSlideHash !== hash;
  const orphans = manifest ? Object.entries(manifest.slides).filter(([id]) => !deck.slides.some((s) => s.id === id)) : [];
  const edit = (patch: Parameters<typeof state.edit>[1]) => { if (selected) state.edit(selected, patch); };
  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex-1 overflow-y-auto p-3 text-xs">
        <div className="mb-3 flex items-center justify-between text-muted-foreground">
          <span role="status">{state.saving ? "Saving…" : Object.keys(state.edits).length || state.languageEdit || Object.keys(state.settingsEdits).length ? "Unsaved edits" : manifest ? "Saved locally" : "Loading narration…"}</span>
          <span>Local narration</span>
        </div>
        {state.error && (
          <div role="alert" className="mb-3 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 leading-relaxed">
            <p>{state.error}</p>
            {state.conflict ? (
              <div className="mt-2 flex flex-wrap gap-2">
                <button className={field} onClick={() => void state.resolve(false)}>Use file version</button>
                <button className={field} onClick={() => void state.resolve(true)}>Keep my edits</button>
              </div>
            ) : (
              <button className="mt-2 underline" onClick={() => void state.refresh().then(() => state.save())}>Retry</button>
            )}
          </div>
        )}
        <label className="mb-4 block text-muted-foreground">Deck language
          <select aria-label="Deck narration language" className={`${field} mt-1 text-foreground`} value={manifest?.defaultLanguage ?? "en"} disabled={!manifest} onChange={(e) => state.setLanguage(e.target.value as NarrationLanguage)}>
            <option value="en">English</option><option value="de">German</option>
          </select>
        </label>
        <div className="mb-2 flex items-center justify-between">
          <span className="font-medium">{slide ? `Slide ${index + 1} of ${deck.slides.length}` : "Select a slide"}</span>
          <div className="flex gap-1">
            <button aria-label="Previous narration slide" disabled={index <= 0} className="rounded p-1 hover:bg-accent disabled:opacity-30" onClick={() => useApp.getState().selectRelative(-1)}><ChevronLeft className="size-4" /></button>
            <button aria-label="Next narration slide" disabled={index < 0 || index >= deck.slides.length - 1} className="rounded p-1 hover:bg-accent disabled:opacity-30" onClick={() => useApp.getState().selectRelative(1)}><ChevronRight className="size-4" /></button>
          </div>
        </div>
        {slide?.hidden && <p className="mb-2 text-muted-foreground">Hidden slide · excluded from whole-deck drafting and future export.</p>}
        {slide?.id.startsWith("#") && <p className="mb-2 text-amber-600">Waiting for a stable slide ID before saving narration.</p>}
        <label className="block">Narration script
          <textarea aria-label="Narration script" className={`${field} mt-1 min-h-56 resize-y leading-relaxed`} disabled={!editable} value={script.text} placeholder="Explain this slide in your own words, or draft with the agent below…" maxLength={100000} onChange={(e) => edit({ text: e.target.value, reviewedSlideHash: hash })} />
        </label>
        <div className="mt-2 flex items-center justify-between gap-2 text-muted-foreground">
          <span>{words} words{words > 0 ? ` · ~${Math.ceil(words / 150 * 60)}s estimated speech` : " · no speech"}</span>
          <span>{needsReview ? "Review needed" : words ? "Reviewed" : "No script"}</span>
        </div>
        {needsReview && <button className={`${field} mt-2`} onClick={() => edit({ reviewedSlideHash: hash })}>Mark reviewed</button>}
        <label className="mt-3 block text-muted-foreground">Slide language
          <select aria-label="Slide narration language" className={`${field} mt-1 text-foreground`} disabled={!editable} value={script.languageOverride ?? ""} onChange={(e) => edit({ languageOverride: (e.target.value || null) as NarrationLanguage | null })}>
            <option value="">Use deck language</option><option value="en">English</option><option value="de">German</option>
          </select>
        </label>
        <div className="mt-3 grid grid-cols-2 gap-2">
          <label className="text-muted-foreground">Pause before (ms)<input aria-label="Pause before speech" type="number" min={0} max={60000} step={50} className={`${field} mt-1 text-foreground`} disabled={!editable} value={script.leadInMs} onChange={(e) => edit({ leadInMs: Math.min(60000, Math.max(0, Math.round(Number(e.target.value)))) })} /></label>
          <label className="text-muted-foreground">Pause after (ms)<input aria-label="Pause after speech" type="number" min={0} max={60000} step={50} className={`${field} mt-1 text-foreground`} disabled={!editable} value={script.tailMs} onChange={(e) => edit({ tailMs: Math.min(60000, Math.max(0, Math.round(Number(e.target.value)))) })} /></label>
        </div>
        {!words && <label className="mt-3 block text-muted-foreground">Silent slide duration (seconds)
          <input aria-label="Silent slide duration" type="number" min={0.1} max={600} step={0.5} placeholder="Choose a duration for a silent slide" className={`${field} mt-1 text-foreground`} disabled={!editable} value={script.silentDurationMs === null ? "" : script.silentDurationMs / 1000} onChange={(e) => edit({ silentDurationMs: e.target.value === "" ? null : Math.min(600000, Math.max(100, Math.round(Number(e.target.value) * 1000))) })} />
        </label>}
        <SpeechControls deck={deck} selected={selected} manifest={manifest} editable={editable} />
        <div className="mt-5 border-t border-border pt-4">
          <div className="mb-2 font-medium">Draft with your agent</div>
          <p className="mb-3 leading-relaxed text-muted-foreground">Uses your selected chat model. Review and edit the result before generating speech.</p>
          <select aria-label="Draft narration scope" className={field} value={scope} onChange={(e) => setScope(e.target.value as "slide" | "deck")}><option value="slide">Selected slide</option><option value="deck">Whole deck · visible slides</option></select>
          <input aria-label="Narration audience" className={`${field} mt-2`} placeholder="Audience (optional)" value={audience} onChange={(e) => setAudience(e.target.value)} />
          <input aria-label="Narration target duration" className={`${field} mt-2`} type="number" min={0.1} max={180} step={0.5} placeholder="Target minutes (optional)" value={minutes} onChange={(e) => setMinutes(e.target.value)} />
          <button className="mt-3 w-full rounded-md bg-primary px-3 py-2 font-medium text-primary-foreground disabled:opacity-40" disabled={!manifest || !!state.error || !!state.conflict || running || (scope === "slide" ? !editable : !deck.slides.some((s) => !s.hidden && !s.id.startsWith("#")))} onClick={() => void useApp.getState().draftNarration(scope, audience, minutes)}>Draft narration</button>
          {running && <p className="mt-2 text-muted-foreground">The agent is working. Follow progress in Chat.</p>}
        </div>
        {orphans.length > 0 && <details className="mt-5 border-t border-border pt-3">
          <summary className="cursor-pointer text-muted-foreground">Recovered scripts ({orphans.length})</summary>
          <p className="my-2 text-muted-foreground">These slide IDs are no longer in the deck. Restore a slide with the same ID, or copy its script to the selected slide.</p>
          {orphans.map(([id, saved]) => <div key={id} className="mb-2 rounded border border-border p-2"><p className="break-all font-medium">{id}</p><p className="my-1 line-clamp-3 whitespace-pre-wrap">{saved.text || "Silent slide"}</p><button disabled={!editable} className="underline disabled:opacity-40" onClick={() => edit({ text: saved.text, languageOverride: saved.languageOverride, leadInMs: saved.leadInMs, tailMs: saved.tailMs, silentDurationMs: saved.silentDurationMs, reviewedSlideHash: hash })}>Copy to selected slide</button></div>)}
        </details>}
      </div>
    </div>
  );
}
