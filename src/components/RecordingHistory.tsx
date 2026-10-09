import { useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { matchesTake, type SpeechHistoryTake, type SpeechTake } from "../lib/speech";
import type { NarrationManifest, SlideNarration } from "../lib/narration";
import { useSpeech } from "../speechStore";

export function RecordingHistory({ deckId, slide, script, manifest, accepted, editable, version }: { deckId: string; slide: string; script: SlideNarration; manifest: NarrationManifest; accepted?: SpeechTake; editable: boolean; version?: string }) {
  const speech = useSpeech();
  const [history, setHistory] = useState<SpeechHistoryTake[]>([]);
  const [chosen, setChosen] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let alive = true;
    setHistory([]); setChosen(""); setError(null); setLoading(true);
    void api.speechHistory(deckId, slide).then((takes) => {
      if (alive) { setHistory(takes ?? []); setChosen(accepted?.id ?? takes?.[0]?.id ?? ""); }
    }).catch((e: unknown) => { if (alive) setError(errorMessage(e)); }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [deckId, slide, accepted?.id, version]);
  useEffect(() => {
    useSpeech.setState({ preview: null });
    return () => { useSpeech.setState({ preview: null }); };
  }, [deckId, slide]);
  const takes: SpeechHistoryTake[] = accepted && !history.some((t) => t.id === accepted.id) ? [{ ...accepted, createdAt: 0 }, ...history] : history;
  const take = takes.find((t) => t.id === chosen) ?? takes[0];
  const restoresText = !!take && take.source.text !== script.text.trim().replace(/\r\n/g, "\n");
  const selected = !!take && take.id === accepted?.id && matchesTake(take, script, manifest);
  const voiceName = (t: SpeechTake) => speech.status?.providers.find((p) => p.id === (t.source.providerId ?? "qwen-local"))?.voices.find((v) => v.id === t.source.presenterId)?.name ?? t.source.presenterId.replace(/^preset:/, "");
  return <details className="mt-3 rounded-md border border-border p-2">
    <summary className="cursor-pointer text-muted-foreground">Recording history ({takes.length})</summary>
    {loading && <p className="mt-2">Loading recordings…</p>}
    {error && <p role="alert" className="mt-2">{error}</p>}
    {!loading && !takes.length && !error && <p className="mt-2 text-muted-foreground">Recordings appear here after generating audio.</p>}
    {take && <>
      <select aria-label="Recording history" className="mt-2 w-full rounded-md border border-border bg-background p-2" value={take.id} onChange={(e) => { setChosen(e.target.value); useSpeech.setState({ preview: null }); }}>
        {takes.map((t, index) => <option key={t.id} value={t.id}>Take {takes.length - index} · {t.id === accepted?.id ? "Selected · " : ""}{t.createdAt ? new Date(t.createdAt).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" }) : "Earlier recording"} · {voiceName(t)} · {(t.samples / t.sampleRate).toFixed(1)}s · {t.source.pace}×</option>)}
      </select>
      <p className="mt-2 text-muted-foreground">{tLanguage(take)} · {voiceName(take)} · {take.source.pace}×</p>
      <p className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-accent p-2 leading-relaxed" aria-label="Recording script">{take.source.text}</p>
      <div className="mt-2 flex flex-wrap gap-3">
        <button className="underline" onClick={() => useSpeech.setState({ preview: { deckId, slide, take, requestId: crypto.randomUUID() } })}>Listen to this recording</button>
        <button className="underline disabled:opacity-40" disabled={!!speech.job || !editable || selected} onClick={() => void speech.selectTake(deckId, slide, take.id)}>{selected ? "Selected for video" : restoresText ? "Use recording & script" : "Use this recording"}</button>
      </div>
      {!selected && <p className="mt-2 text-[11px] text-muted-foreground">{restoresText ? "Using this recording restores the script above and its voice settings for this slide." : "Using this recording restores its voice settings for this slide."} Listening leaves the video selection unchanged.</p>}
    </>}
  </details>;
}
function tLanguage(take: SpeechTake) { return take.source.language === "de" ? "German" : "English"; }
