export interface VideoSegment { id: string; startSample: number; endSample: number; startFrame: number; endFrame: number; audioStartSample: number; takeId: string | null }
export interface VideoTimeline { id: string; deckId: string; sourceRevision: number; sampleRate: number; fps: number; totalSamples: number; totalFrames: number; slides: VideoSegment[]; warnings: string[] }
export interface VideoProgress { id: string; stage: string; completed: number; total: number }
/** The exported frame boundary drives preview too; lead/tail silence belongs to its slide. */
export function videoSlideAt(timeline: VideoTimeline, seconds: number): number {
  const frame = Math.max(0, Math.floor(seconds * timeline.fps));
  const next = timeline.slides.findIndex((s) => frame < s.endFrame);
  return next < 0 ? timeline.slides.length - 1 : next;
}
