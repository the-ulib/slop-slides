//! Versioned narration source. Missing files mean an old, unnarrated deck; invalid
//! files are errors, never silently replaced. Removed slide IDs remain recoverable.
use fs2::FileExt;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::deck;
use crate::error::{Error, Result};
use crate::html;

pub const FILE: &str = "narration.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    De,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct SlideNarration {
    pub text: String,
    pub language_override: Option<Language>,
    pub lead_in_ms: u32,
    pub tail_ms: u32,
    pub silent_duration_ms: Option<u32>,
    pub accepted_take_id: Option<String>,
    pub reviewed_slide_hash: Option<String>,
}
impl Default for SlideNarration {
    fn default() -> Self {
        Self {
            text: String::new(),
            language_override: None,
            lead_in_ms: 250,
            tail_ms: 500,
            silent_duration_ms: None,
            accepted_take_id: None,
            reviewed_slide_hash: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub revision: u64,
    #[serde(default = "default_presenter")]
    pub presenter_id: String,
    #[serde(default = "default_presenter_name")]
    pub presenter_name_snapshot: String,
    #[serde(default)]
    pub default_language: Language,
    #[serde(default = "default_pace")]
    pub pace: f64,
    pub slides: BTreeMap<String, SlideNarration>,
}
fn default_pace() -> f64 {
    1.1
}
fn default_presenter() -> String {
    "preset:ryan".into()
}
fn default_presenter_name() -> String {
    "Ryan".into()
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            presenter_id: default_presenter(),
            presenter_name_snapshot: default_presenter_name(),
            default_language: Language::En,
            pace: default_pace(),
            slides: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub manifest: Manifest,
    /// Fingerprint of the actual file, catching external edits even without a revision bump.
    pub version: String,
}

fn validate(m: &Manifest) -> Result<()> {
    if m.schema_version != 1 {
        return Err(Error::msg(
            "Unsupported narration schema; the file was not changed.",
        ));
    }
    if m.presenter_id.is_empty()
        || m.presenter_id.len() > 256
        || m.presenter_id.contains(['/', '\\'])
    {
        return Err(Error::msg("Invalid presenter ID."));
    }
    if !m.pace.is_finite() || !(0.9..=1.25).contains(&m.pace) {
        return Err(Error::msg("Speaking pace must be between 0.9 and 1.25."));
    }
    if m.slides.len() > 10_000 {
        return Err(Error::msg("Too many narration entries."));
    }
    for (id, s) in &m.slides {
        if id.is_empty() || id.len() > 256 || id.starts_with('#') || s.text.len() > 100_000 {
            return Err(Error::msg(
                "Narration needs stable slide IDs and scripts below 100 KB.",
            ));
        }
        if s.lead_in_ms > 60_000
            || s.tail_ms > 60_000
            || s.silent_duration_ms.is_some_and(|n| n == 0 || n > 600_000)
        {
            return Err(Error::msg(
                "Narration pauses must be at most 60 seconds; silent slides at most 10 minutes.",
            ));
        }
        if s.accepted_take_id
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 256 || s.contains(['/', '\\']))
        {
            return Err(Error::msg("Invalid narration take ID."));
        }
    }
    Ok(())
}

pub fn load(dir: &Path) -> Result<Document> {
    let path = dir.join(FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document {
                manifest: Manifest::default(),
                version: "missing".into(),
            })
        }
        Err(e) => return Err(e.into()),
    };
    if raw.len() > 4_000_000 {
        return Err(Error::msg("Narration file exceeds 4 MB."));
    }
    let manifest: Manifest = serde_json::from_str(&raw)
        .map_err(|e| Error::msg(format!("Cannot read narration.json: {e}")))?;
    validate(&manifest)?;
    Ok(Document {
        manifest,
        version: html::content_hash(&raw),
    })
}

