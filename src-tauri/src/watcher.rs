//! Watches the open deck folder and tells the frontend which files changed, so slide
//! iframes refresh while the agent (or any editor) writes to disk.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::deck::INTERNAL_DIR;
use crate::error::{Error, Result};

#[derive(Default)]
pub struct DeckWatcher(Mutex<Option<Debouncer<notify::RecommendedWatcher>>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeckChanged {
    deck_id: String,
    paths: Vec<String>,
}

impl DeckWatcher {
    pub fn watch(&self, app: AppHandle, deck_id: String, dir: PathBuf) -> Result<()> {
        // Deleted files cannot be canonicalized, so match against both spellings of the root.
        let roots = [
            dir.canonicalize().unwrap_or_else(|_| dir.clone()),
            dir.clone(),
        ];
        let mut debouncer = new_debouncer(
            Duration::from_millis(120),
            move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                let paths = changed_paths(&roots, events.iter().map(|e| e.path.as_path()));
                if !paths.is_empty() {
                    let _ = app.emit(
                        "deck-changed",
                        DeckChanged {
                            deck_id: deck_id.clone(),
                            paths,
                        },
                    );
                }
            },
        )
        .map_err(|e| Error::msg(format!("cannot watch deck: {e}")))?;
        debouncer
            .watcher()
            .watch(&dir, RecursiveMode::Recursive)
            .map_err(|e| Error::msg(format!("cannot watch deck: {e}")))?;
        *self.0.lock().unwrap() = Some(debouncer);
        Ok(())
    }

    pub fn stop(&self) {
        self.0.lock().unwrap().take();
    }
}

/// Deck-relative paths worth telling the frontend about, sorted and deduplicated.
fn changed_paths<'a>(roots: &[PathBuf], events: impl Iterator<Item = &'a Path>) -> Vec<String> {
    let mut paths: Vec<String> = events
        .filter_map(|path| roots.iter().find_map(|root| relative(root, path)))
        .filter(|rel| !rel.starts_with(INTERNAL_DIR) && !is_temp_file(rel))
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Atomic-write temp files: ours (`deck.tmp-<uuid>`) and Claude Code's (`x.html.tmp.<pid>.<hash>`).
fn is_temp_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.contains(".tmp-") || name.contains(".tmp.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_atomic_write_temp_files() {
        assert!(is_temp_file("slides/01-title.html.tmp.18967.9f8e072fc565"));
        assert!(is_temp_file("deck.tmp-0a1b2c"));
        assert!(!is_temp_file("slides/01-title.html"));
        assert!(!is_temp_file("assets/template.png"));
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("slopslide-watch-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(dir.join("assets")).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = TempDir::new();
        let file = root.0.join("assets").join("a.png");
        std::fs::write(&file, "x").unwrap();
        // Existing paths are canonicalized (e.g. /var -> /private/var on macOS).
        let canonical = root.0.canonicalize().unwrap();
        assert_eq!(relative(&canonical, &file).as_deref(), Some("assets/a.png"));
        assert_eq!(
            relative(&canonical, &root.0),
            None,
            "the root itself is not a change"
        );
        assert_eq!(
            relative(&canonical, Path::new("/elsewhere/deck.html")),
            None
        );
    }

    #[test]
    fn relative_handles_deleted_files() {
        let root = TempDir::new();
        // A deleted file cannot be canonicalized; matching uses the path as given.
        let gone = root.0.join("deck.html");
        assert_eq!(relative(&root.0, &gone).as_deref(), Some("deck.html"));
    }

    #[test]
    fn changed_paths_filters_sorts_and_dedups() {
        let root = TempDir::new();
        let roots = [root.0.canonicalize().unwrap(), root.0.clone()];
        let deck = root.0.join("deck.html");
        std::fs::write(&deck, "x").unwrap();
        let events = [
            deck.clone(),
            root.0.join("assets/b.png"),
            root.0.join(INTERNAL_DIR).join("chat.json"),
            root.0.join(INTERNAL_DIR).join("snapshots/1.html"),
            root.0.join("deck.tmp-0a1b2c"),
            root.0.join("narration.json"),
            root.0.join("narration.tmp-0a1b2c"),
            root.0.join("assets/a.png"),
            deck.clone(),
            PathBuf::from("/somewhere/else.html"),
            root.0.clone(),
        ];
        assert_eq!(
            changed_paths(&roots, events.iter().map(PathBuf::as_path)),
            [
                "assets/a.png",
                "assets/b.png",
                "deck.html",
                "narration.json"
            ]
        );
        assert!(changed_paths(&roots, std::iter::empty()).is_empty());
    }

    #[test]
    fn temp_file_detection_looks_at_the_file_name_only() {
        assert!(!is_temp_file("my.tmp-dir/deck.html"));
        assert!(is_temp_file("assets/photo.png.tmp.1.2"));
        assert!(!is_temp_file("assets/template.html"));
    }
}
