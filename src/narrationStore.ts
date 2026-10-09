import { create } from "zustand";
import { api, errorMessage } from "./lib/api";
import { editedManifest, emptyNarration, type NarrationDocument, type NarrationEdits, type NarrationLanguage, type NarrationSettings, type SlideNarration } from "./lib/narration";

interface NarrationState {
  deckId: string | null;
  document: NarrationDocument | null;
  edits: NarrationEdits;
  languageEdit: NarrationLanguage | null;
  settingsEdits: NarrationSettings;
  setPresenter: (id: string, name: string) => void;
  setPace: (pace: number) => void;
  saving: boolean;
  error: string | null;
  conflict: NarrationDocument | null;
  load: (id: string | null) => Promise<void>;
  refresh: () => Promise<void>;
  edit: (id: string, patch: Partial<SlideNarration>) => void;
  setLanguage: (language: NarrationLanguage) => void;
  save: () => Promise<boolean>;
  resolve: (keepLocal: boolean) => Promise<void>;
}
let timer: ReturnType<typeof setTimeout> | undefined;
let queue: Promise<boolean> = Promise.resolve(true);
let generation = 0;
let refreshRun = 0;
const dirty = (s: NarrationState) => Object.keys(s.edits).length > 0 || s.languageEdit !== null || Object.keys(s.settingsEdits).length > 0;
function schedule() {
  clearTimeout(timer);
  timer = setTimeout(() => void useNarration.getState().save(), 400);
}

export const useNarration = create<NarrationState>((set, get) => ({
  deckId: null, document: null, edits: {}, languageEdit: null, settingsEdits: {}, saving: false, error: null, conflict: null,
  load: async (id) => {
    clearTimeout(timer);
    const run = ++generation;
    ++refreshRun;
    set({ deckId: id, document: null, edits: {}, languageEdit: null, settingsEdits: {}, saving: false, error: null, conflict: null });
    if (!id) return;
    try {
      const document = await api.loadNarration(id);
      if (run === generation) set({ document: document ?? emptyNarration() });
    } catch (e) {
      if (run === generation) set({ error: errorMessage(e) });
    }
  },
  refresh: async () => {
    // A watcher event may arrive before our own save response. Compare against that response.
    await queue;
    const { deckId } = get();
    if (!deckId) return;
    const run = ++refreshRun;
    const gen = generation;
    const knownVersion = get().document?.version;
    try {
      const document = await api.loadNarration(deckId) ?? emptyNarration();
      if (gen !== generation || run !== refreshRun) return;
      const current = get();
      if (current.document?.version !== knownVersion) { void get().refresh(); return; }
      if (document.version === current.document?.version) {
        if (!current.conflict && !dirty(current)) set({ error: null });
        return;
      }
      if (dirty(current)) set({ conflict: document, error: "Narration changed on disk. Choose which edits to keep." });
      else set({ document, conflict: null, error: null });
    } catch (e) {
      if (gen === generation && run === refreshRun) set({ error: errorMessage(e) });
    }
  },
  edit: (id, patch) => {
    if (!get().document || id.startsWith("#")) return;
    set((s) => ({ edits: { ...s.edits, [id]: { ...s.edits[id], ...patch } } }));
    if (!get().conflict && !get().error) schedule();
  },
  setLanguage: (languageEdit) => {
    if (!get().document) return;
    set({ languageEdit });
    if (!get().conflict && !get().error) schedule();
  },
  setPresenter: (id, name) => { set({ settingsEdits: { ...get().settingsEdits, presenterId: id, presenterNameSnapshot: name } }); if (!get().conflict && !get().error) schedule(); },
  setPace: (pace) => { if (!Number.isFinite(pace) || pace < 0.9 || pace > 1.25) return; set({ settingsEdits: { ...get().settingsEdits, pace } }); if (!get().conflict && !get().error) schedule(); },
  save: () => {
    clearTimeout(timer);
    const gen = generation;
    const save = async (): Promise<boolean> => {
      if (gen !== generation) return false;
      const before = get();
      if (before.conflict) return false;
      if (!dirty(before)) return true;
      const { deckId, document, edits, languageEdit, settingsEdits } = before;
      if (!deckId || !document) return false;
      set({ saving: true });
      try {
        const next = await api.saveNarration(deckId, editedManifest(document.manifest, edits, languageEdit, settingsEdits), document.version);
        if (gen !== generation) return false;
        set((s) => ({
          document: next,
          edits: Object.fromEntries(Object.entries(s.edits).filter(([id, patch]) => patch !== edits[id])),
          languageEdit: s.languageEdit === languageEdit ? null : s.languageEdit,
          settingsEdits: s.settingsEdits === settingsEdits ? {} : s.settingsEdits,
          saving: false, error: null,
        }));
        // Drain edits made while this request was in flight before allowing a deck switch.
        if (dirty(get())) return save();
        return true;
      } catch (e) {
        if (gen !== generation) return false;
        set({ saving: false, error: errorMessage(e) });
        // Read directly here: refresh waits for this queue and would deadlock.
        try {
          const incoming = await api.loadNarration(deckId);
          if (gen === generation && incoming && incoming.version !== document.version) set({ conflict: incoming });
        } catch { /* Keep all local edits if the file cannot be read. */ }
        return false;
      }
    };
    queue = queue.then(save, save);
    return queue;
  },
  resolve: async (keepLocal) => {
    const { conflict } = get();
    if (!conflict) return;
    set({ document: conflict, conflict: null, error: null, ...(keepLocal ? {} : { edits: {}, languageEdit: null, settingsEdits: {} }) });
    if (keepLocal) await get().save();
  },
}));
