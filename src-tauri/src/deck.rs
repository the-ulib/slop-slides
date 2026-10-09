//! Deck storage. A deck is a folder in the library holding one self-contained HTML file:
//!
//! ```text
//! <library>/<deck-id>/
//!   deck.html      every slide, the shared styles, and the embedded player runtime
//!   narration.json optional versioned spoken scripts, keyed by slide ID
//!   assets/        user-attached media, referenced as assets/<file>
//!   .slopslide/    app internals: chat history, agent session, reference docs, snapshots,
//!                  sketches (screenshots of slides the user drew on, for the agent)
//! ```
//!
//! `deck.html` opens directly in any browser as a slideshow; [`export`] inlines the
//! assets so the single file can be shared on its own.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::html;
use crate::lint;
use crate::review::{self, Review};

pub const INTERNAL_DIR: &str = ".slopslide";
pub const DECK_FILE: &str = "deck.html";
const DECK_TEMPLATE: &str = include_str!("../assets/deck-template.html");
const BLANK_SLIDE: &str = include_str!("../assets/blank-slide.html");
const SNAPSHOTS_KEPT: usize = 30;
const SKETCHES_KEPT: usize = 30;
const REFERENCE_DOCS: &[(&str, &str)] = &[
    (
        "STYLE_PRESETS.md",
        include_str!("../prompts/STYLE_PRESETS.md"),
    ),
    (
        "animation-patterns.md",
        include_str!("../prompts/animation-patterns.md"),
    ),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckSummary {
    pub id: String,
    pub title: String,
    pub slide_count: usize,
    pub first_slide: Option<String>,
    pub updated_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    pub id: String,
    /// Changes whenever this slide's markup changes, so only its preview reloads.
    pub hash: String,
    /// Skipped by the player (presenting, exported file); still shown in the editor.
    pub hidden: bool,
    /// Has elements the user moved by hand, waiting for the agent to tidy the layout.
    pub moved: bool,
}

/// A named group of slides, started by a marker between slides in deck.html.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// Position among the deck's section markers, in document order.
    pub index: usize,
    pub title: String,
    /// Number of slides before the marker; the section starts at the slide with this index.
    pub before: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deck {
    pub id: String,
    pub title: String,
    pub path: String,
    pub slides: Vec<Slide>,
    pub sections: Vec<Section>,
    /// Changes whenever anything outside the slides (styles, fonts, runtime) changes.
    pub shell_hash: String,
    /// What the user drew on slides, by slide id (see [`review`]).
    pub review: Review,
}

pub fn library_root(app: &AppHandle) -> Result<PathBuf> {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| Error::msg(format!("cannot locate documents folder: {e}")))?;
    let root = base.join("SlopSlide");
    fs::create_dir_all(&root)?;
    Ok(root)
}

pub fn deck_dir(app: &AppHandle, id: &str) -> Result<PathBuf> {
    deck_dir_in(&library_root(app)?, id)
}

fn deck_dir_in(root: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !is_plain_name(id) {
        return Err(Error::msg(format!("invalid deck id: {id}")));
    }
    let dir = root.join(id);
    if !dir.join(DECK_FILE).is_file() {
        return Err(Error::msg(format!("deck not found: {id}")));
    }
    Ok(dir)
}

/// Resolves a deck-relative path, refusing anything that escapes the deck folder.
pub fn resolve_in_deck(dir: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    let safe = rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !safe || rel.is_empty() {
        return Err(Error::msg(format!("invalid path: {rel}")));
    }
    Ok(dir.join(rel_path))
}

fn is_plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

fn read_html(dir: &Path) -> Result<String> {
    Ok(fs::read_to_string(dir.join(DECK_FILE))?)
}

fn write_html(dir: &Path, html: &str) -> Result<()> {
    atomic_write(&dir.join(DECK_FILE), html.as_bytes())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn fallback_title(id: &str) -> String {
    id.replace('-', " ")
}

pub fn list(root: &Path) -> Result<Vec<DeckSummary>> {
    let mut decks = Vec::new();
    for entry in fs::read_dir(root)? {
        let dir = entry?.path();
        let Ok(source) = read_html(&dir) else {
            continue;
        };
        let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let slides = html::find_slides(&source);
        decks.push(DeckSummary {
            title: html::title(&source).unwrap_or_else(|| fallback_title(&id)),
            slide_count: slides.len(),
            first_slide: slides.first().and_then(|s| s.id.clone()),
            updated_ms: modified_ms(&dir.join(DECK_FILE)),
            id,
        });
    }
    decks.sort_by_key(|d| std::cmp::Reverse(d.updated_ms));
    Ok(decks)
}

fn unique_dir(root: &Path, stem: &str) -> PathBuf {
    let stem = if stem.is_empty() { "untitled" } else { stem };
    std::iter::once(root.join(stem))
        .chain((2..).map(|n| root.join(format!("{stem}-{n}"))))
        .find(|p| !p.exists())
        .expect("unbounded")
}

/// Writes app-owned files and keeps the deck consistent: unique slide ids and the current
/// player runtime. Safe to call repeatedly; only writes when something changed.
pub fn normalize(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir.join("assets"))?;
    let reference = dir.join(INTERNAL_DIR).join("reference");
    fs::create_dir_all(&reference)?;
    for (name, body) in REFERENCE_DOCS {
        fs::write(reference.join(name), body)?;
    }
    let source = read_html(dir)?;
    let fixed = html::normalize_ids(&source).unwrap_or_else(|| source.clone());
    let fixed = html::ensure_runtime(&fixed);
    let fixed = prune_review(&fixed).unwrap_or(fixed);
    if fixed != source {
        write_html(dir, &fixed)?;
    }
    Ok(())
}

/// `html` without review marks of slides it no longer has; None when there are none to drop.
fn prune_review(html: &str) -> Option<String> {
    let slides = html::find_slides(html);
    let ids = slides.iter().filter_map(|s| s.id.as_deref()).collect();
    review::prune(html, &ids)
}

pub fn create(root: &Path, title: &str) -> Result<Deck> {
    let title = title.trim();
    let title = if title.is_empty() {
        "Untitled deck"
    } else {
        title
    };
    let dir = unique_dir(root, &html::slugify(title));
    fs::create_dir_all(&dir)?;
    write_html(&dir, &html::set_title(DECK_TEMPLATE, title))?;
    let id = dir.file_name().unwrap().to_string_lossy().into_owned();
    open(&dir, &id, true)
}

/// Loads a deck, normalizing it first unless the agent may be mid-edit.
pub fn open(dir: &Path, id: &str, normalize_first: bool) -> Result<Deck> {
    if normalize_first {
        normalize(dir)?;
    }
    load(dir, id)
}

