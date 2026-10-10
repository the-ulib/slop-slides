import type { SpeechProvider } from "./speech";
export interface PauseMarker { start: number; end: number; ms: number }
export function narrationMarkers(text: string, controls?: SpeechProvider["narrationControls"]) {
  const pauses: PauseMarker[] = [];
  let error: string | null = null;
  if (/<\/?\p{L}/u.test(text)) error = "SSML is not supported. Use [pause:800ms] for an explicit pause.";
  const pattern = /\[(?=\s*\p{L})/gu;
  let match: RegExpExecArray | null;
  while (!error && (match = pattern.exec(text))) {
    const start = match.index;
    const close = text.indexOf("]", start);
    if (close < 0) { error = "Incomplete narration marker. Use [pause:800ms]."; break; }
    const marker = text.slice(start + 1, close);
    const value = /^pause:([0-9]+)ms$/.exec(marker);
    if (!value) { error = `Unsupported narration marker [${marker}]. Only [pause:800ms] is supported; tone instructions are unavailable for this provider.`; break; }
    const ms = Number(value[1]);
    if (!Number.isSafeInteger(ms) || ms < 1 || ms > (controls?.maxPauseMs ?? 60000)) { error = "Pause duration must be an integer from 1 to 60000 ms, for example [pause:800ms]."; break; }
    pauses.push({ start, end: close + 1, ms });
    if (pauses.length > (controls?.maxMarkers ?? 100)) { error = "Use at most 100 pause markers per slide."; break; }
    pattern.lastIndex = close + 1;
  }
  const spoken = text.replace(/\[pause:[0-9]+ms\]/g, " ");
  const words = spoken.trim().split(/\s+/).filter(Boolean).length;
  const pauseMs = pauses.reduce((n, p) => n + p.ms, 0);
  if (!error && pauseMs > 600000) error = "Total explicit pauses must not exceed ten minutes.";
  if (!error && text.trim() && !words) error = "Write spoken text as well as pause markers.";
  return { pauses, words, pauseMs, error };
}
export function insertPause(text: string, start: number, end: number, ms = 800) {
  const before = text.slice(0, start), after = text.slice(end);
  const marker = `${before && !/\s$/.test(before) ? " " : ""}[pause:${ms}ms]${after && !/^\s/.test(after) ? " " : ""}`;
  return { text: before + marker + after, cursor: before.length + marker.length };
}
