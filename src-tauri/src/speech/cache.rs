//! Immutable accepted recordings. Source keys exclude visual hashes and timeline pauses.
use crate::{
    deck,
    error::{Error, Result},
    narration,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const ENGINE: &str = speech_connector::QWEN_ENGINE;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(default = "narration::default_provider")]
    pub provider_id: String,
    pub text: String,
    pub language: narration::Language,
    pub presenter_id: String,
    pub pace: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub narration_format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presenter_revision: Option<String>,
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}
impl Source {
    pub fn from_manifest(manifest: &narration::Manifest, slide: &str) -> Result<Self> {
        let script = manifest
            .slides
            .get(slide)
            .ok_or_else(|| Error::msg("This slide has no script."))?;
        let text = script.text.trim().replace("\r\n", "\n");
        if text.is_empty() {
            return Err(Error::msg("Write a narration script first."));
        }
        let plan = speech_connector::narration::parse(&text)?;
        let source = Self {
            presenter_revision: None,
            narration_format_version: if plan.marked {
                speech_connector::narration::FORMAT_VERSION
            } else {
                0
            },
            provider_id: script
                .speech_provider_id_override
                .as_ref()
                .unwrap_or(&manifest.speech_provider_id)
                .clone(),
            text,
            language: script
                .language_override
                .clone()
                .unwrap_or_else(|| manifest.default_language.clone()),
            presenter_id: script
                .presenter_id_override
                .as_ref()
                .unwrap_or(&manifest.presenter_id)
                .clone(),
            pace: script.pace_override.unwrap_or(manifest.pace),
        };
        Ok(source)
    }
    pub fn request(&self) -> speech_connector::SynthesisRequest {
        speech_connector::SynthesisRequest {
            text: self.text.clone(),
            language: match self.language {
                narration::Language::De => "de",
                narration::Language::En => "en",
            }
            .into(),
            voice_id: self.presenter_id.clone(),
            pace: self.pace,
            voice_revision: self.presenter_revision.clone(),
        }
    }
    #[cfg(test)]
    pub fn key(&self) -> String {
        // Only the unchanged pinned local engine is reusable without a descriptor.
        self.key_for(ENGINE, &speech_connector::models::pack().revision)
    }
    pub fn key_for(&self, engine: &str, revision: &str) -> String {
        // Preserve the exact legacy Qwen hash (field order included). Provider
        // binding is implicit only for that pinned legacy engine/normalization.
        let identity = if self.provider_id == speech_connector::QWEN_ID
            && engine == ENGINE
            && self.narration_format_version == 0
            && self.presenter_revision.is_none()
        {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Legacy<'a> {
                text: &'a str,
                language: &'a narration::Language,
                presenter_id: &'a str,
                pace: f64,
            }
            serde_json::to_vec(&(
                engine,
                revision,
                Legacy {
                    text: &self.text,
                    language: &self.language,
                    presenter_id: &self.presenter_id,
                    pace: self.pace,
                },
            ))
            .expect("source JSON")
        } else {
            serde_json::to_vec(&(engine, revision, speech_connector::NORMALIZATION, self))
                .expect("source JSON")
        };
        format!("{:x}", Sha256::digest(identity))
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
    pub id: String,
    pub key: String,
    pub engine_version: String,
    pub model_revision: String,
    pub source: Source,
    pub samples: usize,
    pub sample_rate: u32,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTake {
    #[serde(flatten)]
    pub take: Take,
    pub created_at: u64,
}
// Associations belong to the deck, not the provider. One cached recording can
// belong to multiple slides; immutable links also retain superseded results.
pub fn remember(dir: &Path, slide: &str, take: &Take) -> Result<()> {
    let prefix = format!("{:x}", Sha256::digest(slide.as_bytes()));
    let path = safe_directory(dir, "history")?.join(format!("{prefix}-{}.json", take.id));
    if !path.exists() {
        deck::atomic_write(
            &path,
            &serde_json::to_vec(&(slide, &take.id)).expect("history JSON"),
        )?;
    }
    Ok(())
}
/// Recover legacy associations from accepted references and deck snapshots.
/// Never guess ownership from matching words: two slides can share a script.
pub fn history(dir: &Path, slide: &str) -> Result<Vec<HistoryTake>> {
    let mut ids = std::collections::BTreeSet::new();
    let doc = narration::load(dir)?;
    if let Some(id) = doc
        .manifest
        .slides
        .get(slide)
        .and_then(|s| s.accepted_take_id.as_ref())
    {
        ids.insert(id.clone());
    }
    let snapshots = dir.join(deck::INTERNAL_DIR).join("narration-snapshots");
    if let Ok(entries) = fs::read_dir(snapshots) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|m| m.is_file())
                && entry.metadata().is_ok_and(|m| m.len() <= 4_000_000)
            {
                if let Ok(manifest) = fs::read(entry.path()).and_then(|b| {
                    serde_json::from_slice::<narration::Manifest>(&b).map_err(std::io::Error::other)
                }) {
                    if let Some(id) = manifest
                        .slides
                        .get(slide)
                        .and_then(|s| s.accepted_take_id.as_ref())
                    {
                        ids.insert(id.clone());
                    }
                }
            }
        }
    }
    let prefix = format!("{:x}-", Sha256::digest(slide.as_bytes()));
    let links = safe_directory(dir, "history")?;
    for entry in fs::read_dir(&links)?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(&prefix)
            || !entry.file_type().is_ok_and(|m| m.is_file())
            || !entry.metadata().is_ok_and(|m| m.len() <= 2048)
        {
            continue;
        }
        if let Ok((owner, id)) = fs::read(entry.path()).and_then(|b| {
            serde_json::from_slice::<(String, String)>(&b).map_err(std::io::Error::other)
        }) {
            if owner == slide {
                ids.insert(id);
            }
        }
    }
    let mut takes = Vec::new();
    for id in ids {
        // A damaged or deleted old take must not hide the remaining history.
        if let Ok(Some(take)) = read(dir, &id) {
            remember(dir, slide, &take)?;
            let created_at = fs::metadata(take_path(dir, &id, "json")?)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            takes.push(HistoryTake { take, created_at });
        }
    }
    takes.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.take.id.cmp(&a.take.id))
    });
    Ok(takes)
}
/// Selecting a take explicitly restores its source. A fingerprint prevents
/// overwriting edits from another window or the agent while the picker is open.
pub fn select(dir: &Path, slide: &str, id: &str, base: &str) -> Result<narration::Document> {
    let take = history(dir, slide)?
        .into_iter()
        .find(|t| t.take.id == id)
        .ok_or_else(|| Error::msg("This recording is not available for this slide."))?
        .take;
    let mut doc = narration::load(dir)?;
    let script = doc.manifest.slides.entry(slide.into()).or_default();
    if script.text.trim().replace("\r\n", "\n") != take.source.text {
        script.reviewed_slide_hash = None;
    }
    script.text = take.source.text;
    script.language_override = Some(take.source.language);
    script.speech_provider_id_override = Some(take.source.provider_id);
    script.presenter_name_snapshot_override = Some(take.source.presenter_id.clone());
    script.presenter_id_override = Some(take.source.presenter_id);
    script.pace_override = Some(take.source.pace);
    script.accepted_take_id = Some(take.id);
    narration::save(dir, doc.manifest, base)
}
pub fn reusable(
    dir: &Path,
    key: &str,
    accepted: Option<&str>,
    fresh: bool,
) -> Result<Option<Take>> {
    if fresh {
        return Ok(None);
    }
    if let Some(id) = accepted {
        if let Ok(Some(take)) = read(dir, id) {
            if take.key == key {
                return Ok(Some(take));
            }
        }
    }
    find(dir, key)
}
pub fn safe_directory(dir: &Path, kind: &str) -> Result<PathBuf> {
    safe_directory_parts(dir, &[deck::INTERNAL_DIR, "speech", kind])
}
fn safe_directory_parts(dir: &Path, parts: &[&str]) -> Result<PathBuf> {
    let mut current = dir.to_path_buf();
    for part in parts {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                return Err(Error::msg(
                    "Speech storage must be a regular directory inside the deck.",
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(current)
}
pub fn root(dir: &Path) -> PathBuf {
    dir.join(deck::INTERNAL_DIR).join("speech/takes")
}
pub fn take_path(dir: &Path, id: &str, extension: &str) -> Result<PathBuf> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| Error::msg("Invalid recording ID."))?;
    if uuid.to_string() != id {
        return Err(Error::msg("Invalid recording ID."));
    }
    let (relative, mut parents) = match extension {
        "wav" => (format!("audio/{id}.wav"), vec![dir.join("audio")]),
        "json" => (
            format!("{}/speech/takes/{id}.json", deck::INTERNAL_DIR),
            vec![
                dir.join(deck::INTERNAL_DIR),
                dir.join(deck::INTERNAL_DIR).join("speech"),
                root(dir),
            ],
        ),
        _ => return Err(Error::msg("Invalid recording file type.")),
    };
    let path = deck::resolve_in_deck(dir, &relative)?;
    parents.push(path.clone());
    for parent in parents {
        if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error::msg("Speech storage cannot use symbolic links."));
        }
    }
    Ok(path)
}
pub fn read(dir: &Path, id: &str) -> Result<Option<Take>> {
    let path = take_path(dir, id, "json")?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)?;
    if bytes.len() > 110_000 {
        return Err(Error::msg("Recording metadata is too large."));
    }
    let take: Take = serde_json::from_slice(&bytes).map_err(|e| Error::msg(e.to_string()))?;
    if take.id != id
        || take.sample_rate != 24000
        || take.samples == 0
        || take.samples > 600 * 24000
        || take.key
            != take
                .source
                .key_for(&take.engine_version, &take.model_revision)
    {
        return Err(Error::msg("Invalid recording metadata."));
    }
    let wav = take_path(dir, id, "wav")?;
    // Earlier previews hid WAVs beside cache metadata. Expose a verified copy
    // without changing take IDs, metadata, accepted references or the original.
    let legacy = path.with_extension("wav");
    if fs::symlink_metadata(&legacy).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::msg("Speech storage cannot use symbolic links."));
    }
    let migrate = !wav.exists() && legacy.is_file();
    if !wav.exists() && !migrate {
        return Ok(None);
    }
    let bytes = fs::read(if migrate { &legacy } else { &wav })?;
    let pcm = decode_wav(&bytes)?;
    if pcm.len() != take.samples || format!("{:x}", Sha256::digest(&bytes)) != take.sha256 {
        return Err(Error::msg(
            "Recording integrity check failed. Generate it again.",
        ));
    }
    if migrate {
        safe_directory_parts(dir, &["audio"])?;
        deck::atomic_write(&wav, &bytes)?;
    }
    Ok(Some(take))
}
pub fn find(dir: &Path, key: &str) -> Result<Option<Take>> {
    let entries = match fs::read_dir(root(dir)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
            if let Ok(Some(take)) = read(dir, id) {
                if take.key == key {
                    return Ok(Some(take));
                }
            }
        }
    }
    Ok(None)
}
#[cfg(test)]
pub fn publish(dir: &Path, source: Source, pcm: &[i16]) -> Result<Take> {
    let provider = speech_connector::QwenConnector::new(PathBuf::new(), None);
    publish_for(
        dir,
        source,
        pcm,
        &speech_connector::SpeechProvider::describe(&provider),
    )
}
pub fn publish_for(
    dir: &Path,
    source: Source,
    pcm: &[i16],
    provider: &speech_connector::Descriptor,
) -> Result<Take> {
    if pcm.is_empty() || pcm.len() > 600 * 24000 {
        return Err(Error::msg(
            "Recording must be at most ten minutes per slide.",
        ));
    }
    safe_directory(dir, "takes")?;
    safe_directory_parts(dir, &["audio"])?;
    let id = uuid::Uuid::new_v4().to_string();
    let wav = encode_wav(pcm);
    let take = Take {
        id,
        key: source.key_for(&provider.engine_version, &provider.model_revision),
        engine_version: provider.engine_version.clone(),
        model_revision: provider.model_revision.clone(),
        source,
        samples: pcm.len(),
        sample_rate: 24000,
        sha256: format!("{:x}", Sha256::digest(&wav)),
    };
    deck::atomic_write(&take_path(dir, &take.id, "wav")?, &wav)?;
    deck::atomic_write(
        &take_path(dir, &take.id, "json")?,
        &serde_json::to_vec(&take).expect("take JSON"),
    )?;
    Ok(take)
}
/// Accept only if the current speech source still matches; merge unrelated concurrent edits.
pub fn accept(dir: &Path, slide: &str, take: &Take) -> Result<bool> {
    remember(dir, slide, take)?;
    for _ in 0..4 {
        let mut doc = narration::load(dir)?;
        if let Some(id) = doc
            .manifest
            .slides
            .get(slide)
            .and_then(|s| s.accepted_take_id.as_ref())
        {
            if let Ok(Some(previous)) = read(dir, id) {
                remember(dir, slide, &previous)?;
            }
        }
        if Source::from_manifest(&doc.manifest, slide)
            .map(|mut s| {
                s.presenter_revision = take.source.presenter_revision.clone();
                s.key_for(&take.engine_version, &take.model_revision)
            })
            .ok()
            .as_deref()
            != Some(&take.key)
        {
            return Ok(false);
        }
        doc.manifest
            .slides
            .get_mut(slide)
            .expect("source exists")
            .accepted_take_id = Some(take.id.clone());
        if narration::save(dir, doc.manifest, &doc.version).is_ok() {
            return Ok(true);
        }
    }
    Err(Error::msg(
        "Narration changed while accepting audio. The recording is cached; retry to reuse it.",
    ))
}
pub fn encode_wav(pcm: &[i16]) -> Vec<u8> {
    let bytes = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(bytes as usize + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(bytes + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&24000_u32.to_le_bytes());
    wav.extend_from_slice(&48000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for value in pcm {
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}
pub fn decode_wav(wav: &[u8]) -> Result<Vec<i16>> {
    let bad = || Error::msg("Speech worker returned invalid 24 kHz mono PCM16 audio.");
    if wav.len() < 44
        || wav.len() > 600 * 48000 + 4096
        || &wav[..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
    {
        return Err(bad());
    }
    let mut offset = 12;
    let mut format = false;
    let mut pcm = None;
    while offset + 8 <= wav.len() {
        let kind = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().expect("chunk size"))
            as usize;
        offset += 8;
        let end = offset
            .checked_add(size)
            .filter(|n| *n <= wav.len())
            .ok_or_else(bad)?;
        let data = &wav[offset..end];
        if kind == b"fmt " {
            format = size >= 16
                && data[..4] == [1, 0, 1, 0]
                && data[4..8] == 24000_u32.to_le_bytes()
                && data[12..16] == [2, 0, 16, 0];
        }
        if kind == b"data" {
            if size == 0 || size % 2 != 0 {
                return Err(bad());
            }
            pcm = Some(
                data.chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect(),
            );
        }
        offset = end + size % 2;
    }
    if !format {
        return Err(bad());
    }
    pcm.ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use speech_connector::SpeechProvider;
    #[test]
    fn profile_revision_versions_cache_and_accepted_take_preserves_frozen_source() {
        let dir = dir();
        let mut m = narration::Manifest {
            presenter_id: "profile:test".into(),
            ..Default::default()
        };
        m.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                ..Default::default()
            },
        );
        narration::save(&dir, m.clone(), "missing").unwrap();
        let mut old = Source::from_manifest(&m, "intro").unwrap();
        old.presenter_revision = Some("revision-1".into());
        let new = Source {
            presenter_revision: Some("revision-2".into()),
            ..old.clone()
        };
        assert_ne!(
            old.key_for("profile-engine", "base"),
            new.key_for("profile-engine", "base")
        );
        let mut descriptor = speech_connector::FixtureProvider.describe();
        descriptor.engine_version = "profile-engine".into();
        descriptor.model_revision = "base".into();
        let take = publish_for(&dir, old, &[1, 2, 3], &descriptor).unwrap();
        assert!(accept(&dir, "intro", &take).unwrap());
        assert_eq!(
            read(&dir, &take.id)
                .unwrap()
                .unwrap()
                .source
                .presenter_revision
                .as_deref(),
            Some("revision-1")
        );
        assert_eq!(
            narration::load(&dir).unwrap().manifest.slides["intro"]
                .accepted_take_id
                .as_deref(),
            Some(take.id.as_str())
        );
        fs::remove_dir_all(dir).unwrap();
    }
    fn dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("speech-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn source() -> Source {
        Source {
            provider_id: narration::default_provider(),
            narration_format_version: 0,
            presenter_revision: None,
            text: "Hello".into(),
            language: narration::Language::En,
            presenter_id: "preset:ryan".into(),
            pace: 1.1,
        }
    }
    #[test]
    fn pause_format_keys_do_not_reuse_pre_marker_recordings_and_legacy_keys_stay_stable() {
        let mut manifest = narration::Manifest::default();
        manifest.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello [pause:800ms] world".into(),
                ..Default::default()
            },
        );
        let modern = Source::from_manifest(&manifest, "intro").unwrap();
        let old = Source {
            narration_format_version: 0,
            ..modern.clone()
        };
        assert_eq!(modern.narration_format_version, 1);
        assert_ne!(modern.key(), old.key());
        let dir = dir();
        let old_take = publish(&dir, old, &[1, 2, 3]).unwrap();
        assert_eq!(
            read(&dir, &old_take.id)
                .unwrap()
                .unwrap()
                .source
                .narration_format_version,
            0
        );
        assert!(find(&dir, &modern.key()).unwrap().is_none());
        let new_take = publish(&dir, modern, &[4, 5, 6]).unwrap();
        assert_eq!(
            read(&dir, &new_take.id)
                .unwrap()
                .unwrap()
                .source
                .narration_format_version,
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn history_keeps_variants_isolated_and_restores_source_with_a_fingerprint() {
        let dir = dir();
        let mut m = narration::Manifest::default();
        m.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                reviewed_slide_hash: Some("review".into()),
                tail_ms: 1200,
                ..Default::default()
            },
        );
        m.slides.insert(
            "other".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                ..Default::default()
            },
        );
        narration::save(&dir, m, "missing").unwrap();
        let old = publish(&dir, source(), &[1, 2, 3]).unwrap();
        assert!(accept(&dir, "intro", &old).unwrap());
        let other = publish(&dir, source(), &[4, 5, 6]).unwrap();
        assert!(accept(&dir, "other", &other).unwrap());
        let mut doc = narration::load(&dir).unwrap();
        doc.manifest.slides.get_mut("intro").unwrap().text = "Changed".into();
        let doc = narration::save(&dir, doc.manifest, &doc.version).unwrap();
        let new = publish(
            &dir,
            Source {
                text: "Changed".into(),
                language: narration::Language::De,
                ..source()
            },
            &[7, 8, 9],
        )
        .unwrap();
        // Superseded language still retains the new take for this slide.
        assert!(!accept(&dir, "intro", &new).unwrap());
        let h = history(&dir, "intro").unwrap();
        assert_eq!(h.len(), 2);
        assert!(!h.iter().any(|t| t.take.id == other.id));
        assert!(h.windows(2).all(|w| w[0].created_at >= w[1].created_at));
        let bytes = fs::read(take_path(&dir, &old.id, "wav").unwrap()).unwrap();
        assert!(select(&dir, "intro", &old.id, "stale").is_err());
        assert_eq!(
            narration::load(&dir).unwrap().manifest.slides["intro"].text,
            "Changed"
        );
        assert!(select(&dir, "intro", &other.id, &doc.version).is_err());
        let selected = select(&dir, "intro", &old.id, &doc.version).unwrap();
        let script = &selected.manifest.slides["intro"];
        assert_eq!(script.text, "Hello");
        assert_eq!(script.accepted_take_id.as_deref(), Some(old.id.as_str()));
        assert_eq!(script.tail_ms, 1200);
        assert!(script.reviewed_slide_hash.is_none());
        assert_eq!(
            Source::from_manifest(&selected.manifest, "intro").unwrap(),
            old.source
        );
        assert_eq!(
            selected.manifest.slides["other"]
                .accepted_take_id
                .as_deref(),
            Some(other.id.as_str())
        );
        assert_eq!(
            fs::read(take_path(&dir, &old.id, "wav").unwrap()).unwrap(),
            bytes
        );
        assert!(reusable(&dir, &old.key, Some(&old.id), true)
            .unwrap()
            .is_none());
        assert_eq!(
            reusable(&dir, &old.key, Some(&old.id), false)
                .unwrap()
                .unwrap()
                .id,
            old.id
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_history_recovers_snapshot_references_and_skips_damaged_audio() {
        let dir = dir();
        let old = publish(&dir, source(), &[1, 2, 3]).unwrap();
        let current = publish(&dir, source(), &[4, 5, 6]).unwrap();
        let mut m = narration::Manifest::default();
        m.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                accepted_take_id: Some(old.id.clone()),
                ..Default::default()
            },
        );
        let snapshots = dir.join(deck::INTERNAL_DIR).join("narration-snapshots");
        fs::create_dir_all(&snapshots).unwrap();
        fs::write(snapshots.join("old.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        m.slides.get_mut("intro").unwrap().accepted_take_id = Some(current.id.clone());
        narration::save(&dir, m, "missing").unwrap();
        assert_eq!(history(&dir, "intro").unwrap().len(), 2);
        fs::remove_dir_all(snapshots).unwrap();
        assert_eq!(history(&dir, "intro").unwrap().len(), 2);
        fs::write(take_path(&dir, &old.id, "wav").unwrap(), b"broken").unwrap();
        let history = history(&dir, "intro").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].take.id, current.id);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_qwen_metadata_keeps_its_exact_identity_without_provider_field() {
        let source = source();
        let legacy = serde_json::json!({"id":uuid::Uuid::new_v4().to_string(),"key":source.key(),"engineVersion":ENGINE,"modelRevision":speech_connector::models::pack().revision,"source":{"text":"Hello","language":"en","presenterId":"preset:ryan","pace":1.1},"samples":3,"sampleRate":24000,"sha256":"unused"});
        let take: Take = serde_json::from_value(legacy).unwrap();
        assert_eq!(take.source.provider_id, "qwen-local");
        let old_json=format!("[\"{}\",\"{}\",{{\"text\":\"Hello\",\"language\":\"en\",\"presenterId\":\"preset:ryan\",\"pace\":1.1}}]",ENGINE,speech_connector::models::pack().revision);
        assert_eq!(
            take.key,
            format!("{:x}", Sha256::digest(old_json.as_bytes()))
        );
        let changed = Source {
            provider_id: "another-provider".into(),
            ..source
        };
        assert_ne!(changed.key(), take.key);
    }
    #[test]
    fn keys_cover_text_voice_language_pace() {
        let s = source();
        let key = s.key();
        for changed in [
            Source {
                text: "Other".into(),
                ..s.clone()
            },
            Source {
                language: narration::Language::De,
                ..s.clone()
            },
            Source {
                presenter_id: "preset:aiden".into(),
                ..s.clone()
            },
            Source {
                pace: 1.0,
                ..s.clone()
            },
        ] {
            assert_ne!(key, changed.key());
        }
    }
    #[test]
    fn wav_round_trip_and_invalid_format() {
        let pcm = vec![0, 32767, -32768, 12];
        let mut wav = encode_wav(&pcm);
        assert_eq!(decode_wav(&wav).unwrap(), pcm);
        wav[24] = 0;
        assert!(decode_wav(&wav).is_err());
        assert!(decode_wav(&wav[..15]).is_err());
    }
    #[test]
    fn immutable_cache_checks_integrity_and_paths() {
        let dir = dir();
        let take = publish(&dir, source(), &[0, 1, 2]).unwrap();
        assert!(dir.join("audio").join(format!("{}.wav", take.id)).is_file());
        assert!(!root(&dir).join(format!("{}.wav", take.id)).exists());
        assert_eq!(find(&dir, &take.key).unwrap().unwrap().id, take.id);
        assert!(take_path(&dir, "../../secret", "wav").is_err());
        fs::write(
            take_path(&dir, &take.id, "wav").unwrap(),
            encode_wav(&[0, 1, 3]),
        )
        .unwrap();
        assert!(read(&dir, &take.id).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_recordings_become_visible_without_changing_references_or_metadata() {
        let dir = dir();
        let mut manifest = narration::Manifest::default();
        manifest.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                ..Default::default()
            },
        );
        narration::save(&dir, manifest, "missing").unwrap();
        let take = publish(&dir, source(), &[1, 2, 3]).unwrap();
        assert!(accept(&dir, "intro", &take).unwrap());
        let visible = take_path(&dir, &take.id, "wav").unwrap();
        let legacy = root(&dir).join(format!("{}.wav", take.id));
        fs::rename(&visible, &legacy).unwrap();
        let metadata = fs::read(take_path(&dir, &take.id, "json").unwrap()).unwrap();
        let manifest = fs::read(dir.join("narration.json")).unwrap();
        assert_eq!(find(&dir, &take.key).unwrap().unwrap().id, take.id);
        assert_eq!(fs::read(&visible).unwrap(), fs::read(&legacy).unwrap());
        assert_eq!(
            fs::read(take_path(&dir, &take.id, "json").unwrap()).unwrap(),
            metadata
        );
        assert_eq!(fs::read(dir.join("narration.json")).unwrap(), manifest);
        // A visible user-modified file must not be overwritten by the legacy copy.
        fs::write(&visible, encode_wav(&[7, 8, 9])).unwrap();
        assert!(read(&dir, &take.id).is_err());
        assert_eq!(
            decode_wav(&fs::read(&visible).unwrap()).unwrap(),
            vec![7, 8, 9]
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn visible_audio_directory_cannot_redirect_writes_outside_the_deck() {
        let dir = dir();
        let outside = self::dir();
        std::os::unix::fs::symlink(&outside, dir.join("audio")).unwrap();
        assert!(publish(&dir, source(), &[1, 2, 3]).is_err());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
    #[test]
    fn accept_merges_unrelated_edits_but_rejects_changed_source() {
        let dir = dir();
        let mut manifest = narration::Manifest::default();
        manifest.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                ..Default::default()
            },
        );
        let doc = narration::save(&dir, manifest, "missing").unwrap();
        let s = Source::from_manifest(&doc.manifest, "intro").unwrap();
        let take = publish(&dir, s, &[1, 2, 3]).unwrap();
        let mut m = doc.manifest;
        m.slides.insert(
            "other".into(),
            narration::SlideNarration {
                text: "Keep".into(),
                ..Default::default()
            },
        );
        m.slides.get_mut("intro").unwrap().tail_ms = 1000;
        narration::save(&dir, m, &doc.version).unwrap();
        assert!(accept(&dir, "intro", &take).unwrap());
        let mut doc = narration::load(&dir).unwrap();
        assert_eq!(doc.manifest.slides["other"].text, "Keep");
        doc.manifest.slides.get_mut("intro").unwrap().text = "Changed".into();
        narration::save(&dir, doc.manifest, &doc.version).unwrap();
        assert!(!accept(&dir, "intro", &take).unwrap());
        assert_eq!(
            narration::load(&dir).unwrap().manifest.slides["intro"]
                .accepted_take_id
                .as_deref(),
            Some(take.id.as_str())
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