pub fn load(dir: &Path, id: &str) -> Result<Deck> {
    let source = read_html(dir)?;
    let spans = html::find_slides(&source);
    let section_spans = html::find_sections(&source);
    let slides = spans
        .iter()
        .enumerate()
        .map(|(index, span)| Slide {
            // A slide the agent has not given an id yet is addressed by position until
            // the turn ends and `normalize` assigns one.
            id: span.id.clone().unwrap_or_else(|| format!("#{}", index + 1)),
            hash: html::content_hash(&source[span.range.clone()]),
            hidden: span.hidden.is_some(),
            moved: html::has_moved(&source[span.range.clone()]),
        })
        .collect();
    Ok(Deck {
        id: id.to_string(),
        title: html::title(&source).unwrap_or_else(|| fallback_title(id)),
        path: dir.to_string_lossy().into_owned(),
        shell_hash: html::shell_hash(&source, &spans, &section_spans),
        review: review::read(&source),
        sections: section_spans
            .iter()
            .enumerate()
            .map(|(index, span)| Section {
                index,
                title: span.title.clone(),
                before: span.before,
            })
            .collect(),
        slides,
    })
}

/// Saves deck.html and its optional narration source under matched snapshot stems.
pub fn snapshot(dir: &Path) -> Result<()> {
    let snapshots = dir.join(INTERNAL_DIR).join("snapshots");
    fs::create_dir_all(&snapshots)?;
    let stamp = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        uuid::Uuid::new_v4().simple()
    );
    fs::copy(dir.join(DECK_FILE), snapshots.join(format!("{stamp}.html")))?;
    let narration = dir.join(crate::narration::FILE);
    let archived = dir.join(INTERNAL_DIR).join("narration-snapshots");
    if narration.exists() {
        fs::create_dir_all(&archived)?;
        fs::copy(narration, archived.join(format!("{stamp}.json")))?;
    }
    prune_oldest(&snapshots, SNAPSHOTS_KEPT)?;
    if archived.exists() {
        for file in fs::read_dir(&archived)? {
            let path = file?.path();
            if let Some(stem) = path.file_stem() {
                if !snapshots.join(stem).with_extension("html").exists() {
                    fs::remove_file(path)?;
                }
            }
        }
    }
    Ok(())
}

/// Saves a screenshot of a sketched-on slide under `.slopslide/sketches/`, keeping the
/// newest few. Returns its deck-relative path, for the agent to read.
pub fn save_sketch(dir: &Path, png: &[u8]) -> Result<String> {
    let sketches = dir.join(INTERNAL_DIR).join("sketches");
    fs::create_dir_all(&sketches)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let short = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let name = format!("{stamp}-{short}.png");
    fs::write(sketches.join(&name), png)?;
    prune_oldest(&sketches, SKETCHES_KEPT)?;
    Ok(format!("{INTERNAL_DIR}/sketches/{name}"))
}

/// Deletes all but the `keep` newest files of `dir`, whose names start with a timestamp.
fn prune_oldest(dir: &Path, keep: usize) -> Result<()> {
    let mut files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    for old in files.iter().rev().skip(keep) {
        let _ = fs::remove_file(old);
    }
    Ok(())
}

type EditResult<T> = std::result::Result<(String, T), String>;

fn edit<T>(dir: &Path, id: &str, f: impl FnOnce(&str) -> EditResult<T>) -> Result<(Deck, T)> {
    let source = read_html(dir)?;
    let (updated, value) = f(&source).map_err(Error::Message)?;
    write_html(dir, &updated)?;
    Ok((load(dir, id)?, value))
}

/// Stores the user's review marks in deck.html, dropping those of slides that are gone.
/// Leaves the file alone when nothing changed, so the watcher stays quiet.
pub fn save_review(dir: &Path, review: &Review) -> Result<()> {
    let source = read_html(dir)?;
    let updated = review::write(&source, review);
    let updated = prune_review(&updated).unwrap_or(updated);
    if updated != source {
        write_html(dir, &updated)?;
    }
    Ok(())
}

pub fn rename(dir: &Path, id: &str, title: &str) -> Result<Deck> {
    let title = title.trim().to_string();
    Ok(edit(dir, id, |s| Ok((html::set_title(s, &title), ())))?.0)
}

pub fn reorder(dir: &Path, id: &str, slides: Vec<String>) -> Result<Deck> {
    Ok(edit(dir, id, |s| Ok((html::reorder(s, &slides)?, ())))?.0)
}

pub fn add_blank(dir: &Path, id: &str, after: Option<String>) -> Result<(Deck, String)> {
    edit(dir, id, |s| {
        html::insert(s, after.as_deref(), BLANK_SLIDE, "slide")
    })
}

pub fn duplicate(dir: &Path, id: &str, slide: &str) -> Result<(Deck, String)> {
    let source = read_html(dir)?;
    let (updated, copied) = html::duplicate(&source, slide).map_err(Error::Message)?;
    let span = html::find_slides(&updated)
        .into_iter()
        .find(|s| s.id.as_deref() == Some(&copied))
        .expect("duplicate exists");
    let hash = format!(
        "{}:{}",
        html::shell_hash(
            &updated,
            &html::find_slides(&updated),
            &html::find_sections(&updated)
        ),
        html::content_hash(&updated[span.range])
    );
    let original = load(dir, id)?;
    let from_hash = format!(
        "{}:{}",
        original.shell_hash,
        original
            .slides
            .iter()
            .find(|s| s.id == slide)
            .expect("source exists")
            .hash
    );
    crate::narration::duplicate(dir, slide, &copied, &from_hash, &hash)?;
    write_html(dir, &updated)?;
    Ok((load(dir, id)?, copied))
}

pub fn set_slide_hidden(dir: &Path, id: &str, slide: &str, hidden: bool) -> Result<Deck> {
    Ok(edit(dir, id, |s| Ok((html::set_hidden(s, slide, hidden)?, ())))?.0)
}

pub fn add_section(dir: &Path, id: &str, before: Option<String>, title: &str) -> Result<Deck> {
    Ok(edit(dir, id, |s| {
        Ok((html::add_section(s, before.as_deref(), title)?, ()))
    })?
    .0)
}

pub fn rename_section(dir: &Path, id: &str, index: usize, title: &str) -> Result<Deck> {
    Ok(edit(dir, id, |s| {
        Ok((html::rename_section(s, index, title)?, ()))
    })?
    .0)
}

pub fn delete_section(dir: &Path, id: &str, index: usize) -> Result<Deck> {
    Ok(edit(dir, id, |s| Ok((html::delete_section(s, index)?, ())))?.0)
}

pub fn delete_slide(dir: &Path, id: &str, slide: &str) -> Result<Deck> {
    snapshot(dir)?;
    Ok(edit(dir, id, |s| Ok((html::delete(s, slide)?, ())))?.0)
}

/// Replaces one slide with markup edited on the stage. `base` is the slide's hash the edit
/// started from; the save is refused when the slide has changed since. Returns the slide's
/// previous markup, so the edit can be undone by saving it back.
pub fn update_slide(
    dir: &Path,
    id: &str,
    slide: &str,
    markup: &str,
    base: &str,
) -> Result<(Deck, String)> {
    let source = read_html(dir)?;
    let current = html::find_slides(&source)
        .into_iter()
        .find(|s| s.id.as_deref() == Some(slide))
        .map(|s| html::content_hash(&source[s.range]))
        .ok_or_else(|| Error::msg(format!("Slide not found: {slide}")))?;
    if current != base {
        return Err(Error::msg(
            "The slide changed while you were editing it; your last change was not saved.",
        ));
    }
    snapshot(dir)?;
    edit(dir, id, |s| html::replace_slide(s, slide, markup))
}

