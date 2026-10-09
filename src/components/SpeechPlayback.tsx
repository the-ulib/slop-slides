import { useApp } from "../store";
import { useNarration } from "../narrationStore";
import { useSpeech } from "../speechStore";
import { editedManifest, emptyScript } from "../lib/narration";
import { matchesTake } from "../lib/speech";
import { deckFileUrl } from "../lib/utils";
export function SpeechPlayback() {
  const deck = useApp((s) => s.deck); const selected = useApp((s) => s.selected);
  const narration = useNarration(); const speech = useSpeech();
  const take = selected && speech.deckId === deck?.id ? speech.takes[selected] : undefined;
  if (!take || !deck) return null;
  const manifest = narration.deckId === deck.id && narration.document ? editedManifest(narration.document.manifest, narration.edits, narration.languageEdit, narration.settingsEdits) : null;
  const current = manifest && selected && take.engineVersion === speech.status?.engineVersion && matchesTake(take, manifest.slides[selected] ?? emptyScript(), manifest);
  const url = deckFileUrl(deck.id, `audio/${take.id}.wav`);
  return <div className="flex items-center gap-3 border-t border-border bg-background px-4 py-2 text-xs" aria-label="Speech preview">
    <div className="min-w-0 shrink-0"><p className="font-medium">{current ? "Speech preview" : "Previous recording"}</p><p className="text-muted-foreground">{(take.samples / take.sampleRate).toFixed(1)}s · {take.source.pace.toFixed(1)}× pace</p></div>
    <audio key={`${deck.id}:${selected}:${take.id}`} controls preload="metadata" aria-label="Narration audio" src={url} className="h-9 min-w-0 flex-1" onError={() => useSpeech.setState({ error: "This recording could not be played. Generate audio again to recover it." })} />
  </div>;
}
