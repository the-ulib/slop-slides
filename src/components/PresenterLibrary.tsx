import { useState } from "react";
import { api, errorMessage } from "../lib/api";
import type { SavedPresenter } from "../lib/speech";
import { useSpeech } from "../speechStore";
function PresenterRow({ profile, onReplace }: { profile: SavedPresenter; onReplace: (profile: SavedPresenter) => void }) {
  const speech = useSpeech(); const [name, setName] = useState(profile.name); const [removing, setRemoving] = useState(false); const [working, setWorking] = useState(false);
  const busy = working || !!speech.job;
  const selected = speech.status?.defaultPresenter?.providerId === profile.providerId && speech.status.defaultPresenter.presenterId === profile.id;
  const run = async (action: () => Promise<unknown>) => { setWorking(true); try { await action(); await speech.refresh(); } catch (e) { useSpeech.setState({ error: errorMessage(e) }); } finally { setWorking(false); } };
  return <div className="mt-3 rounded-md border border-border p-3"><label className="block">Name<input aria-label={`Rename ${profile.name}`} value={name} maxLength={80} disabled={busy} onChange={(e) => setName(e.target.value)} className="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5" /></label>
    <p className="mt-1 text-[11px] text-muted-foreground">Recorded in {profile.referenceLanguage === "de" ? "German" : "English"}{!profile.ready && " · profile missing or incompatible"}{selected && " · new-deck default"}</p>
    <div className="mt-2 flex flex-wrap gap-x-3 gap-y-2"><button disabled={busy || !name.trim() || name.trim() === profile.name} className="underline disabled:opacity-40" onClick={() => void run(() => api.voiceProfileAction(profile.providerId, "rename", profile.id, name))}>Save name</button><button disabled={busy || !profile.ready} className="underline disabled:opacity-40" onClick={() => void run(() => api.setDefaultPresenter(selected ? null : { providerId: profile.providerId, presenterId: profile.id }))}>{selected ? "Clear default" : "Default for new decks"}</button><button disabled={busy} className="underline disabled:opacity-40" onClick={() => onReplace(profile)}>Replace recording…</button><button disabled={busy} className="underline disabled:opacity-40" onClick={() => setRemoving(true)}>Delete…</button></div>
    {removing && <div className="mt-3"><p>Delete this voice profile and its private recordings? Saved narration audio in decks will remain.</p><div className="mt-2 flex gap-3"><button disabled={busy} className="underline text-amber-600" onClick={() => void run(() => api.voiceProfileAction(profile.providerId, "delete", profile.id))}>Delete presenter</button><button disabled={busy} className="underline" onClick={() => setRemoving(false)}>Keep presenter</button></div></div>}
  </div>;
}
export function PresenterLibrary({ providerId, onReplace }: { providerId: string; onReplace: (profile: SavedPresenter) => void }) {
  const status = useSpeech((s) => s.status);
  const profiles = status?.presenters?.filter((p) => p.providerId === providerId) ?? [];
  return <details className="mt-2 text-muted-foreground"><summary className="cursor-pointer">Manage presenters{profiles.length > 0 ? ` (${profiles.length})` : ""}</summary><p className="mt-2 text-[11px]">Changing the new-deck default does not change existing decks. Renaming keeps recordings; replacing a reference changes future speech.</p>{profiles.length ? profiles.map((p) => <PresenterRow key={`${p.id}:${p.name}`} profile={p} onReplace={onReplace} />) : <p className="mt-2">No saved personal presenters yet.</p>}</details>;
}