pub fn save(dir: &Path, mut manifest: Manifest, base: &str) -> Result<Document> {
    // App and MCP are separate processes. OS locks release on crash and keep the
    // fingerprint check and atomic replacement together for cooperating writers.
    let internals = dir.join(deck::INTERNAL_DIR);
    fs::create_dir_all(&internals)?;
    let guard = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(internals.join("narration-write.lock"))?;
    FileExt::lock_exclusive(&guard)?;
    let current = load(dir)?;
    if current.version != base {
        return Err(Error::msg("Narration changed on disk. Your edits are kept; review the file version before saving."));
    }
    validate(&manifest)?;
    manifest.revision = current
        .manifest
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::msg("Narration revision overflow."))?;
    let raw =
        serde_json::to_string_pretty(&manifest).map_err(|e| Error::msg(e.to_string()))? + "\n";
    if raw.len() > 4_000_000 {
        return Err(Error::msg("Narration file exceeds 4 MB."));
    }
    deck::atomic_write(&dir.join(FILE), raw.as_bytes())?;
    Ok(Document {
        manifest,
        version: html::content_hash(&raw),
    })
}

/// Prepare narration before creating the duplicate HTML. A failed HTML write leaves
/// a recoverable orphan, rather than losing the source or an older removed copy.
pub fn duplicate(dir: &Path, from: &str, to: &str, from_hash: &str, hash: &str) -> Result<()> {
    let doc = load(dir)?;
    let Some(mut source) = doc.manifest.slides.get(from).cloned() else {
        return Ok(());
    };
    let mut manifest = doc.manifest;
    if let Some(previous) = manifest.slides.remove(to) {
        manifest.slides.insert(
            format!("{to}-archived-{}", uuid::Uuid::new_v4().simple()),
            previous,
        );
    }
    source.reviewed_slide_hash =
        (source.reviewed_slide_hash.as_deref() == Some(from_hash)).then(|| hash.into());
    manifest.slides.insert(to.into(), source);
    save(dir, manifest, &doc.version)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_scripts_default_pace_and_reject_invalid_settings() {
        let old =
            serde_json::json!({"schemaVersion":1,"revision":0,"defaultLanguage":"en","slides":{}});
        let mut manifest: Manifest = serde_json::from_value(old).unwrap();
        assert_eq!(manifest.pace, 1.1);
        for pace in [0.0, 2.0, f64::NAN] {
            manifest.pace = pace;
            assert!(validate(&manifest).is_err());
        }
    }
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("slopslide-narration-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(deck::DECK_FILE), "<html><main class=\"deck\"><section class=\"slide\" id=\"a\">A</section><section class=\"slide\" id=\"b\">B</section></main></html>").unwrap();
            Self(dir)
        }
        fn with_script() -> Self {
            let t = Self::new();
            let mut m = Manifest::default();
            m.slides.insert(
                "a".into(),
                SlideNarration {
                    text: "Hello ✨".into(),
                    ..Default::default()
                },
            );
            save(&t.0, m, "missing").unwrap();
            t
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn old_decks_load_without_creating_a_file_and_scripts_survive_reopen() {
        let t = Temp::new();
        assert_eq!(load(&t.0).unwrap().version, "missing");
        assert!(!t.0.join(FILE).exists());
        let mut m = Manifest::default();
        m.slides.insert(
            "a".into(),
            SlideNarration {
                text: "Hallo".into(),
                language_override: Some(Language::De),
                ..Default::default()
            },
        );
        let saved = save(&t.0, m, "missing").unwrap();
        assert_eq!(saved.manifest.revision, 1);
        assert_eq!(load(&t.0).unwrap().manifest, saved.manifest);
    }
    #[test]
    fn rejects_stale_and_invalid_saves_without_touching_disk() {
        let t = Temp::with_script();
        let doc = load(&t.0).unwrap();
        let before = fs::read(t.0.join(FILE)).unwrap();
        assert!(save(&t.0, doc.manifest.clone(), "missing").is_err());
        let mut bad = doc.manifest.clone();
        bad.schema_version = 2;
        assert!(save(&t.0, bad, &doc.version).is_err());
        let mut bad = doc.manifest.clone();
        bad.slides.get_mut("a").unwrap().tail_ms = 60_001;
        assert!(save(&t.0, bad, &doc.version).is_err());
        assert_eq!(fs::read(t.0.join(FILE)).unwrap(), before);
        // An external writer that forgets to increment revision is still detected.
        fs::write(
            t.0.join(FILE),
            String::from_utf8(before)
                .unwrap()
                .replace("Hello", "Changed"),
        )
        .unwrap();
        assert!(save(&t.0, doc.manifest, &doc.version).is_err());
    }
    #[test]
    fn simultaneous_writers_cannot_both_replace_the_same_version() {
        let t = Temp::with_script();
        let doc = load(&t.0).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let dir = t.0.clone();
                let doc = doc.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    save(&dir, doc.manifest, &doc.version).is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .filter(|ok| *ok)
                .count(),
            1
        );
        assert_eq!(load(&t.0).unwrap().manifest.revision, 2);
    }
    #[test]
    fn paired_snapshot_pruning_keeps_narration_and_deck_together() {
        let t = Temp::with_script();
        for _ in 0..35 {
            deck::snapshot(&t.0).unwrap();
        }
        let internal = t.0.join(deck::INTERNAL_DIR);
        let html = internal.join("snapshots");
        let json = internal.join("narration-snapshots");
        assert_eq!(fs::read_dir(&html).unwrap().count(), 30);
        assert_eq!(fs::read_dir(&json).unwrap().count(), 30);
        for entry in fs::read_dir(&json).unwrap() {
            let path = entry.unwrap().path();
            assert!(html
                .join(path.file_stem().unwrap())
                .with_extension("html")
                .exists());
        }
    }

    #[test]
    fn corrupt_future_and_unknown_fields_are_not_silently_replaced() {
        let t = Temp::new();
        for raw in [
            "{oops",
            r#"{"schemaVersion":2,"revision":0,"slides":{}}"#,
            r#"{"schemaVersion":1,"revision":0,"slides":{},"newField":true}"#,
            r#"{"schemaVersion":1,"revision":0,"defaultLanguage":"fr","slides":{}}"#,
        ] {
            fs::write(t.0.join(FILE), raw).unwrap();
            assert!(load(&t.0).is_err());
            assert!(save(&t.0, Manifest::default(), "missing").is_err());
            assert_eq!(fs::read_to_string(t.0.join(FILE)).unwrap(), raw);
        }
    }
    #[test]
    fn reorder_hide_delete_and_restoration_keep_scripts_by_id() {
        let t = Temp::with_script();
        let original = fs::read_to_string(t.0.join(deck::DECK_FILE)).unwrap();
        deck::reorder(&t.0, "test", vec!["b".into(), "a".into()]).unwrap();
        deck::set_slide_hidden(&t.0, "test", "a", true).unwrap();
        deck::delete_slide(&t.0, "test", "a").unwrap();
        assert_eq!(load(&t.0).unwrap().manifest.slides["a"].text, "Hello ✨");
        deck::save_source(&t.0, "test", &original, None, false).unwrap();
        assert_eq!(load(&t.0).unwrap().manifest.slides["a"].text, "Hello ✨");
        let archived = t.0.join(deck::INTERNAL_DIR).join("narration-snapshots");
        assert!(fs::read_dir(archived)
            .unwrap()
            .any(|p| fs::read_to_string(p.unwrap().path())
                .unwrap()
                .contains("Hello")));
    }
    #[test]
    fn duplicate_copies_script_and_preserves_removed_copy() {
        let t = Temp::with_script();
        let (_, copied) = deck::duplicate(&t.0, "test", "a").unwrap();
        assert_eq!(
            load(&t.0).unwrap().manifest.slides[&copied].text,
            "Hello ✨"
        );
        assert_eq!(
            load(&t.0).unwrap().manifest.slides[&copied].reviewed_slide_hash,
            None,
            "unreviewed scripts stay unreviewed"
        );
        deck::delete_slide(&t.0, "test", &copied).unwrap();
        deck::duplicate(&t.0, "test", "a").unwrap();
        assert_eq!(load(&t.0).unwrap().manifest.slides.len(), 3);
    }
}
