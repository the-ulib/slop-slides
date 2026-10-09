// Dev-only: when the UI runs in a plain browser, fake the Tauri IPC with read-only data
// served by dev/browserPreview.ts. Never loaded inside the desktop app.
import { emptyNarration, type NarrationDocument, type NarrationManifest } from "./narration";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

interface RawDeck {
  id: string;
  title: string;
  path: string;
  slides: { id: string; hash: string; hidden: boolean; locked: boolean; moved: boolean }[];
  sections: { index: number; title: string; before: number }[];
  shellHash: string;
  updatedMs: number;
}

export function installBrowserMock() {
  mockWindows("main");
  const decks = async () => (await (await fetch("/__api/decks")).json()) as RawDeck[];
  const deck = async (id: unknown) => {
    const found = (await decks()).find((d) => d.id === id);
    if (!found) throw new Error(`deck not found: ${String(id)}`);
    return { id: found.id, title: found.title, path: found.path, slides: found.slides, sections: found.sections, shellHash: found.shellHash };
  };
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "list_decks":
          return (await decks()).map((d) => ({
            id: d.id,
            title: d.title,
            slideCount: d.slides.length,
            firstSlide: d.slides[0]?.id ?? null,
            updatedMs: d.updatedMs,
          }));
        case "open_deck":
        case "load_deck":
          return deck(a.id);
        case "list_providers":
          return [
            {
              id: "claude",
              installed: true,
              path: "/mock/claude",
              models: [
                { id: "claude-opus-5-5", label: "Claude Opus 5.5", isDefault: true, efforts: ["low", "medium", "high", "xhigh", "max"], defaultEffort: "medium", contextWindows: ["200k", "1m"], defaultContextWindow: "1m" },
                { id: "claude-sonnet-5", label: "Claude Sonnet 5", isDefault: false, efforts: ["low", "medium", "high", "xhigh", "max"], defaultEffort: "medium", contextWindows: ["200k", "1m"], defaultContextWindow: "200k" },
              ],
              error: null,
            },
            { id: "codex", installed: false, path: null, models: [], error: null },
            {
              id: "copilot",
              installed: true,
              path: "/mock/copilot",
              models: [
                { id: "gpt-6-astra", label: "GPT-6 Astra", isDefault: false, efforts: ["low", "medium", "high"], defaultEffort: "medium", contextWindows: [], defaultContextWindow: null },
                { id: "claude-sonnet-5", label: "Claude Sonnet 5", isDefault: false, efforts: [], defaultEffort: null, contextWindows: [], defaultContextWindow: null },
              ],
              error: null,
            },
          ];
        case "agent_running":
          return false;
        case "list_templates":
          return (await fetch("/__api/templates")).json();
        case "stage_template":
          return `.slopslide/templates/${String(a.template)}.html`;
        case "codex_permission_modes":
          throw new Error("Codex permissions require the desktop app.");
        case "lint_deck":
          return [];
        case "save_deck_source":
          throw new Error("Saving is not available in the browser preview.");
        case "load_narration":
          return JSON.parse(localStorage.getItem(`mock-narration-${String(a.id)}`) ?? "null") ?? emptyNarration();
        case "save_narration": {
          const key = `mock-narration-${String(a.id)}`;
          const previous: NarrationDocument = JSON.parse(localStorage.getItem(key) ?? "null") ?? emptyNarration();
          if (previous.version !== a.base) throw new Error("Narration changed in the browser preview.");
          const next = { manifest: { ...(a.manifest as NarrationManifest), revision: previous.manifest.revision + 1 }, version: crypto.randomUUID() };
          localStorage.setItem(key, JSON.stringify(next));
          return next;
        }
        case "save_asset":
          return `assets/${String(a.name)}`;
        case "load_chat":
          return JSON.parse(localStorage.getItem(`mock-chat-${String(a.id)}`) ?? "null");
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
