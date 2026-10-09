import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { Slide } from "../lib/api";
import { isPasteboardUrl, slideUrl } from "../lib/utils";
import { useApp } from "../store";

const STAGE_W = 1920;
const STAGE_H = 1080;

interface SlideFrameProps {
  deckId: string;
  slideId: string;
  /** Any change reloads the preview; see {@link useSlideVersion}. */
  version: string;
  /** Thumbnails render the final animation frame and ignore pointer input. */
  thumbnail?: boolean;
  /** Loads the slide editor (final animation frame, editable); changing the key reloads it. */
  editKey?: string;
  /**
   * The space (CSS px) the preview fills around the centered slide, as a pasteboard to pan and
   * zoom on. The pasteboard draws the slide's frame, which moves with it.
   */
  arena?: { width: number; height: number };
  className?: string;
  onFrameReady?: (frame: HTMLIFrameElement) => void;
  /** Loads this URL instead of the deck's slide (e.g. a template's, see `templateSlideUrl`). */
  url?: string;
}

/**
 * One slide of deck.html, rendered by the deck's own player in a 1920×1080 iframe scaled to
 * fill its (16:9) container. When the slide changes, the new version loads behind the
 * current one and swaps in once painted, so edits stream in without white flashes.
 */
export function SlideFrame({ deckId, slideId, version, thumbnail, editKey, arena, className, onFrameReady, url }: SlideFrameProps) {
  const src = url ?? slideUrl(deckId, slideId, version, thumbnail || editKey !== undefined, editKey, arena !== undefined && !thumbnail);

  const containerRef = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(0);
  const [shown, setShown] = useState(src);
  const [pending, setPending] = useState<string | null>(null);

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setScale(entry.contentRect.width / STAGE_W);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Until the first layout nothing is shown, so there is nothing to keep while a new version
  // loads: the preview starts with whatever is current then (e.g. once the stage's arena is known).
  const [laidOut, setLaidOut] = useState(false);
  if (!laidOut && src !== shown) setShown(src);
  if (!laidOut && scale > 0) setLaidOut(true);

  useEffect(() => {
    if (laidOut && src !== shown) setPending(src);
  }, [src, shown, laidOut]);

  const frames = pending ? [shown, pending] : [shown];
  // Only a pasteboard preview fills the arena, with the slide in its middle. A plain preview given
  // the same room would scale the slide up to fill it.
  const slideW = STAGE_W * scale;
  const slideH = STAGE_H * scale;
  const sizeFor = (url: string) =>
    isPasteboardUrl(url) && arena ? { w: Math.max(arena.width, slideW), h: Math.max(arena.height, slideH) } : { w: slideW, h: slideH };
  const bleeds = frames.some(isPasteboardUrl);

  return (
    <div
      ref={containerRef}
      className={className}
      style={{
        position: "relative",
        aspectRatio: "16 / 9",
        overflow: bleeds ? "visible" : "hidden",
        // The pasteboard paints the slide wherever it is panned to, and nothing around it.
        background: bleeds ? "transparent" : "#000",
      }}
    >
      {scale > 0 &&
        frames.map((url) => {
          const size = sizeFor(url);
          return (
            <iframe
              key={url}
              src={url}
              title={slideId}
              tabIndex={thumbnail ? -1 : undefined}
              sandbox="allow-scripts"
              onLoad={(event) => {
                if (url === pending) {
                  setShown(url);
                  setPending(null);
                }
                onFrameReady?.(event.currentTarget);
              }}
              style={{
                position: "absolute",
                left: (slideW - size.w) / 2,
                top: (slideH - size.h) / 2,
                width: size.w / scale,
                height: size.h / scale,
                border: 0,
                transformOrigin: "0 0",
                transform: `scale(${scale})`,
                pointerEvents: thumbnail ? "none" : "auto",
                visibility: url === shown ? "visible" : "hidden",
              }}
            />
          );
        })}
    </div>
  );
}

/** Version key for a slide of the open deck: its markup, the deck's shared styles, assets. */
export function useSlideVersion(slide: Slide): string {
  const shellHash = useApp((s) => s.deck?.shellHash ?? "");
  const assetsRev = useApp((s) => s.assetsRev);
  return `${shellHash}.${slide.hash}.${assetsRev}`;
}