/// Replaces deck.html with hand-edited source. `base` is the text the edit started from;
/// when given and the file has changed since (say, the agent wrote to it), the save is
/// refused so neither side's work is silently lost.
pub fn save_source(
    dir: &Path,
    id: &str,
    source: &str,
    base: Option<&str>,
    normalize_after: bool,
) -> Result<Deck> {
    if let Some(base) = base {
        if !same_text(&read_html(dir)?, base) {
            return Err(Error::msg(
                "deck.html changed on disk since you started editing",
            ));
        }
    }
    snapshot(dir)?;
    write_html(dir, source)?;
    if normalize_after {
        normalize(dir)?;
    }
    load(dir, id)
}

/// Equal up to line endings (the editor normalizes them to `\n`).
fn same_text(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n") == b.replace("\r\n", "\n")
}

pub fn delete_deck(dir: &Path) -> Result<()> {
    fs::remove_dir_all(dir)?;
    Ok(())
}

/// Writes a standalone copy of the deck with attached assets embedded as data URIs.
pub fn export(dir: &Path, dest: &Path) -> Result<()> {
    let source = html::ensure_runtime(&read_html(dir)?);
    let standalone = html::inline_assets(&source, |rel| {
        let path = resolve_in_deck(dir, rel).ok()?;
        Some((mime_for(rel).to_string(), fs::read(path).ok()?))
    });
    fs::write(dest, standalone)?;
    Ok(())
}

/// Creates a new folder for exported slide images inside `parent`, named after the deck
/// title (made safe for file systems); `Title 2`, `Title 3`, … if that name is taken.
pub fn create_export_dir(parent: &Path, title: &str) -> Result<PathBuf> {
    if !parent.is_dir() {
        return Err(Error::msg(format!("not a folder: {}", parent.display())));
    }
    let name = safe_file_name(title);
    let name = if name.is_empty() {
        "presentation".into()
    } else {
        name
    };
    for n in 1.. {
        let dir = parent.join(if n == 1 {
            name.clone()
        } else {
            format!("{name} {n}")
        });
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("unbounded")
}

/// `text` without characters file systems reject, trimmed of spaces and trailing dots.
fn safe_file_name(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control() && !r#"\/:*?"<>|"#.contains(*c))
        .collect();
    cleaned.trim().trim_end_matches('.').trim().to_string()
}

/// File name of the `index`th (0-based) of `total` exported slides: `slide-01.png`, with
/// enough digits that the files sort in slide order.
pub fn slide_image_name(index: usize, total: usize) -> String {
    let digits = total.max(1).to_string().len().max(2);
    format!("slide-{:0digits$}.png", index + 1)
}

/// Lints deck.html; asset references are checked against the deck's own files.
pub fn lint(dir: &Path) -> Result<Vec<lint::Issue>> {
    let source = read_html(dir)?;
    Ok(lint::lint(&source, |rel| {
        let rel = percent_encoding::percent_decode_str(rel).decode_utf8_lossy();
        resolve_in_deck(dir, &rel).is_ok_and(|path| path.is_file())
    }))
}

pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

pub fn import_assets(dir: &Path, paths: Vec<String>) -> Result<Vec<String>> {
    let assets = dir.join("assets");
    fs::create_dir_all(&assets)?;
    let mut imported = Vec::new();
    for source in paths {
        let source = PathBuf::from(source);
        if !source.is_file() {
            continue;
        }
        let stem = html::slugify(&source.file_stem().unwrap_or_default().to_string_lossy());
        let stem = if stem.is_empty() {
            "asset".to_string()
        } else {
            stem
        };
        let ext = source
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let dest = std::iter::once(assets.join(format!("{stem}{ext}")))
            .chain((2..).map(|n| assets.join(format!("{stem}-{n}{ext}"))))
            .find(|p| !p.exists())
            .expect("unbounded");
        fs::copy(&source, &dest)?;
        imported.push(format!(
            "assets/{}",
            dest.file_name().unwrap().to_string_lossy()
        ));
    }
    Ok(imported)
}

fn internal_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let dir = dir.join(INTERNAL_DIR);
    fs::create_dir_all(&dir)?;
    Ok(dir.join(name))
}

pub fn load_chat(dir: &Path) -> Result<serde_json::Value> {
    let path = internal_file(dir, "chat.json")?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Value::Null),
        Err(e) => Err(e.into()),
    }
}

pub fn save_chat(dir: &Path, chat: &serde_json::Value) -> Result<()> {
    let path = internal_file(dir, "chat.json")?;
    atomic_write(&path, serde_json::to_string(chat).expect("json").as_bytes())
}

