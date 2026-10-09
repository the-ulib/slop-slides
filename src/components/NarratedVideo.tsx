import { memo, useEffect, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { useVideo } from "../videoStore";
import { cn, deckFileUrl, isMac } from "../lib/utils";
import { videoSlideAt } from "../lib/video";
import type { VideoTimeline } from "../lib/video";

const VideoAudio = memo(function VideoAudio({ timeline, onSlide }: { timeline: VideoTimeline; onSlide: (index: number) => void }) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const slideRef = useRef(0);
  const [seconds, setSeconds] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const duration = timeline.totalSamples / timeline.sampleRate;
  const updateSlide = () => {
    const index = videoSlideAt(timeline, audioRef.current?.currentTime ?? 0);
    if (index !== slideRef.current) { slideRef.current = index; onSlide(index); }
  };
  const updateTime = () => { setSeconds(audioRef.current?.currentTime ?? 0); updateSlide(); };
  // Follow the native audio clock each paint, but redraw pictures only at slide
  // boundaries. The transport stays independent and updates on media events.
  useEffect(() => {
    let frame = 0;
    const tick = () => { updateSlide(); frame = requestAnimationFrame(tick); };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [timeline, onSlide]);
  const time = (value: number) => `${Math.floor(value / 60)}:${Math.floor(value % 60).toString().padStart(2, "0")}`;
  return <>
    <audio ref={audioRef} aria-label="Narrated deck audio" preload="auto" src={deckFileUrl(".video", `${timeline.id}/timeline.wav`)} onTimeUpdate={updateTime} onSeeked={updateTime} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => { setPlaying(false); updateTime(); }} onError={() => setError("Could not play the prepared audio. Close and reopen the preview.")} />
    <div className="flex items-center gap-3 rounded-lg bg-white/10 px-3 py-2">
      <button aria-label={playing ? "Pause narrated deck" : "Play narrated deck"} className="rounded p-2 hover:bg-white/10" onClick={() => {
        const audio = audioRef.current;
        if (!audio) return;
        if (audio.paused) { setError(null); void audio.play().catch(() => setError("Playback could not start. Try Play again.")); } else audio.pause();
      }}>{playing ? <Pause size={18} /> : <Play size={18} />}</button>
      <span className="tabular-nums" aria-label="Playback time">{time(seconds)}</span>
      <input aria-label="Seek narrated deck" type="range" className="min-w-0 flex-1 accent-white" min={0} max={duration} step={1 / timeline.fps} value={Math.min(seconds, duration)} onChange={(e) => {
        if (audioRef.current) { audioRef.current.currentTime = Number(e.target.value); updateTime(); }
      }} />
      <span className="tabular-nums">{time(duration)}</span>
    </div>
    {error && <p role="alert">{error}</p>}
  </>;
});

export function NarratedVideo() {
  const state = useVideo();
  const [index, setIndex] = useState(0);
  useEffect(() => { setIndex(0); }, [state.timeline?.id]);
  useEffect(() => {
    if (!state.deckId) return;
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.preventDefault(); useVideo.getState().close(); } };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [state.deckId]);
  if (!state.deckId) return null;
  const { timeline } = state;
  const stage = state.progress?.stage;
  const detail = stage === "rendering" ? "Rendering slides" : stage === "encoding" ? "Encoding video" : stage === "finalizing" ? "Finalizing MP4" : "Preparing deck and audio";
  return <div role="dialog" aria-modal="true" aria-label="Narrated deck preview" className="fixed inset-0 z-50 flex flex-col bg-neutral-950 text-white">
    <div className={cn("flex items-center justify-between gap-3 border-b border-white/15 px-5 py-3 text-sm", isMac && "pl-[84px]")}>
      <span>Narrated deck{timeline ? ` · Slide ${index + 1} of ${timeline.slides.length} · ${(timeline.totalSamples / timeline.sampleRate).toFixed(1)} s` : ""}</span>
      <div className="flex gap-3">
        {timeline && <button disabled={state.busy} className="rounded-md border border-white/25 px-3 py-1.5 disabled:opacity-40" onClick={() => void state.export()}>Export MP4</button>}
        <button className="rounded-md border border-white/25 px-3 py-1.5" onClick={state.close}>{state.busy ? "Cancel" : "Close preview"}</button>
      </div>
    </div>
    <div className="flex min-h-0 flex-1 items-center justify-center p-6">
      {timeline && <img onLoad={() => {
        // Decode the following slide ahead of its boundary without keeping the
        // entire deck's image pixels resident.
        if (index + 1 < timeline.slides.length) {
          const next = new Image(); next.src = deckFileUrl(".video", `${timeline.id}/frame-${index + 1}.png`); void next.decode().catch(() => {});
        }
      }} alt={`Narrated slide ${index + 1}`} className="max-h-full max-w-full object-contain" src={deckFileUrl(".video", `${timeline.id}/frame-${index}.png`)} />}
      {!timeline && !state.busy && !state.error && <span>No narrated preview prepared.</span>}
    </div>
    <div className="mx-auto w-full max-w-4xl space-y-2 px-5 pb-5 text-xs text-white/75">
      {timeline && <VideoAudio key={timeline.id} timeline={timeline} onSlide={setIndex} />}
      {state.busy && <div role="status"><p>{detail}…</p>{(state.progress?.total ?? 0) > 0 && <progress aria-label="Video progress" className="mt-2 w-full" max={state.progress!.total} value={state.progress!.completed} />}</div>}
      {state.error && <div role="alert" className="whitespace-pre-line rounded-md bg-amber-500/15 p-3 text-amber-200">{state.error}<button className="ml-3 underline" onClick={() => void state.open(state.deckId!)}>Retry preparation</button></div>}
      {state.output && <p role="status">Saved MP4: {state.output}</p>}
      {timeline && <details><summary className="cursor-pointer">Export notes · frozen revision {timeline.sourceRevision}</summary><p className="mt-1">{timeline.warnings.join(" ")} Changes made after preparation are included when you reopen the preview.</p></details>}
    </div>
  </div>;
}
