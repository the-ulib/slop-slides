import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useNarration } from "../narrationStore";
import { useSpeech } from "../speechStore";
import { emptyScript, type NarrationManifest } from "../lib/narration";
import { currentTake } from "../lib/speech";
import type { Deck } from "../lib/api";
const field = "w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs disabled:opacity-40";
export function SpeechControls({ deck, selected, manifest, editable }: { deck: Deck; selected: string | null; manifest: NarrationManifest | null; editable: boolean }) {
  const speech = useSpeech(); const narration = useNarration();
  const [scope, setScope] = useState<"slide" | "deck">("slide");
  useEffect(() => { void useSpeech.getState().initialize(); }, []);
  useEffect(() => { void useSpeech.getState().loadTakes(deck.id); }, [deck.id, narration.document?.version]);
  const providers = speech.status?.providers ?? [];
  const providerId = manifest?.speechProviderId ?? "qwen-local";
  const provider = providers.find((p) => p.id === providerId);
  const take = selected && speech.deckId === deck.id ? speech.takes[selected] : undefined;
  const script = selected && manifest ? manifest.slides[selected] ?? emptyScript() : emptyScript();
  const current = take && manifest && currentTake(take, script, manifest, providers);
  const busy = !!speech.job;
  const compatibleVoice = provider?.voices.some((v) => v.id === manifest?.presenterId);
  const compatibleLanguage = provider?.languages.includes(script.languageOverride ?? manifest?.defaultLanguage ?? "en");
  const compatiblePace = provider && manifest && (manifest.pace ?? provider.pace.default) >= provider.pace.min && (manifest.pace ?? provider.pace.default) <= provider.pace.max;
  return <div className="mt-4 border-t border-border pt-4">
    <div className="mb-2 font-medium">Speech</div>
    <label className="block text-muted-foreground">Speech provider
      <select aria-label="Speech provider" className={`${field} mt-1 text-foreground`} value={providerId} disabled={!manifest || busy || !providers.length} onChange={(e) => narration.setProvider(e.target.value)}>
        {!provider && <option value={providerId}>{providerId} · unavailable</option>}
        {providers.map((p) => <option key={p.id} value={p.id}>{p.label} · {p.processing === "local" ? "On device" : p.processing === "cloud" ? "Cloud" : "Test"}</option>)}
      </select>
    </label>
    {!speech.status ? <p className="mt-3 text-muted-foreground">Checking speech providers…</p> : !provider?.available ? <p className="mt-3 text-muted-foreground">{provider?.unavailableReason ?? "This deck’s provider is unavailable. Choose another provider to generate speech. Saved recordings remain playable."}</p> : <>
      <label className="mt-3 block text-muted-foreground">Presenter
        <select aria-label="Narration presenter" className={`${field} mt-1 text-foreground`} value={manifest?.presenterId ?? ""} disabled={!manifest || busy} onChange={(e) => { const voice = provider.voices.find((v) => v.id === e.target.value); if (voice) narration.setPresenter(voice.id, voice.name); }}>
          {!compatibleVoice && <option value={manifest?.presenterId ?? ""}>{manifest?.presenterNameSnapshot ?? "Choose presenter"} · choose compatible presenter</option>}
          {provider.voices.map((voice) => <option key={voice.id} value={voice.id}>{voice.name}</option>)}
        </select>
      </label>
      {provider.voiceHint && <p className="mt-1 text-[11px] text-muted-foreground">{provider.voiceHint}</p>}
      <label className="mt-3 block text-muted-foreground">Speaking pace
        <select aria-label="Narration pace" className={`${field} mt-1 text-foreground`} value={manifest?.pace ?? provider.pace.default} disabled={!manifest || busy} onChange={(e) => narration.setPace(Number(e.target.value))}>
          {!provider.pace.choices.includes(manifest?.pace ?? provider.pace.default) && <option value={manifest?.pace ?? provider.pace.default}>{manifest?.pace ?? provider.pace.default}×{compatiblePace ? "" : " · unsupported"}</option>}
          {provider.pace.choices.map((pace) => <option key={pace} value={pace}>{pace.toFixed(1)}×{pace === provider.pace.default ? " · Default" : ""}</option>)}
        </select>
      </label>
      {!provider.ready && provider.setup ? <div className="mt-3 rounded-md border border-border p-3">
        <p className="font-medium">Set up {provider.label}</p>
        <p className="mt-1 leading-relaxed text-muted-foreground">Voice pack: {(provider.setup.totalBytes / 1e9).toFixed(2)} GB. {provider.setup.detail}</p>
        <button className={`${field} mt-3`} disabled={busy} onClick={() => void speech.install(provider.id)}>Download voice pack</button>
        {provider.setup.importTitle && <button className="mt-2 underline disabled:opacity-40" disabled={busy} onClick={() => void open({ directory: true, multiple: false, title: provider.setup!.importTitle! }).then((path) => { if (typeof path === "string") void speech.install(provider.id, path); }).catch((e: unknown) => useSpeech.setState({ error: String(e) }))}>Import existing voice pack…</button>}
      </div> : provider.ready ? <>
        {!compatibleLanguage && <p className="mt-2 text-amber-600">This provider does not support this script’s language.</p>}
        <select aria-label="Generate audio scope" className={`${field} mt-3`} value={scope} onChange={(e) => setScope(e.target.value as "slide" | "deck")}><option value="slide">Selected slide</option><option value="deck">Whole deck · visible scripts</option></select>
        <button className="mt-2 w-full rounded-md bg-primary px-3 py-2 font-medium text-primary-foreground disabled:opacity-40" disabled={busy || !manifest || !compatibleVoice || !compatiblePace || !compatibleLanguage || !!narration.error || (scope === "slide" ? !editable || !script.text.trim() : !deck.slides.some((s) => !s.hidden && manifest.slides[s.id]?.text.trim()))} onClick={() => void speech.generate(deck.id, scope === "slide" ? selected : null)}>{current && scope === "slide" ? "Reuse saved audio" : "Generate audio"}</button>
        {provider.setup && <details className="mt-3 text-muted-foreground"><summary className="cursor-pointer">Manage voice pack</summary><p className="mt-2">Removing the pack frees {(provider.setup.totalBytes / 1e9).toFixed(2)} GB. Saved recordings remain playable.</p><button className="mt-2 underline disabled:opacity-40" disabled={busy} onClick={() => void speech.remove(provider.id)}>Remove voice pack</button></details>}
      </> : <p className="mt-3 text-muted-foreground">Configure this provider before generating speech.</p>}
    </>}
    {take && <p className="mt-2 text-muted-foreground">{current ? "Recording ready" : "Previous recording · script or voice settings changed"} · {(take.samples / take.sampleRate).toFixed(1)}s. Play below the slide. Saved in this deck’s audio folder.</p>}
    {speech.job && <div className="mt-3 rounded-md bg-accent p-2" role="status">
      <p>{speech.job.detail}</p>
      {speech.job.deckId && speech.job.deckId !== deck.id && <p className="mt-1 text-muted-foreground">Generating for another open deck.</p>}
      {speech.job.total > 0 && <progress aria-label="Speech progress" className="mt-2 w-full" max={speech.job.total} value={speech.job.completed} />}
      <button className="mt-2 underline disabled:opacity-40" disabled={speech.cancelling} onClick={() => void speech.cancel()}>{speech.cancelling ? "Cancelling…" : "Cancel"}</button>
    </div>}
    {speech.error && <div role="alert" className="mt-3 rounded-md bg-amber-500/10 p-2 leading-relaxed">{speech.error}</div>}
    {speech.message && !speech.job && <p role="status" className="mt-2 text-muted-foreground">{speech.message}</p>}
  </div>;
}
