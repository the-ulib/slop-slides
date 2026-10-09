/** A stand-in for a 2D canvas context that records what is drawn, for jsdom, which has no canvas. */
export function fakeCanvas2D(canvas: { width: number; height: number } = { width: 0, height: 0 }) {
  const ctx = {
    canvas,
    lineWidth: 1,
    lineCap: "butt" as CanvasLineCap,
    strokeStyle: "#000" as string | CanvasGradient | CanvasPattern,
    shadowBlur: 0,
    shadowColor: "transparent",
    /** Width of each line stroked since the last clear. */
    strokes: [] as number[],
    /** Shadow of each image copied in since the last clear. */
    images: [] as { shadowBlur: number; shadowColor: string }[],
    /** The last transform set, as [a, b, c, d, e, f]. */
    transform: [1, 0, 0, 1, 0, 0] as number[],
    setTransform(...matrix: unknown[]) {
      ctx.transform = matrix as number[];
    },
    clearRect() {
      ctx.strokes = [];
      ctx.images = [];
    },
    beginPath() {},
    moveTo() {},
    quadraticCurveTo() {},
    stroke() {
      ctx.strokes.push(ctx.lineWidth);
    },
    drawImage() {
      ctx.images.push({ shadowBlur: ctx.shadowBlur, shadowColor: ctx.shadowColor });
    },
  };
  return ctx;
}
