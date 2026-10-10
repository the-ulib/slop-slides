import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../lib/api";
import { deckFileUrl } from "../lib/utils";
import type { SavedPresenter, SpeechProvider, VoiceProfile } from "../lib/speech";
import { startVoiceRecording, type VoiceRecording } from "../lib/voiceRecording";
import { runProfileJob, useSpeech } from "../speechStore";
const field = "w-full rounded-md border border-border bg-background px-3 py-2 text-sm disabled:opacity-40";
const primary = "rounded-md bg-primary px-4 py-2 text-primary-foreground disabled:opacity-40";
const passages = {
  en: "Good presentations make ideas easy to follow. I speak clearly, leave room for important points, and guide the audience through one thought at a time. This short recording will help me use my voice again in future presentations.",
  de: "Gute Präsentationen machen Ideen leicht verständlich. Ich spreche klar, lasse wichtigen Punkten genügend Raum und führe das Publikum Schritt für Schritt durch meine Gedanken. Mit dieser kurzen Aufnahme kann ich meine Stimme in zukünftigen Präsentationen wiederverwenden.",
};
export function VoiceSetup({ provider, initialLanguage, replace, onClose, onSaved }: { provider: SpeechProvider; initialLanguage: "en" | "de"; replace?: SavedPresenter; onClose: () => void; onSaved: (profile: VoiceProfile) => void }) {
  const speech = useSpeech(); const [preparing, setPreparing] = useState(false); const busy = !!speech.job || preparing;
  const [name, setName] = useState(replace?.name ?? "My voice");
  const [language, setLanguage] = useState<"en" | "de">(replace?.referenceLanguage ?? initialLanguage);
  const [transcript, setTranscript] = useState(""); const [reference, setReference] = useState<string | null>(null);
  const [referenceLabel, setReferenceLabel] = useState("");
  const [recording, setRecording] = useState(false); const [requestingMic, setRequestingMic] = useState(false); const [seconds, setSeconds] = useState(0);
  const [authorized, setAuthorized] = useState(false); const [draft, setDraft] = useState<VoiceProfile | null>(null);
  const [previewLanguage, setPreviewLanguage] = useState<"en" | "de">(language); const [previews, setPreviews] = useState<Record<string, string>>({}); const [heard, setHeard] = useState<string[]>([]);
  const [makeDefault, setMakeDefault] = useState(false); const [error, setError] = useState<string | null>(null); const [saving, setSaving] = useState(false);
  const capture = useRef<VoiceRecording | null>(null); const tempRecording = useRef<string | null>(null); const token = useRef<string | null>(null); const saved = useRef(false); const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; capture.current?.cancel(); if (tempRecording.current) void api.releaseVoiceRecording(tempRecording.current); if (token.current && !saved.current) void api.voiceProfileAction(provider.id, "discard", token.current); }; }, [provider.id]);
  useEffect(() => { if (!recording) return; const timer = setInterval(() => setSeconds((n) => n + 1), 1000); return () => clearInterval(timer); }, [recording]);
  const attempt = async (run: () => Promise<void>) => { setError(null); try { await run(); } catch (e) { if (alive.current) setError(errorMessage(e)); } };
  const stopRecording = async () => {
    const current = capture.current; capture.current = null; setRecording(false);
    if (!current) return;
    await attempt(async () => {
      const wave = await current.stop();
      const file = await api.stageVoiceRecording(Array.from(wave));
      if (!alive.current) { await api.releaseVoiceRecording(file.id); return; }
      if (tempRecording.current) await api.releaseVoiceRecording(tempRecording.current);
      tempRecording.current = file.id; setReferenceLabel("Microphone recording"); setReference(file.path);
    });
  };
  const record = async () => {
    setRequestingMic(true); setError(null); setReference(null); setSeconds(0);
    try {
      const current = await startVoiceRecording(() => { void stopRecording(); });
      if (!alive.current) { current.cancel(); return; }
      capture.current = current; setRecording(true); setTranscript(passages[language]);
    } catch (e) { if (alive.current) setError(`${errorMessage(e)} You can import a WAV recording instead.`); }
    finally { if (alive.current) setRequestingMic(false); }
  };
  const preview = async (profile: VoiceProfile, lang: "en" | "de") => {
    await runProfileJob("profile-preview", (id) => api.previewVoiceProfile(id, provider.id, profile.revision, lang));
    if (alive.current) { setPreviewLanguage(lang); setPreviews((p) => ({ ...p, [lang]: deckFileUrl(".voice", `${profile.revision}/${lang}.wav`) + `?v=${crypto.randomUUID()}` })); setHeard((h) => h.filter((l) => l !== lang)); }
  };
  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onKeyDown={(e) => { if (e.key === "Escape" && !busy && !saving && !recording && !requestingMic) onClose(); }}>
    <div role="dialog" aria-modal="true" aria-labelledby="voice-setup-title" className="max-h-[90vh] w-full max-w-xl overflow-y-auto rounded-xl border border-border bg-background p-6 text-sm shadow-xl">
      <div className="flex items-center justify-between gap-3"><h2 id="voice-setup-title" className="text-lg font-semibold">{replace ? "Replace voice recording" : "Create voice from recording"}</h2><button aria-label="Close voice setup" disabled={busy || saving || recording || requestingMic} onClick={onClose}>✕</button></div>
      <p className="mt-2 text-muted-foreground">{draft ? "2 · Listen, then save" : "1 · Import or record"} · {provider.label}</p>
      <p className="mt-3 text-muted-foreground">Your recording and voice profile stay on this device. Once saved, the presenter works across decks without recording again.</p>
      {!draft ? <div className="mt-4 space-y-4">
        <p className="text-muted-foreground">Choose a reference audio file to create a reusable presenter. Use 10–20 seconds of clear speech in your primary narration language, without music or other speakers. Accepted length: 3–30 seconds. Imported files must be PCM16 WAV.</p>
        <div className="flex items-center gap-3"><button className={`${primary} flex-1`} disabled={busy || recording || requestingMic} onClick={() => void attempt(async () => { const path = await open({ multiple: false, title: "Import reference audio (WAV)", filters: [{ name: "WAV audio", extensions: ["wav"] }] }); if (typeof path === "string") { const file = await api.importVoiceRecording(path); if (!alive.current) { await api.releaseVoiceRecording(file.id); return; } if (tempRecording.current) await api.releaseVoiceRecording(tempRecording.current); tempRecording.current = file.id; setReferenceLabel(path.split(/[\\/]/).pop() ?? "Imported recording"); setReference(file.path); setTranscript(""); } })}>Import reference audio…</button><button className="rounded-md border border-border px-3 py-2 disabled:opacity-40" disabled={busy || requestingMic} onClick={() => recording ? void stopRecording() : void record()}>{recording ? `Stop recording · ${seconds}s` : requestingMic ? "Requesting microphone…" : "Record here"}</button></div>
        <label className="block">Recording language<select aria-label="Recording language" className={`${field} mt-1`} value={language} disabled={busy || recording || requestingMic} onChange={(e) => { setLanguage(e.target.value as "en" | "de"); }}><option value="en">English</option><option value="de">German</option></select></label>
        {recording && <div className="rounded-md bg-accent p-3"><p className="mb-2 font-medium">Read this passage naturally</p><p>{passages[language]}</p></div>}
        {reference && <p role="status" className="text-muted-foreground">Recording ready: {referenceLabel}</p>}
        {reference && tempRecording.current && <audio aria-label="Reference recording" controls preload="metadata" className="w-full" src={deckFileUrl(".recording", `${tempRecording.current}.wav`)} />}
        {(reference || recording) && <>
        <label className="block">Presenter name<input aria-label="Presenter name" className={`${field} mt-1`} value={name} maxLength={80} disabled={busy || recording || !!replace} onChange={(e) => setName(e.target.value)} /></label>
        <label className="block">Exact spoken words<textarea aria-label="Voice recording transcript" className={`${field} mt-1 min-h-28`} value={transcript} maxLength={4096} disabled={busy || recording} placeholder="Enter exactly what is spoken in the recording…" onChange={(e) => setTranscript(e.target.value)} /></label>
        <label className="flex items-start gap-2"><input type="checkbox" checked={authorized} disabled={busy || recording} onChange={(e) => setAuthorized(e.target.checked)} />I am the speaker or have permission to create and use this voice.</label>
        {!provider.cloneReady && <section className="rounded-md border border-border p-3"><h3 className="font-medium">One-time voice model setup</h3><p className="mt-1 text-muted-foreground">Creating the preview downloads a {((provider.cloneSetup?.totalBytes ?? 0) / 1e9).toFixed(2)} GB local voice model. {provider.cloneSetup?.detail}</p>{provider.cloneSetup?.importTitle && <details className="mt-2"><summary className="cursor-pointer text-muted-foreground">Already have the model files?</summary><button className="mt-2 underline disabled:opacity-40" disabled={busy} onClick={() => void attempt(async () => { const path = await open({ directory: true, title: provider.cloneSetup!.importTitle! }); if (typeof path === "string") await runProfileJob("setup", (id) => api.installCloningPack(id, path, provider.id)); })}>Import existing model folder…</button></details>}</section>}
        <button className={primary} disabled={busy || recording || requestingMic || !name.trim() || !reference || !transcript.trim() || !authorized} onClick={() => void attempt(async () => {
          setPreparing(true);
          try {
          if (!provider.cloneReady) await runProfileJob("setup", (id) => api.installCloningPack(id, null, provider.id));
          if (!alive.current) return;
          const profile = await runProfileJob("profile", (id) => api.createVoiceProfile(id, provider.id, { name, language, transcript, reference: reference!, authorized }));
          token.current = profile.revision;
          if (!alive.current) { await api.voiceProfileAction(provider.id, "discard", profile.revision); return; }
          setDraft(profile); setPreviewLanguage(language); await preview(profile, language);
          } finally { if (alive.current) setPreparing(false); }
        })}>{provider.cloneReady ? "Create voice preview" : "Download model & create preview"}</button>
        </>}
      </div> : <div className="mt-4 space-y-4">
        <p className="font-medium">{draft.name}</p><p className="text-muted-foreground">After saving, choose this voice from the Presenter menu in any deck.</p><p className="text-muted-foreground">Listen for voice likeness, pronunciation and pacing. A cross-language accent may remain.</p>
        <label className="block">Preview language<select aria-label="Voice preview language" className={`${field} mt-1`} value={previewLanguage} disabled={busy || saving} onChange={(e) => setPreviewLanguage(e.target.value as "en" | "de")}><option value="en">English</option><option value="de">German</option></select></label>
        <button className={field} disabled={busy || saving} onClick={() => void attempt(() => preview(draft, previewLanguage))}>{previews[previewLanguage] ? "Generate preview again" : "Generate preview"}</button>
        {previews[previewLanguage] && <audio key={previews[previewLanguage]} aria-label="Voice setup preview" controls preload="metadata" src={previews[previewLanguage]} className="w-full" onTimeUpdate={(e) => { if (e.currentTarget.currentTime >= 1) setHeard((h) => h.includes(previewLanguage) ? h : [...h, previewLanguage]); }} onEnded={() => setHeard((h) => h.includes(previewLanguage) ? h : [...h, previewLanguage])} onError={() => setError("The preview could not be played. Generate it again.")} />}
        {!heard.includes(draft.referenceLanguage) && <p className="text-muted-foreground">Play the {draft.referenceLanguage === "de" ? "German" : "English"} preview before saving.</p>}
        <label className="flex gap-2"><input type="checkbox" checked={makeDefault} disabled={busy || saving} onChange={(e) => setMakeDefault(e.target.checked)} />Use as default presenter for new decks</label>
        {replace && <p className="text-muted-foreground">Replaces {replace.name} for future generation. Existing recordings stay available; affected narration will need a new take.</p>}
        <div className="flex gap-3"><button className={primary} disabled={busy || saving || !heard.includes(draft.referenceLanguage)} onClick={() => void attempt(async () => {
          setSaving(true);
          try {
            const profile = await api.voiceProfileAction(provider.id, "save", draft.revision, replace?.id ?? null);
            if (!profile) throw new Error("Presenter could not be saved."); saved.current = true;
            if (makeDefault) { try { await api.setDefaultPresenter({ providerId: provider.id, presenterId: profile.id }); } catch (e) { useSpeech.setState({ error: `Presenter saved; default could not be set: ${errorMessage(e)}` }); } }
            await speech.refresh(); onSaved(profile);
          } finally { setSaving(false); }
        })}>Save presenter</button><button className="underline disabled:opacity-40" disabled={busy || saving} onClick={() => void attempt(async () => { await api.voiceProfileAction(provider.id, "discard", draft.revision); token.current = null; setDraft(null); setPreviews({}); setHeard([]); })}>Choose another recording</button></div>
      </div>}
      {speech.job && <div className="mt-4 rounded-md bg-accent p-3" role="status"><p>{speech.job.detail}</p>{speech.job.total > 0 && <progress className="mt-2 w-full" max={speech.job.total} value={speech.job.completed} />}<button className="mt-2 underline" disabled={speech.cancelling} onClick={() => void speech.cancel()}>{speech.cancelling ? "Cancelling…" : "Cancel"}</button></div>}
      {error && <p role="alert" className="mt-4 rounded-md bg-amber-500/10 p-3">{error}</p>}
    </div>
  </div>;
}
