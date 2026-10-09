import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useNarration } from "../narrationStore";
import { useSpeech } from "../speechStore";
import { emptyScript, type NarrationManifest } from "../lib/narration";
import { matchesTake, STOCK_VOICES } from "../lib/speech";
import type { Deck } from "../lib/api";
const field = "w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs disabled:opacity-40";
export function SpeechControls({ deck, selected, manifest, editable }: { deck: Deck; selected: string | null; manifest: NarrationManifest | null; editable: boolean }) {
  const speech = useSpeech(); const narration = useNarration();
  const [scope, setScope] = useState<"slide" | "deck">("slide");
  useEffect(() => { void useSpeech.getState().initialize(); }, []);
  useEffect(() => { void useSpeech.getState().loadTakes(deck.id); }, [deck.id, narration.document?.version]);
  const take = selected && speech.deckId === deck.id ? speech.takes[selected] : undefined;
  const script = selected && manifest ? manifest.slides[selected] ?? emptyScript() : emptyScript();
  const current = take && manifest && take.engineVersion === speech.status?.engineVersion && matchesTake(take, script, manifest);
  const busy = !!speech.job;
  return <div className="mt-4 border-t border-border pt-4">
    <div className="mb-2 font-medium">Local speech</div>
    {speech.status && !speech.status.runtimeAvailable ? <p className="mb-3 text-muted-foreground">Speech generation is available in the macOS desktop preview. This build supports script editing.</p> : <>
      <label className="block text-muted-foreground">Presenter
        <select aria-label="Narration presenter" className={`${field} mt-1 text-foreground`} value={manifest?.presenterId ?? "preset:ryan"} disabled={!manifest} onChange={(e) => { const voice = STOCK_VOICES.find(([id]) => `preset:${id}` === e.target.value); if (voice) narration.setPresenter(`preset:${voice[0]}`, voice[1]); }}>
          {manifest && !STOCK_VOICES.some(([id]) => `preset:${id}` === manifest.presenterId) && <option value={manifest.presenterId}>{manifest.presenterNameSnapshot} · unavailable</option>}
          {STOCK_VOICES.map(([id, name]) => <option key={id} value={`preset:${id}`}>{name}</option>)}
        </select>
      </label>
      <p className="mt-1 text-[11px] text-muted-foreground">Ryan and Aiden are English voices. Preview pronunciation when using another language. Saved personal voices come later.</p>
      <label className="mt-3 block text-muted-foreground">Speaking pace
        <select aria-label="Narration pace" className={`${field} mt-1 text-foreground`} value={manifest?.pace ?? 1.1} disabled={!manifest} onChange={(e) => narration.setPace(Number(e.target.value))}>
          <option value={0.9}>Relaxed · 0.9×</option><option value={1}>Original · 1.0×</option><option value={1.1}>Presentation · 1.1×</option><option value={1.2}>Brisk · 1.2×</option>
        </select>
      </label>
      {!speech.status ? <p className="mt-3 text-muted-foreground">Checking local speech setup…</p> : !speech.status.installed ? <div className="mt-3 rounded-md border border-border p-3">
        <p className="font-medium">Set up local speech once</p>
        <p className="mt-1 leading-relaxed text-muted-foreground">Voice pack: {(speech.status.totalBytes / 1e9).toFixed(2)} GB. After setup, speech runs offline with no service fees. Generation uses about 3 GB of memory on our tested Mac.</p>
        <button className={`${field} mt-3`} disabled={busy} onClick={() => void speech.install()}>Download voice pack</button>
        <button className="mt-2 underline disabled:opacity-40" disabled={busy} onClick={() => void open({ directory: true, multiple: false, title: "Choose the pinned Qwen 0.6B CustomVoice pack" }).then((path) => { if (typeof path === "string") void speech.install(path); }).catch((e: unknown) => useSpeech.setState({ error: String(e) }))}>Import existing voice pack…</button>
      </div> : <>
        <select aria-label="Generate audio scope" className={`${field} mt-3`} value={scope} onChange={(e) => setScope(e.target.value as "slide" | "deck")}><option value="slide">Selected slide</option><option value="deck">Whole deck · visible scripts</option></select>
        <button className="mt-2 w-full rounded-md bg-primary px-3 py-2 font-medium text-primary-foreground disabled:opacity-40" disabled={busy || !manifest || !!narration.error || (scope === "slide" ? !editable || !script.text.trim() : !deck.slides.some((s) => !s.hidden && manifest.slides[s.id]?.text.trim()))} onClick={() => void speech.generate(deck.id, scope === "slide" ? selected : null)}>{current && scope === "slide" ? "Reuse saved audio" : "Generate audio"}</button>
        {take && <p className="mt-2 text-muted-foreground">{current ? "Recording ready" : "Previous recording · script or voice settings changed"} · {(take.samples / take.sampleRate).toFixed(1)}s. Play below the slide. Saved in this deck’s audio folder.</p>}
        <details className="mt-3 text-muted-foreground"><summary className="cursor-pointer">Manage voice pack</summary><p className="mt-2">Removing the pack frees {(speech.status.totalBytes / 1e9).toFixed(2)} GB. Saved recordings remain playable.</p><button className="mt-2 underline disabled:opacity-40" disabled={busy} onClick={() => void speech.remove()}>Remove voice pack</button></details>
      </>}
    </>}
    {speech.job && <div className="mt-3 rounded-md bg-accent p-2" role="status">
      <p>{speech.job.detail}</p>
      {speech.job.deckId && speech.job.deckId !== deck.id && <p className="mt-1 text-muted-foreground">Generating for another open deck.</p>}
      {speech.job.total > 0 && <progress aria-label="Local speech progress" className="mt-2 w-full" max={speech.job.total} value={speech.job.completed} />}
      <button className="mt-2 underline disabled:opacity-40" disabled={speech.cancelling} onClick={() => void speech.cancel()}>{speech.cancelling ? "Cancelling…" : "Cancel"}</button>
    </div>}
    {speech.error && <div role="alert" className="mt-3 rounded-md bg-amber-500/10 p-2 leading-relaxed">{speech.error}</div>}
    {speech.message && !speech.job && <p role="status" className="mt-2 text-muted-foreground">{speech.message}</p>}
  </div>;
}
