import { afterEach, describe, expect, it, vi } from "vitest";

import { cn, deckFileUrl, layoutLabel, relativeTime, slideUrl, templateSlideUrl } from "./utils";

describe("cn", () => {
  it("joins truthy classes and lets later Tailwind classes win", () => {
    expect(cn("p-1", false, null, undefined, "text-sm")).toBe("p-1 text-sm");
    expect(cn("p-1 text-xs", "p-2")).toBe("text-xs p-2");
    expect(cn({ hidden: true, block: false })).toBe("hidden");
  });
});

describe("templates", () => {
  it("builds still previews of a template's slides", () => {
    expect(templateSlideUrl("bento-grid", "stats")).toBe("/__deck/.template/bento-grid/deck.html?embed&slide=stats&static");
    expect(templateSlideUrl("my style", "#1")).toBe("/__deck/.template/my%20style/deck.html?embed&slide=%231&static");
  });

  it("names layouts after their slide ids", () => {
    expect(layoutLabel("pricing-tiers")).toBe("Pricing tiers");
    expect(layoutLabel("quote")).toBe("Quote");
    expect(layoutLabel("big_number")).toBe("Big number");
    expect(layoutLabel("---")).toBe("---");
  });
});

describe("deck URLs (browser preview)", () => {
  // Tests run without Tauri internals, i.e. like the dev browser preview.
  it("serves deck files from the dev server", () => {
    expect(deckFileUrl("talk", "deck.html")).toBe("/__deck/talk/deck.html");
  });

  it("encodes each path segment but keeps the slashes", () => {
    expect(deckFileUrl("my talk", "assets/café #1.png")).toBe("/__deck/my%20talk/assets/caf%C3%A9%20%231.png");
  });

  it("appends a query when given", () => {
    expect(deckFileUrl("talk", "deck.html", "v=1")).toBe("/__deck/talk/deck.html?v=1");
    expect(deckFileUrl("talk", "deck.html", "")).toBe("/__deck/talk/deck.html");
  });

  it("builds embedded single-slide URLs", () => {
    expect(slideUrl("talk", "intro", "abc")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=abc");
    expect(slideUrl("talk", "#2", "abc", true)).toBe("/__deck/talk/deck.html?embed&slide=%232&v=abc&static");
    expect(slideUrl("talk", "intro", "abc", true, "3")).toBe("/__deck/talk/deck.html?embed&slide=intro&v=abc&static&edit=3");
    expect(slideUrl("talk", "intro", "abc", false, undefined, true)).toBe("/__deck/talk/deck.html?embed&slide=intro&v=abc&pan");
    expect(slideUrl("talk", "intro", "abc", true, "3", true)).toBe("/__deck/talk/deck.html?embed&slide=intro&v=abc&static&pan&edit=3");
  });
});

describe("deck URLs (desktop app)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    vi.resetModules();
  });

  async function loadWith(userAgent: string) {
    vi.resetModules();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    vi.stubGlobal("navigator", { ...navigator, userAgent });
    return import("./utils");
  }

  it("uses the slop:// scheme on macOS and Linux", async () => {
    const utils = await loadWith("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
    expect(utils.isMac).toBe(true);
    expect(utils.deckFileUrl("talk", "deck.html")).toBe("slop://localhost/talk/deck.html");
    const linux = await loadWith("Mozilla/5.0 (X11; Linux x86_64)");
    expect(linux.isMac).toBe(false);
    expect(linux.deckFileUrl("talk", "deck.html")).toBe("slop://localhost/talk/deck.html");
  });

  it("uses http://slop.localhost on Windows (WebView2)", async () => {
    const utils = await loadWith("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
    expect(utils.deckFileUrl("talk", "assets/a.png")).toBe("http://slop.localhost/talk/assets/a.png");
  });
});

describe("relativeTime", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const now = new Date("2026-03-15T12:00:00Z").getTime();
  const ago = (ms: number) => relativeTime(now - ms);
  const MIN = 60_000;

  it.each([
    [0, "just now"],
    [29_000, "just now"],
    [31_000, "1m ago"],
    [59 * MIN, "59m ago"],
    [60 * MIN, "1h ago"],
    [23 * 60 * MIN, "23h ago"],
    [24 * 60 * MIN, "1d ago"],
    [29 * 24 * 60 * MIN, "29d ago"],
  ])("%i ms ago -> %s", (diff, expected) => {
    vi.useFakeTimers({ now });
    expect(ago(diff)).toBe(expected);
  });

  it("falls back to a date after a month", () => {
    vi.useFakeTimers({ now });
    const then = now - 45 * 24 * 60 * MIN;
    expect(relativeTime(then)).toBe(new Date(then).toLocaleDateString());
  });

  it("treats timestamps in the future as just now", () => {
    vi.useFakeTimers({ now });
    expect(relativeTime(now + 5 * MIN)).toBe("just now");
  });
});