/// Each agent provider keeps its own resumable session, stored under `name`.
pub fn read_session(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(INTERNAL_DIR).join(name))
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn write_session(dir: &Path, name: &str, session_id: Option<&str>) -> Result<()> {
    let path = dir.join(INTERNAL_DIR).join(name);
    match session_id {
        Some(id) => fs::write(path, id)?,
        None if path.exists() => fs::remove_file(path)?,
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_escapes() {
        let dir = Path::new("/tmp/deck");
        assert!(resolve_in_deck(dir, "../secret").is_err());
        assert!(resolve_in_deck(dir, "/etc/passwd").is_err());
        assert!(resolve_in_deck(dir, "assets/../../x").is_err());
        assert!(resolve_in_deck(dir, "assets/photo.png").is_ok());
    }

    #[test]
    fn plain_names_only() {
        assert!(is_plain_name("my-deck"));
        assert!(!is_plain_name("a/b"));
        assert!(!is_plain_name(".."));
    }

    /// A throwaway deck folder holding `html` as deck.html.
    struct TempDeck(PathBuf);

    impl TempDeck {
        fn new(html: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("slopslide-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(DECK_FILE), html).unwrap();
            TempDeck(dir)
        }
        fn html(&self) -> String {
            fs::read_to_string(self.0.join(DECK_FILE)).unwrap()
        }
        fn snapshots(&self) -> Vec<String> {
            let dir = self.0.join(INTERNAL_DIR).join("snapshots");
            let Ok(entries) = fs::read_dir(dir) else {
                return Vec::new();
            };
            entries
                .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
                .collect()
        }
    }

    impl Drop for TempDeck {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ORIGINAL: &str = "<html><head><title>Talk</title></head><body><main>\n<section class=\"slide\" id=\"a\">A</section>\n</main></body></html>";
    const EDITED: &str = "<html><head><title>Talk</title></head><body><main>\n<section class=\"slide\" id=\"a\">A!</section>\n<section class=\"slide\" id=\"b\">B</section>\n</main></body></html>";

    #[test]
    fn saves_source_when_base_matches() {
        let deck = TempDeck::new(ORIGINAL);
        let saved = save_source(&deck.0, "talk", EDITED, Some(ORIGINAL), false).unwrap();
        assert_eq!(deck.html(), EDITED);
        let ids: Vec<_> = saved.slides.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(saved.title, "Talk");
    }

    #[test]
    fn save_snapshots_the_previous_version() {
        let deck = TempDeck::new(ORIGINAL);
        save_source(&deck.0, "talk", EDITED, Some(ORIGINAL), false).unwrap();
        assert_eq!(deck.snapshots(), [ORIGINAL]);
    }

    #[test]
    fn save_refuses_when_file_changed_since_base() {
        let deck = TempDeck::new(ORIGINAL);
        let agent_version = ORIGINAL.replace(">A<", ">Agent<");
        fs::write(deck.0.join(DECK_FILE), &agent_version).unwrap();
        let err = save_source(&deck.0, "talk", EDITED, Some(ORIGINAL), false).unwrap_err();
        assert!(err.to_string().contains("changed on disk"), "{err}");
        assert_eq!(
            deck.html(),
            agent_version,
            "the agent's version must survive"
        );
        assert!(deck.snapshots().is_empty());
    }

    #[test]
    fn save_without_base_overwrites() {
        let deck = TempDeck::new(ORIGINAL);
        fs::write(deck.0.join(DECK_FILE), "<html>agent</html>").unwrap();
        save_source(&deck.0, "talk", EDITED, None, false).unwrap();
        assert_eq!(deck.html(), EDITED);
    }

    #[test]
    fn save_accepts_base_with_different_line_endings() {
        let deck = TempDeck::new(&ORIGINAL.replace('\n', "\r\n"));
        save_source(&deck.0, "talk", EDITED, Some(ORIGINAL), false).unwrap();
        assert_eq!(deck.html(), EDITED);
    }

    #[test]
    fn save_normalizes_when_asked() {
        let deck = TempDeck::new(ORIGINAL);
        let no_id = EDITED.replace(" id=\"b\"", "");
        let saved = save_source(&deck.0, "talk", &no_id, Some(ORIGINAL), true).unwrap();
        let html = deck.html();
        assert!(html.contains("slopslide:runtime-js"), "runtime installed");
        assert!(
            saved.slides.iter().all(|s| !s.id.starts_with('#')),
            "ids assigned"
        );
        assert_eq!(saved.slides.len(), 2);
        assert!(deck.0.join("assets").is_dir());
    }

    #[test]
    fn save_leaves_markup_alone_without_normalizing() {
        let deck = TempDeck::new(ORIGINAL);
        let no_id = EDITED.replace(" id=\"b\"", "");
        let saved = save_source(&deck.0, "talk", &no_id, None, false).unwrap();
        assert_eq!(deck.html(), no_id);
        assert_eq!(
            saved.slides[1].id, "#2",
            "unnamed slide addressed by position"
        );
    }

    #[test]
    fn update_slide_replaces_it_and_returns_the_old_markup() {
        let deck = TempDeck::new(EDITED);
        let base = load(&deck.0, "talk").unwrap().slides[0].hash.clone();
        let markup = "<section class=\"slide\" id=\"a\"><p data-moved=\"\" style=\"translate: 10px 0px\">A!</p></section>";
        let (saved, previous) = update_slide(&deck.0, "talk", "a", markup, &base).unwrap();
        assert_eq!(previous, "<section class=\"slide\" id=\"a\">A!</section>");
        assert_eq!(deck.html(), EDITED.replace(&previous, markup));
        assert_eq!(deck.snapshots(), [EDITED]);
        let moved: Vec<_> = saved.slides.iter().map(|s| s.moved).collect();
        assert_eq!(moved, [true, false]);
        assert_ne!(saved.slides[0].hash, base);

        // Undo: save the previous markup back on top of the new version.
        let (restored, _) =
            update_slide(&deck.0, "talk", "a", &previous, &saved.slides[0].hash).unwrap();
        assert_eq!(deck.html(), EDITED);
        assert_eq!(restored.slides[0].hash, base);
    }

    #[test]
    fn update_slide_refuses_when_the_slide_changed_since_base() {
        let deck = TempDeck::new(EDITED);
        let markup = "<section class=\"slide\" id=\"a\">mine</section>";
        let err = update_slide(&deck.0, "talk", "a", markup, "stale").unwrap_err();
        assert!(err.to_string().contains("changed"), "{err}");
        assert_eq!(deck.html(), EDITED);
        assert!(deck.snapshots().is_empty());

        let base = load(&deck.0, "talk").unwrap().slides[0].hash.clone();
        assert!(update_slide(&deck.0, "talk", "zz", markup, &base).is_err());
        let other = "<section class=\"slide\" id=\"b\">mine</section>";
        assert!(update_slide(&deck.0, "talk", "a", other, &base).is_err());
        assert_eq!(deck.html(), EDITED);
    }

    #[test]
    fn load_reports_hidden_slides() {
        let deck = TempDeck::new(&EDITED.replace(" id=\"b\"", " id=\"b\" data-hidden"));
        let loaded = load(&deck.0, "talk").unwrap();
        let hidden: Vec<_> = loaded.slides.iter().map(|s| s.hidden).collect();
        assert_eq!(hidden, [false, true]);
    }

    #[test]
    fn save_reports_missing_deck() {
        let deck = TempDeck::new(ORIGINAL);
        fs::remove_file(deck.0.join(DECK_FILE)).unwrap();
        assert!(save_source(&deck.0, "talk", EDITED, Some(ORIGINAL), false).is_err());
        assert!(!deck.0.join(DECK_FILE).exists());
    }

    #[test]
    fn edit_base_ignores_line_endings() {
        assert!(same_text("<p>\r\n</p>\r\n", "<p>\n</p>\n"));
        assert!(same_text("", ""));
        assert!(!same_text("<p>a</p>", "<p>b</p>"));
        assert!(!same_text("<p>\n</p>", "<p>\n\n</p>"));
    }

    #[test]
    fn template_is_a_valid_empty_deck() {
        let deck = html::ensure_runtime(&html::set_title(DECK_TEMPLATE, "Hello"));
        assert!(html::find_slides(&deck).is_empty());
        assert_eq!(html::title(&deck).as_deref(), Some("Hello"));
        let (with_slide, id) = html::insert(&deck, None, BLANK_SLIDE, "slide").unwrap();
        assert_eq!(id, "slide");
        assert_eq!(html::find_slides(&with_slide).len(), 1);
    }

    /// A throwaway library folder.
    struct TempLib(PathBuf);

    impl TempLib {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("slopslide-lib-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            TempLib(dir)
        }
        fn add(&self, id: &str, html: &str) -> PathBuf {
            let dir = self.0.join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(DECK_FILE), html).unwrap();
            dir
        }
    }

    impl Drop for TempLib {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const THREE: &str = "<html><head><title>Three</title></head><body><main class=\"deck\">\n  <section class=\"slide\" id=\"a\">A</section>\n  <section class=\"slide\" id=\"b\">B</section>\n  <section class=\"slide\" id=\"c\">C</section>\n</main></body></html>";

    fn slide_ids(deck: &Deck) -> Vec<&str> {
        deck.slides.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn resolve_accepts_nested_and_dot_paths() {
        let dir = Path::new("/tmp/deck");
        assert_eq!(
            resolve_in_deck(dir, "./assets/a b.png").unwrap(),
            dir.join("./assets/a b.png")
        );
        assert!(resolve_in_deck(dir, "deck.html").is_ok());
        assert!(resolve_in_deck(dir, "").is_err());
        assert!(resolve_in_deck(dir, "..").is_err());
    }

    #[test]
    fn deck_dir_validates_ids_and_requires_deck_html() {
        let lib = TempLib::new();
        lib.add("talk", ORIGINAL);
        fs::create_dir_all(lib.0.join("not-a-deck")).unwrap();
        assert_eq!(deck_dir_in(&lib.0, "talk").unwrap(), lib.0.join("talk"));
        for bad in ["", "..", ".", "a/b", "/abs", "../talk"] {
            let err = deck_dir_in(&lib.0, bad).unwrap_err().to_string();
            assert!(err.contains("invalid deck id"), "{bad}: {err}");
        }
        let err = deck_dir_in(&lib.0, "not-a-deck").unwrap_err().to_string();
        assert!(err.contains("deck not found"), "{err}");
        assert!(deck_dir_in(&lib.0, "missing").is_err());
    }

    #[test]
    fn lists_decks_newest_first_and_skips_other_folders() {
        let lib = TempLib::new();
        let old = lib.add("old-one", THREE);
        lib.add(
            "no-title",
            "<main><section class=\"slide\"></section></main>",
        );
        lib.add(
            "empty-deck",
            "<title>Empty</title><main class=\"deck\"></main>",
        );
        fs::create_dir_all(lib.0.join("stray-folder")).unwrap();
        fs::write(lib.0.join("loose-file.txt"), "x").unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(old.join(DECK_FILE))
            .unwrap()
            .set_modified(past)
            .unwrap();

        let decks = list(&lib.0).unwrap();
        let ids: Vec<_> = decks.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids.last(), Some(&"old-one"), "oldest last: {ids:?}");

        let three = decks.iter().find(|d| d.id == "old-one").unwrap();
        assert_eq!(three.title, "Three");
        assert_eq!(three.slide_count, 3);
        assert_eq!(three.first_slide.as_deref(), Some("a"));
        assert!(three.updated_ms > 0);

        let untitled = decks.iter().find(|d| d.id == "no-title").unwrap();
        assert_eq!(untitled.title, "no title", "falls back to the folder name");
        assert_eq!(
            untitled.first_slide, None,
            "slide without id has no addressable id"
        );

        let empty = decks.iter().find(|d| d.id == "empty-deck").unwrap();
        assert_eq!((empty.slide_count, empty.first_slide.as_deref()), (0, None));
    }

    #[test]
    fn lists_an_empty_library() {
        let lib = TempLib::new();
        assert!(list(&lib.0).unwrap().is_empty());
    }

    #[test]
    fn creates_decks_in_unique_slugged_folders() {
        let lib = TempLib::new();
        let first = create(&lib.0, "  Series A pitch!  ").unwrap();
        assert_eq!(first.id, "series-a-pitch");
        assert_eq!(first.title, "Series A pitch!");
        assert!(first.slides.is_empty());
        let second = create(&lib.0, "Series A Pitch").unwrap();
        assert_eq!(second.id, "series-a-pitch-2");
        let untitled = create(&lib.0, "   ").unwrap();
        assert_eq!(
            (untitled.id.as_str(), untitled.title.as_str()),
            ("untitled-deck", "Untitled deck")
        );
        let symbols = create(&lib.0, "日本語").unwrap();
        assert_eq!(symbols.id, "untitled");
        assert_eq!(symbols.title, "日本語");

        let dir = lib.0.join(&first.id);
        let html = fs::read_to_string(dir.join(DECK_FILE)).unwrap();
        assert!(html.contains("<title>Series A pitch!</title>"));
        assert!(
            html.contains("slopslide:runtime-js"),
            "created decks are normalized"
        );
        assert!(dir.join("assets").is_dir());
        assert_eq!(Path::new(&first.path), dir);
    }

    #[test]
    fn normalize_writes_app_files_and_is_idempotent() {
        let deck = TempDeck::new(&EDITED.replace(" id=\"b\"", ""));
        normalize(&deck.0).unwrap();
        for (name, body) in REFERENCE_DOCS {
            let path = deck.0.join(INTERNAL_DIR).join("reference").join(name);
            assert_eq!(fs::read_to_string(path).unwrap(), *body);
        }
        assert!(deck.0.join("assets").is_dir());
        let once = deck.html();
        assert!(once.contains("id=\"slide-2\""));
        normalize(&deck.0).unwrap();
        assert_eq!(deck.html(), once);
    }

    #[test]
    fn normalize_does_not_rewrite_an_already_tidy_deck() {
        let deck = TempDeck::new(ORIGINAL);
        normalize(&deck.0).unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        let file = deck.0.join(DECK_FILE);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(past)
            .unwrap();
        let before = modified_ms(&file);
        normalize(&deck.0).unwrap();
        assert_eq!(
            modified_ms(&file),
            before,
            "no write, so the watcher stays quiet"
        );
    }

    #[test]
    fn open_only_normalizes_when_asked() {
        let deck = TempDeck::new(&EDITED.replace(" id=\"b\"", ""));
        let raw = open(&deck.0, "talk", false).unwrap();
        assert_eq!(slide_ids(&raw), ["a", "#2"]);
        assert!(!deck.html().contains("slopslide:runtime"));
        let tidy = open(&deck.0, "talk", true).unwrap();
        assert_eq!(slide_ids(&tidy), ["a", "slide-2"]);
    }

    #[test]
    fn load_hashes_change_per_slide() {
        let deck = TempDeck::new(THREE);
        let before = load(&deck.0, "three").unwrap();
        assert_eq!(before.id, "three");
        assert_eq!(before.title, "Three");
        fs::write(deck.0.join(DECK_FILE), THREE.replace(">B<", ">Bee<")).unwrap();
        let after = load(&deck.0, "three").unwrap();
        let changed: Vec<_> = before
            .slides
            .iter()
            .zip(&after.slides)
            .filter(|(a, b)| a.hash != b.hash)
            .map(|(a, _)| a.id.as_str())
            .collect();
        assert_eq!(changed, ["b"]);
        assert_eq!(before.shell_hash, after.shell_hash);

        fs::write(
            deck.0.join(DECK_FILE),
            THREE.replace("<head>", "<head><style>x</style>"),
        )
        .unwrap();
        let restyled = load(&deck.0, "three").unwrap();
        assert_ne!(before.shell_hash, restyled.shell_hash);
        assert!(before
            .slides
            .iter()
            .zip(&restyled.slides)
            .all(|(a, b)| a.hash == b.hash));
    }

    #[test]
    fn load_falls_back_to_the_id_for_the_title() {
        let deck = TempDeck::new("<main></main>");
        assert_eq!(load(&deck.0, "my-talk").unwrap().title, "my talk");
    }

    #[test]
    fn deck_serializes_in_camel_case_for_the_frontend() {
        let deck = TempDeck::new(ORIGINAL);
        let json = serde_json::to_value(load(&deck.0, "talk").unwrap()).unwrap();
        assert!(json["shellHash"].is_string());
        assert_eq!(json["slides"][0]["id"], "a");
        assert!(json["slides"][0]["hash"].is_string());
        let summary = serde_json::to_value(DeckSummary {
            id: "x".into(),
            title: "X".into(),
            slide_count: 2,
            first_slide: None,
            updated_ms: 5,
        })
        .unwrap();
        assert_eq!(
            summary,
            serde_json::json!({"id":"x","title":"X","slideCount":2,"firstSlide":null,"updatedMs":5})
        );
    }

    #[test]
    fn snapshots_keep_only_the_newest() {
        let deck = TempDeck::new(ORIGINAL);
        let snapshots = deck.0.join(INTERNAL_DIR).join("snapshots");
        fs::create_dir_all(&snapshots).unwrap();
        // Older snapshots, named by timestamp like real ones.
        for n in 0..SNAPSHOTS_KEPT + 5 {
            fs::write(
                snapshots.join(format!("{}.html", 1_000_000_000_000u64 + n as u64)),
                "old",
            )
            .unwrap();
        }
        snapshot(&deck.0).unwrap();
        let mut names: Vec<_> = fs::read_dir(&snapshots)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), SNAPSHOTS_KEPT);
        assert!(
            !names.contains(&"1000000000000.html".to_string()),
            "oldest pruned"
        );
        let newest = snapshots.join(names.last().unwrap());
        assert_eq!(fs::read_to_string(newest).unwrap(), ORIGINAL);
    }

    #[test]
    fn save_sketch_writes_a_png_and_prunes_old_ones() {
        let deck = TempDeck::new(ORIGINAL);
        let sketches = deck.0.join(INTERNAL_DIR).join("sketches");
        fs::create_dir_all(&sketches).unwrap();
        for n in 0..SKETCHES_KEPT + 3 {
            let name = format!("{}-old.png", 1_000_000_000_000u64 + n as u64);
            fs::write(sketches.join(name), b"old").unwrap();
        }
        let rel = save_sketch(&deck.0, b"\x89PNG fake").unwrap();
        assert!(
            rel.starts_with(".slopslide/sketches/") && rel.ends_with(".png"),
            "{rel}"
        );
        assert_eq!(fs::read(deck.0.join(&rel)).unwrap(), b"\x89PNG fake");
        let names: Vec<_> = fs::read_dir(&sketches)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), SKETCHES_KEPT);
        assert!(
            !names.contains(&"1000000000000-old.png".to_string()),
            "oldest pruned"
        );
        assert!(
            rel.ends_with(names.iter().max().unwrap().as_str()),
            "new sketch kept"
        );
    }

    #[test]
    fn export_dir_is_named_after_the_deck() {
        let parent = TempDeck::new(ORIGINAL);
        let dir = create_export_dir(&parent.0, "Q3 Review").unwrap();
        assert_eq!(dir, parent.0.join("Q3 Review"));
        assert!(dir.is_dir());
    }

    #[test]
    fn export_dir_never_reuses_an_existing_folder() {
        let parent = TempDeck::new(ORIGINAL);
        fs::create_dir(parent.0.join("Talk")).unwrap();
        fs::write(parent.0.join("Talk").join("slide-01.png"), b"keep").unwrap();
        assert_eq!(
            create_export_dir(&parent.0, "Talk").unwrap(),
            parent.0.join("Talk 2")
        );
        assert_eq!(
            create_export_dir(&parent.0, "Talk").unwrap(),
            parent.0.join("Talk 3")
        );
        assert_eq!(
            fs::read(parent.0.join("Talk").join("slide-01.png")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn export_dir_names_are_safe() {
        let parent = TempDeck::new(ORIGINAL);
        let dir = create_export_dir(&parent.0, " A/B: \"why?\" <draft>. ").unwrap();
        assert_eq!(dir.file_name().unwrap(), "AB why draft");
        let dir = create_export_dir(&parent.0, "../..").unwrap();
        assert_eq!(dir, parent.0.join("presentation"));
        let dir = create_export_dir(&parent.0, "\t").unwrap();
        assert_eq!(dir, parent.0.join("presentation 2"));
    }

    #[test]
    fn export_dir_needs_an_existing_parent() {
        let parent = TempDeck::new(ORIGINAL);
        assert!(create_export_dir(&parent.0.join("missing"), "Talk").is_err());
        assert!(create_export_dir(&parent.0.join(DECK_FILE), "Talk").is_err());
    }

    #[test]
    fn slide_images_sort_in_slide_order() {
        assert_eq!(slide_image_name(0, 1), "slide-01.png");
        assert_eq!(slide_image_name(8, 12), "slide-09.png");
        assert_eq!(slide_image_name(99, 120), "slide-100.png");
        assert_eq!(slide_image_name(4, 120), "slide-005.png");
        assert_eq!(slide_image_name(0, 0), "slide-01.png");
    }

    fn marks(ids: &[&str]) -> Review {
        ids.iter()
            .map(|id| {
                let stroke = review::Stroke {
                    tool: review::InkTool::Highlighter,
                    color: "#facc15".into(),
                    points: vec![[0.25, 0.5], [0.75, 0.5]],
                };
                (id.to_string(), vec![stroke])
            })
            .collect()
    }

    #[test]
    fn review_marks_are_saved_in_the_deck_and_loaded_back() {
        let deck = TempDeck::new(THREE);
        save_review(&deck.0, &marks(&["b", "gone"])).unwrap();
        let loaded = load(&deck.0, "three").unwrap();
        assert_eq!(
            loaded.review,
            marks(&["b"]),
            "marks of missing slides are dropped"
        );
        assert!(deck.html().contains(review::START));

        save_review(&deck.0, &Review::new()).unwrap();
        assert_eq!(deck.html(), THREE, "clearing every mark removes the block");
    }

    #[test]
    fn saving_unchanged_review_marks_leaves_the_file_alone() {
        let deck = TempDeck::new(THREE);
        save_review(&deck.0, &marks(&["a"])).unwrap();
        let path = deck.0.join(DECK_FILE);
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save_review(&deck.0, &marks(&["a"])).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn review_marks_leave_slide_and_shell_hashes_alone() {
        let deck = TempDeck::new(THREE);
        let before = load(&deck.0, "three").unwrap();
        save_review(&deck.0, &marks(&["a", "c"])).unwrap();
        let after = load(&deck.0, "three").unwrap();
        assert_eq!(before.shell_hash, after.shell_hash);
        assert!(before
            .slides
            .iter()
            .zip(&after.slides)
            .all(|(a, b)| a.hash == b.hash));
    }

    #[test]
    fn normalize_drops_marks_of_deleted_slides() {
        let deck = TempDeck::new(&review::write(THREE, &marks(&["a", "b"])));
        fs::write(
            deck.0.join(DECK_FILE),
            deck.html()
                .replace("<section class=\"slide\" id=\"b\">B</section>", ""),
        )
        .unwrap();
        normalize(&deck.0).unwrap();
        assert_eq!(load(&deck.0, "three").unwrap().review, marks(&["a"]));
    }

    #[test]
    fn save_sketch_names_are_unique() {
        let deck = TempDeck::new(ORIGINAL);
        let a = save_sketch(&deck.0, b"a").unwrap();
        let b = save_sketch(&deck.0, b"b").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn snapshot_fails_without_a_deck_file() {
        let deck = TempDeck::new(ORIGINAL);
        fs::remove_file(deck.0.join(DECK_FILE)).unwrap();
        assert!(snapshot(&deck.0).is_err());
    }

    #[test]
    fn rename_trims_and_escapes() {
        let deck = TempDeck::new(THREE);
        let renamed = rename(&deck.0, "three", "  R&D <2025>  ").unwrap();
        assert_eq!(renamed.title, "R&D <2025>");
        assert!(deck.html().contains("<title>R&amp;D &lt;2025&gt;</title>"));
        assert_eq!(slide_ids(&renamed), ["a", "b", "c"]);
    }

    #[test]
    fn reorder_persists_and_refuses_stale_orders() {
        let deck = TempDeck::new(THREE);
        let order = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(
            slide_ids(&reorder(&deck.0, "three", order).unwrap()),
            ["c", "a", "b"]
        );
        assert_eq!(slide_ids(&load(&deck.0, "three").unwrap()), ["c", "a", "b"]);
        let before = deck.html();
        let err = reorder(&deck.0, "three", vec!["a".into(), "b".into()]).unwrap_err();
        assert!(err.to_string().contains("try again"), "{err}");
        assert_eq!(deck.html(), before, "a refused edit writes nothing");
    }

    #[test]
    fn add_blank_inserts_after_the_given_slide() {
        let deck = TempDeck::new(THREE);
        let (after_a, id) = add_blank(&deck.0, "three", Some("a".into())).unwrap();
        assert_eq!(id, "slide");
        assert_eq!(slide_ids(&after_a), ["a", "slide", "b", "c"]);
        let (at_end, id) = add_blank(&deck.0, "three", None).unwrap();
        assert_eq!(id, "slide-2");
        assert_eq!(slide_ids(&at_end), ["a", "slide", "b", "c", "slide-2"]);
        assert!(deck.html().contains("Untitled slide"));
    }

    #[test]
    fn duplicate_and_delete_slides_on_disk() {
        let deck = TempDeck::new(THREE);
        let (copied, id) = duplicate(&deck.0, "three", "b").unwrap();
        assert_eq!(id, "b-copy");
        assert_eq!(slide_ids(&copied), ["a", "b", "b-copy", "c"]);
        assert!(deck.html().contains("id=\"b-copy\">B</section>"));

        let before_delete = deck.html();
        let deleted = delete_slide(&deck.0, "three", "b").unwrap();
        assert_eq!(slide_ids(&deleted), ["a", "b-copy", "c"]);
        assert_eq!(
            deck.snapshots(),
            [before_delete],
            "deleting keeps a snapshot to recover from"
        );

        assert!(duplicate(&deck.0, "three", "nope").is_err());
        assert!(delete_slide(&deck.0, "three", "nope").is_err());
    }

    #[test]
    fn delete_deck_removes_the_folder() {
        let lib = TempLib::new();
        let dir = lib.add("gone", ORIGINAL);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/a.png"), "x").unwrap();
        delete_deck(&dir).unwrap();
        assert!(!dir.exists());
        assert!(delete_deck(&dir).is_err());
    }

    #[test]
    fn lint_checks_assets_on_disk() {
        let deck = TempDeck::new(
            "<!DOCTYPE html><html><head><title>T</title></head><body><main class=\"deck\"><section class=\"slide\" id=\"a\"><img src=\"assets/my%20dot.png\" alt=\"\"><img src=\"assets/missing.png\" alt=\"\"><img src=\"assets/../deck.html\" alt=\"\"></section></main></body></html>",
        );
        fs::create_dir_all(deck.0.join("assets")).unwrap();
        fs::write(deck.0.join("assets/my dot.png"), [0]).unwrap();
        let missing: Vec<_> = lint(&deck.0)
            .unwrap()
            .into_iter()
            .filter(|i| i.rule == "missing-asset")
            .map(|i| i.message)
            .collect();
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(missing[0].contains("assets/missing.png"));
        assert!(
            missing[1].contains("assets/../deck.html"),
            "escaping paths count as missing"
        );
        assert!(lint(&deck.0.join("nope")).is_err());
    }

    #[test]
    fn export_inlines_assets_and_installs_the_runtime() {
        let deck = TempDeck::new(
            "<html><head></head><body><main class=\"deck\"><section class=\"slide\" id=\"a\"><img src=\"assets/dot.png\"><img src=\"assets/missing.png\"><img src=\"assets/../../escape.png\"></section></main></body></html>",
        );
        fs::create_dir_all(deck.0.join("assets")).unwrap();
        fs::write(deck.0.join("assets/dot.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let dest = deck.0.join("out.html");
        export(&deck.0, &dest).unwrap();
        let out = fs::read_to_string(&dest).unwrap();
        assert!(
            out.contains("src=\"data:image/png;base64,iVBORw==\""),
            "{out}"
        );
        assert!(
            out.contains("src=\"assets/missing.png\""),
            "missing assets are left as-is"
        );
        assert!(
            out.contains("src=\"assets/../../escape.png\""),
            "escaping paths are not read"
        );
        assert!(out.contains("slopslide:runtime-js"));
        assert!(
            !deck.html().contains("data:"),
            "the deck itself is unchanged"
        );
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for("deck.html"), "text/html; charset=utf-8");
        assert_eq!(mime_for("assets/PHOTO.JPG"), "image/jpeg");
        assert_eq!(mime_for("assets/a.jpeg"), "image/jpeg");
        assert_eq!(mime_for("assets/logo.svg"), "image/svg+xml");
        assert_eq!(mime_for("assets/font.woff2"), "font/woff2");
        assert_eq!(mime_for("assets/clip.webm"), "video/webm");
        assert_eq!(mime_for("assets/data.json"), "application/json");
        assert_eq!(mime_for("assets/x.tar.gz"), "application/octet-stream");
        assert_eq!(mime_for("assets/no-extension"), "application/octet-stream");
        assert_eq!(mime_for(""), "application/octet-stream");
    }

    #[test]
    fn imports_assets_with_slugged_unique_names() {
        let deck = TempDeck::new(ORIGINAL);
        let src = TempLib::new();
        let photo = src.0.join("My Photo.PNG");
        fs::write(&photo, "png").unwrap();
        let nameless = src.0.join("日本.jpg");
        fs::write(&nameless, "jpg").unwrap();
        let no_ext = src.0.join("README");
        fs::write(&no_ext, "txt").unwrap();
        let path = |p: &PathBuf| p.to_string_lossy().into_owned();

        let imported = import_assets(
            &deck.0,
            vec![
                path(&photo),
                path(&photo),
                path(&nameless),
                path(&no_ext),
                path(&src.0.join("does-not-exist.png")),
                path(&src.0),
            ],
        )
        .unwrap();
        assert_eq!(
            imported,
            [
                "assets/my-photo.png",
                "assets/my-photo-2.png",
                "assets/asset.jpg",
                "assets/readme"
            ]
        );
        assert_eq!(
            fs::read_to_string(deck.0.join("assets/my-photo-2.png")).unwrap(),
            "png"
        );
        assert!(import_assets(&deck.0, vec![]).unwrap().is_empty());
    }

    #[test]
    fn chat_round_trips_and_tolerates_bad_files() {
        let deck = TempDeck::new(ORIGINAL);
        assert_eq!(load_chat(&deck.0).unwrap(), serde_json::Value::Null);
        let chat = serde_json::json!([{"id": "1", "role": "user", "text": "hi ✨"}]);
        save_chat(&deck.0, &chat).unwrap();
        assert_eq!(load_chat(&deck.0).unwrap(), chat);
        fs::write(deck.0.join(INTERNAL_DIR).join("chat.json"), "{not json").unwrap();
        assert_eq!(load_chat(&deck.0).unwrap(), serde_json::Value::Null);
        save_chat(&deck.0, &serde_json::Value::Null).unwrap();
        assert_eq!(load_chat(&deck.0).unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn session_is_stored_trimmed_and_cleared() {
        let deck = TempDeck::new(ORIGINAL);
        fs::create_dir_all(deck.0.join(INTERNAL_DIR)).unwrap();
        assert_eq!(read_session(&deck.0, "session"), None);
        write_session(&deck.0, "session", Some("abc-123")).unwrap();
        assert_eq!(read_session(&deck.0, "session").as_deref(), Some("abc-123"));
        fs::write(deck.0.join(INTERNAL_DIR).join("session"), "  xyz\n").unwrap();
        assert_eq!(read_session(&deck.0, "session").as_deref(), Some("xyz"));
        write_session(&deck.0, "session", None).unwrap();
        assert_eq!(read_session(&deck.0, "session"), None);
        write_session(&deck.0, "session", None).unwrap();
    }

    #[test]
    fn sessions_are_kept_per_provider() {
        let deck = TempDeck::new(ORIGINAL);
        fs::create_dir_all(deck.0.join(INTERNAL_DIR)).unwrap();
        write_session(&deck.0, "session", Some("claude-1")).unwrap();
        write_session(&deck.0, "codex-session", Some("codex-1")).unwrap();
        assert_eq!(
            read_session(&deck.0, "session").as_deref(),
            Some("claude-1")
        );
        assert_eq!(
            read_session(&deck.0, "codex-session").as_deref(),
            Some("codex-1")
        );
    }

    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let deck = TempDeck::new(ORIGINAL);
        write_html(&deck.0, EDITED).unwrap();
        assert_eq!(deck.html(), EDITED);
        let names: Vec<_> = fs::read_dir(&deck.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [DECK_FILE]);
    }

    const SECTIONED: &str = "<html><head><title>S</title></head><body><main class=\"deck\">\n  <section class=\"slide\" id=\"a\">A</section>\n  <div class=\"deck-section\" data-title=\"Part two\"></div>\n  <section class=\"slide\" id=\"b\">B</section>\n  <section class=\"slide\" id=\"c\">C</section>\n</main></body></html>";

    fn sections(deck: &Deck) -> Vec<(usize, &str, usize)> {
        deck.sections
            .iter()
            .map(|s| (s.index, s.title.as_str(), s.before))
            .collect()
    }

    #[test]
    fn load_lists_sections_without_counting_them_as_slides() {
        let deck = TempDeck::new(SECTIONED);
        let loaded = load(&deck.0, "s").unwrap();
        assert_eq!(slide_ids(&loaded), ["a", "b", "c"]);
        assert_eq!(sections(&loaded), [(0, "Part two", 1)]);
        assert!(load(&TempDeck::new(THREE).0, "three")
            .unwrap()
            .sections
            .is_empty());
        let json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(json["sections"][0]["title"], "Part two");
        assert_eq!(json["sections"][0]["before"], 1);
    }

    #[test]
    fn adds_renames_and_deletes_sections() {
        let deck = TempDeck::new(THREE);
        let added = add_section(&deck.0, "three", Some("c".into()), "Finale").unwrap();
        assert_eq!(sections(&added), [(0, "Finale", 2)]);
        let added = add_section(&deck.0, "three", Some("a".into()), "Start").unwrap();
        assert_eq!(sections(&added), [(0, "Start", 0), (1, "Finale", 2)]);
        let renamed = rename_section(&deck.0, "three", 1, "The end").unwrap();
        assert_eq!(sections(&renamed), [(0, "Start", 0), (1, "The end", 2)]);
        let removed = delete_section(&deck.0, "three", 0).unwrap();
        assert_eq!(sections(&removed), [(0, "The end", 2)]);
        assert_eq!(slide_ids(&removed), ["a", "b", "c"]);
        assert!(delete_section(&deck.0, "three", 4).is_err());
        assert!(rename_section(&deck.0, "three", 4, "x").is_err());
        assert!(add_section(&deck.0, "three", Some("zzz".into()), "x").is_err());
    }

    #[test]
    fn section_edits_do_not_reload_slide_previews() {
        let deck = TempDeck::new(SECTIONED);
        let before = load(&deck.0, "s").unwrap();
        let renamed = rename_section(&deck.0, "s", 0, "Renamed").unwrap();
        assert_eq!(before.shell_hash, renamed.shell_hash);
        assert!(before
            .slides
            .iter()
            .zip(&renamed.slides)
            .all(|(a, b)| a.hash == b.hash));
    }

    #[test]
    fn reorder_slides_and_sections_together() {
        let deck = TempDeck::new(SECTIONED);
        let order = ["a", "b", "section:0", "c"].map(String::from).to_vec();
        let out = reorder(&deck.0, "s", order).unwrap();
        assert_eq!(slide_ids(&out), ["a", "b", "c"]);
        assert_eq!(sections(&out), [(0, "Part two", 2)]);
        assert!(reorder(&deck.0, "s", vec!["a".into(), "b".into(), "c".into()]).is_err());
    }

    #[test]
    fn export_keeps_section_markers_for_the_player_to_hide() {
        let deck = TempDeck::new(SECTIONED);
        let dest = deck.0.join("out.html");
        export(&deck.0, &dest).unwrap();
        let exported = fs::read_to_string(dest).unwrap();
        assert!(exported.contains("data-title=\"Part two\""));
        assert!(exported.contains(".deck > .deck-section"));
    }
}
