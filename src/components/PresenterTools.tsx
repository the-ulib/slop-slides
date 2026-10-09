import { Eraser, Highlighter, MousePointer2, PenLine, Trash2, Undo2, Wand2, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { flushSync } from "react-dom";

import {
  DEFAULT_COLORS,
  INK_COLORS,
  INK_STYLE,
  isDot,
  LASER_GLOW,
  LASER_RGB,
  LASER_SIZE,
  paintTrails,
  pruneTrails,
  strokePath,
  TOOL_KEYS,
  toFraction,
  toPixels,
  type InkTool,
  type Stroke,
  type Tool,
  type TrailPoint,
  visibleArea,
} from "../lib/ink";
import { cn } from "../lib/utils";

/** How long the toolbar stays visible after a keyboard shortcut changes something. */
const PEEK_MS = 1500;
const laserShadow = LASER_GLOW.map((g) => `0 0 ${g.blur}px ${g.spread}px rgb(${LASER_RGB} / ${g.opacity})`).join(", ");

export interface Annotations {
  tool: Tool;
  setTool: (tool: Tool) => void;
  colors: Record<InkTool, string>;
  setColor: (color: string) => void;
  /** Strokes on the slide being shown. */
  strokes: Stroke[];
  addStroke: (stroke: Stroke) => void;
  erase: (index: number) => void;
  undo: () => void;
  clear: () => void;
  /** Handles a presenter shortcut; returns false for keys that are not one. */
  handleKey: (key: string, mod: boolean) => boolean;
  /** Bumped by keyboard shortcuts so the toolbar can briefly show what changed. */
  peek: number;
}

/** Where ink lives, per slide key; by default in the hook's own state. */
export interface InkStore {
  ink: Record<string, Stroke[]>;
  setInk: (update: (all: Record<string, Stroke[]>) => Record<string, Stroke[]>) => void;
}

/** Laser, pen, highlighter and eraser state, with ink kept per slide for the whole show. */
export function useAnnotations(slideKey: string, store?: InkStore): Annotations {
  const [tool, setToolState] = useState<Tool>("pointer");
  const [colors, setColors] = useState(DEFAULT_COLORS);
  const [localInk, setLocalInk] = useState<Record<string, Stroke[]>>({});
  const { ink, setInk } = store ?? { ink: localInk, setInk: setLocalInk };
  const [peek, setPeek] = useState(0);
  const strokes = ink[slideKey] ?? [];

  const update = (fn: (strokes: Stroke[]) => Stroke[]) =>
    setInk((all) => ({ ...all, [slideKey]: fn(all[slideKey] ?? []) }));

  const setTool = (next: Tool) => setToolState(next);
  const setColor = (color: string) => {
    const target: InkTool = tool === "highlighter" ? "highlighter" : "pen";
    setColors((c) => ({ ...c, [target]: color }));
    if (tool !== "pen" && tool !== "highlighter") setToolState("pen");
  };
  const addStroke = (stroke: Stroke) => update((s) => [...s, stroke]);
  const erase = (index: number) => update((s) => s.filter((_, i) => i !== index));
  const undo = () => update((s) => s.slice(0, -1));
  const clear = () => update(() => []);

  const handleKey = (key: string, mod: boolean): boolean => {
    const lower = key.toLowerCase();
    if (mod) {
      if (lower !== "z") return false;
      undo();
    } else if (key === "Escape") {
      if (tool === "pointer") return false;
      setToolState("pointer");
    } else if (TOOL_KEYS[lower]) {
      const next = TOOL_KEYS[lower];
      setToolState(tool === next ? "pointer" : next);
    } else if (lower === "c") {
      clear();
    } else {
      return false;
    }
    setPeek((n) => n + 1);
    return true;
  };

  return { tool, setTool, colors, setColor, strokes, addStroke, erase, undo, clear, handleKey, peek };
}

/**
 * Transparent layer over the slide that draws ink, the laser dot and, while the laser is
 * dragged, its trail, which stays a moment and then thins away. `zoom` is how far the
 * layer has been enlarged along with a zoomed slide: the ink is drawn at that size, as
 * vectors, so it stays sharp, and thickens with the slide.
 */
export function AnnotationLayer({ annotations, zoom = 1 }: { annotations: Annotations; zoom?: number }) {
  const { tool, colors, strokes, addStroke, erase } = annotations;
  const layerRef = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState<Stroke | null>(null);
  const [laser, setLaser] = useState<{ x: number; y: number } | null>(null);
  const [trails, setTrails] = useState<TrailPoint[][]>([]);
  const [now, setNow] = useState(() => performance.now());
  const tracing = useRef(false);
  const trailRef = useRef<HTMLCanvasElement>(null);
  const trailCore = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const active = tool !== "pointer";

  // Ink is stored in fractions of the layer but drawn in pixels, so strokes keep their width
  // (times the zoom) as the layer resizes.
  useLayoutEffect(() => {
    const el = layerRef.current!;
    const measure = () => {
      const rect = el.getBoundingClientRect();
      setSize({ width: el.offsetWidth || rect.width, height: el.offsetHeight || rect.height });
    };
    measure();
    // Redraw before the resized layer is painted: a zoom resizes it at once, and ink drawn a
    // frame later at the old size flickers.
    const observer = new ResizeObserver(() => flushSync(measure));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setDraft(null);
    setLaser(null);
    setTrails([]);
    tracing.current = false;
  }, [tool]);

  // Animate trails until the last one has faded.
  const fading = trails.length > 0;
  useEffect(() => {
    if (!fading) return;
    let frame = requestAnimationFrame(function tick() {
      const time = performance.now();
      setNow(time);
      setTrails((t) => pruneTrails(t, time));
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [fading]);

  // The trail is painted on a canvas rather than drawn as glowing SVG: WebKit leaves stale,
  // clipped pieces of a filtered SVG's glow behind as it changes.
  useLayoutEffect(() => {
    const canvas = trailRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const scale = window.devicePixelRatio || 1;
    const core = (trailCore.current ??= document.createElement("canvas"));
    const coreCtx = core.getContext("2d");
    if (!coreCtx) return;
    // Only the part of the layer on screen gets a canvas: zoomed in, the layer is many times
    // the screen's size, and glowing a canvas that big every frame makes the trail crawl.
    const { left, top } = layerRef.current!.getBoundingClientRect();
    const area = visibleArea(
      { left, top, width: size.width, height: size.height },
      { width: window.innerWidth, height: window.innerHeight },
    );
    Object.assign(canvas.style, {
      left: `${area.x}px`,
      top: `${area.y}px`,
      width: `${area.width}px`,
      height: `${area.height}px`,
    });
    for (const c of [canvas, core]) {
      const [w, h] = [Math.round(area.width * scale), Math.round(area.height * scale)];
      if (c.width !== w) c.width = w;
      if (c.height !== h) c.height = h;
    }
    paintTrails(ctx, coreCtx, trails, now, size.width, size.height, scale, area);
  }, [trails, now, size]);

  const at = (event: ReactPointerEvent) =>
    toFraction(event.clientX, event.clientY, layerRef.current!.getBoundingClientRect());

  const eraseUnder = (event: ReactPointerEvent) => {
    const hit = (event.target as Element).closest?.("[data-stroke]");
    if (hit) erase(Number(hit.getAttribute("data-stroke")));
  };

  const tracePoint = (event: ReactPointerEvent): TrailPoint => {
    const [x, y] = at(event);
    return { x, y, t: performance.now() };
  };

  /**
   * Where the pointer went since the last move, as fractions of the layer: the browser hands
   * out one move per frame, but keeps the samples in between, which a fast stroke needs.
   */
  const movedThrough = (event: ReactPointerEvent): [number, number][] => {
    const rect = layerRef.current!.getBoundingClientRect();
    const samples = event.nativeEvent.getCoalescedEvents?.() ?? [];
    return (samples.length ? samples : [event]).map((e) => toFraction(e.clientX, e.clientY, rect));
  };

  const onPointerDown = (event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    if (tool === "laser") {
      event.currentTarget.setPointerCapture?.(event.pointerId);
      tracing.current = true;
      const point = tracePoint(event);
      setNow(point.t);
      setTrails((t) => [...t, [point]]);
      return;
    }
    if (tool === "eraser") eraseUnder(event);
    if (tool !== "pen" && tool !== "highlighter") return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDraft({ tool, color: colors[tool], points: [at(event)] });
  };

  const onPointerMove = (event: ReactPointerEvent) => {
    if (tool === "laser") {
      const rect = layerRef.current!.getBoundingClientRect();
      setLaser({ x: event.clientX - rect.left, y: event.clientY - rect.top });
      if (tracing.current && event.buttons & 1) {
        const t = performance.now();
        const points = movedThrough(event).map(([x, y]) => ({ x, y, t }));
        setNow(t);
        // The trail being drawn may have faded away entirely while the pointer rested.
        setTrails((ts) => (ts.length ? [...ts.slice(0, -1), [...ts[ts.length - 1]!, ...points]] : [points]));
      }
    } else if (tool === "eraser" && event.buttons & 1) {
      eraseUnder(event);
    } else if (draft) {
      const points = movedThrough(event);
      setDraft((d) => d && { ...d, points: [...d.points, ...points] });
    }
  };

  const finish = () => {
    if (draft) addStroke(draft);
    setDraft(null);
    tracing.current = false;
  };

  return (
    <div
      ref={layerRef}
      data-testid="annotation-layer"
      className="absolute inset-0"
      style={{
        pointerEvents: active ? "auto" : "none",
        cursor: tool === "laser" ? "none" : tool === "eraser" ? "cell" : active ? "crosshair" : undefined,
        touchAction: "none",
      }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={finish}
      onPointerCancel={finish}
      onPointerLeave={() => {
        finish();
        setLaser(null);
      }}
    >
      <svg className="absolute inset-0 size-full">
        {[...strokes, ...(draft ? [draft] : [])].map((stroke, i) => {
          const { opacity } = INK_STYLE[stroke.tool];
          const width = INK_STYLE[stroke.tool].width * zoom;
          const points = toPixels(stroke.points, size.width, size.height);
          const common = {
            "data-stroke": i < strokes.length ? i : undefined,
            style: { pointerEvents: tool === "eraser" ? ("visiblePainted" as const) : ("none" as const) },
          };
          if (isDot(points)) {
            const [[cx, cy]] = points as [[number, number]];
            return <circle key={i} {...common} cx={cx} cy={cy} r={width / 2} fill={stroke.color} fillOpacity={opacity} />;
          }
          return (
            <path
              key={i}
              {...common}
              d={strokePath(points)}
              fill="none"
              stroke={stroke.color}
              strokeWidth={width}
              strokeOpacity={opacity}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          );
        })}
      </svg>
      {trails.length > 0 && (
        <canvas ref={trailRef} data-testid="laser-trail" className="pointer-events-none absolute" />
      )}
      {laser && (
        <div
          data-testid="laser"
          className="pointer-events-none absolute -translate-1/2 rounded-full bg-red-500"
          style={{ left: laser.x, top: laser.y, width: LASER_SIZE, height: LASER_SIZE, boxShadow: laserShadow }}
        />
      )}
    </div>
  );
}

/** Corner controls hide until the mouse enters their corner; they show at the start and briefly per `peek`. */
export function useReveal(peek: number) {
  const [hovered, setHovered] = useState(false);
  const [peeking, setPeeking] = useState(true);

  useEffect(() => {
    setPeeking(true);
    const timer = setTimeout(() => setPeeking(false), PEEK_MS);
    return () => clearTimeout(timer);
  }, [peek]);

  return {
    visible: hovered || peeking,
    zoneProps: { onMouseEnter: () => setHovered(true), onMouseLeave: () => setHovered(false) },
  };
}

/**
 * Tool palette in the bottom-left corner. Hidden while presenting; appears when the mouse
 * moves into the corner, and briefly after a keyboard shortcut.
 */
export function PresenterToolbar({ annotations, onExit }: { annotations: Annotations; onExit: () => void }) {
  const { tool, setTool, colors, setColor, strokes, undo, clear, peek } = annotations;
  const { visible, zoneProps } = useReveal(peek);
  const inking = tool === "pen" || tool === "highlighter";

  return (
    <div
      data-testid="presenter-toolbar-zone"
      className="absolute bottom-0 left-0 z-10 p-4"
      {...zoneProps}
    >
      <div
        role="toolbar"
        aria-label="Presenter tools"
        data-visible={visible}
        className={cn(
          "flex items-center gap-1 rounded-xl border border-white/10 bg-neutral-900/85 p-1.5 text-white shadow-lg backdrop-blur transition-opacity duration-200",
          visible ? "opacity-100" : "opacity-0",
        )}
      >
        <ToolButton label="Pointer (Esc)" active={tool === "pointer"} onClick={() => setTool("pointer")}>
          <MousePointer2 />
        </ToolButton>
        <ToolButton label="Laser pointer (L)" active={tool === "laser"} onClick={() => setTool("laser")}>
          <Wand2 />
        </ToolButton>
        <ToolButton label="Pen (P)" active={tool === "pen"} onClick={() => setTool("pen")}>
          <PenLine />
        </ToolButton>
        <ToolButton label="Highlighter (H)" active={tool === "highlighter"} onClick={() => setTool("highlighter")}>
          <Highlighter />
        </ToolButton>
        <ToolButton label="Eraser (E)" active={tool === "eraser"} onClick={() => setTool("eraser")}>
          <Eraser />
        </ToolButton>
        {inking && (
          <div className="mx-1 flex items-center gap-1">
            {INK_COLORS.map((color) => (
              <button
                key={color}
                type="button"
                aria-label={`Color ${color}`}
                aria-pressed={colors[tool] === color}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => setColor(color)}
                className={cn(
                  "size-5 rounded-full border border-white/30",
                  colors[tool] === color && "ring-2 ring-white ring-offset-1 ring-offset-neutral-900",
                )}
                style={{ background: color }}
              />
            ))}
          </div>
        )}
        <div className="mx-1 h-5 w-px bg-white/15" />
        <ToolButton label="Undo (⌘Z)" disabled={strokes.length === 0} onClick={undo}>
          <Undo2 />
        </ToolButton>
        <ToolButton label="Clear slide (C)" disabled={strokes.length === 0} onClick={clear}>
          <Trash2 />
        </ToolButton>
        <ToolButton label="End show (Esc)" onClick={onExit}>
          <X />
        </ToolButton>
      </div>
    </div>
  );
}

function ToolButton(props: {
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      title={props.label}
      aria-pressed={props.active}
      disabled={props.disabled}
      // Keep keyboard focus where it was, so Space still advances the slide instead of re-clicking.
      onMouseDown={(event) => event.preventDefault()}
      onClick={props.onClick}
      className={cn(
        "flex size-8 items-center justify-center rounded-lg text-white/80 hover:bg-white/10 hover:text-white disabled:pointer-events-none disabled:opacity-35 [&_svg]:size-4",
        props.active && "bg-white/20 text-white",
      )}
    >
      {props.children}
    </button>
  );
}
