//! Slide templates: decks whose slides are example layouts (title, section, bullets, …) in
//! one style. A deck names the template its design comes from in a `<meta>` (see
//! [`html::TEMPLATE_META`]); the editor offers that template's slides as layouts for new
//! slides and changed ones, and its styles for restyling the deck.
//!
//! Built-in templates ship inside the app. The user's own live in
//! `~/.slopslides/templates/<id>/deck.html` (with an optional `assets/` folder), one folder
//! per template; a user template replaces a built-in one with the same id.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::deck::{self, DECK_FILE, INTERNAL_DIR};
use crate::error::{Error, Result};
use crate::html;
use crate::review;

const BUILTIN: &[(&str, &str)] = &[
    (
        "claymorphism",
        include_str!("../templates/claymorphism.html"),
    ),
    ("cybercore", include_str!("../templates/cybercore.html")),
    (
        "neo-brutalism",
        include_str!("../templates/neo-brutalism.html"),
    ),
    ("scrapbook", include_str!("../templates/scrapbook.html")),
    ("surrealism", include_str!("../templates/surrealism.html")),
    ("y2k", include_str!("../templates/y2k.html")),
    ("pixel-art", include_str!("../templates/pixel-art.html")),
    ("synthwave", include_str!("../templates/synthwave.html")),
    (
        "glassmorphism",
        include_str!("../templates/glassmorphism.html"),
    ),
    ("neumorphism", include_str!("../templates/neumorphism.html")),
    ("bento-grid", include_str!("../templates/bento-grid.html")),
    ("editorial", include_str!("../templates/editorial.html")),
    ("swiss", include_str!("../templates/swiss.html")),
    ("minimalism", include_str!("../templates/minimalism.html")),
    ("maximalism", include_str!("../templates/maximalism.html")),
    (
        "luxury-typography",
        include_str!("../templates/luxury-typography.html"),
    ),
    (
        "conceptual-sketch",
        include_str!("../templates/conceptual-sketch.html"),
    ),
    ("ethereal", include_str!("../templates/ethereal.html")),
    ("bohemian", include_str!("../templates/bohemian.html")),
    ("victorian", include_str!("../templates/victorian.html")),
    ("cyberpunk", include_str!("../templates/cyberpunk.html")),
    ("wabi-sabi", include_str!("../templates/wabi-sabi.html")),
];

/// Where staged copies of templates go inside a deck, for the agent to read.
pub const STAGED_DIR: &str = "templates";

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSummary {
    pub id: String,
    pub title: String,
    /// Ships with the app (as opposed to the user's own, under `~/.slopslides/templates`).
    pub builtin: bool,
    /// Folder of a user template.
    pub path: Option<String>,
    /// Ids of the template's slides: one per layout, in order.
    pub slides: Vec<String>,
}

/// `~/.slopslides/templates`, created on first use.
pub fn user_root(app: &AppHandle) -> Result<PathBuf> {
    let home = app
        .path()
        .home_dir()
        .map_err(|e| Error::msg(format!("cannot locate home folder: {e}")))?;
    let root = home.join(".slopslides").join("templates");
    fs::create_dir_all(&root)?;
    Ok(root)
}

/// Template ids are folder names that are safe in a URL path segment and an attribute.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && !id.contains(['/', '\\', '"', '\'', '<', '>', '&', '?', '#', '%'])
}

fn builtin(id: &str) -> Option<&'static str> {
    BUILTIN
        .iter()
        .find(|(b, _)| *b == id)
        .map(|(_, html)| *html)
}

/// Folder of the user template `id`, if there is one.
pub fn user_dir(root: &Path, id: &str) -> Option<PathBuf> {
    if !is_valid_id(id) {
        return None;
    }
    let dir = root.join(id);
    dir.join(DECK_FILE).is_file().then_some(dir)
}

/// The template's deck.html as stored (user templates win over built-in ones).
pub fn source(root: &Path, id: &str) -> Result<String> {
    if let Some(dir) = user_dir(root, id) {
        return Ok(fs::read_to_string(dir.join(DECK_FILE))?);
    }
    builtin(id)
        .map(str::to_string)
        .ok_or_else(|| Error::msg(format!("template not found: {id}")))
}

fn summary(id: &str, source: &str, path: Option<&Path>) -> TemplateSummary {
    TemplateSummary {
        id: id.to_string(),
        title: html::title(source).unwrap_or_else(|| id.replace('-', " ")),
        builtin: path.is_none(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        slides: html::find_slides(source)
            .into_iter()
            .filter_map(|s| s.id)
            .collect(),
    }
}

/// The user's templates (by title), then the built-in ones they do not replace.
pub fn list(root: &Path) -> Vec<TemplateSummary> {
    let mut own: Vec<TemplateSummary> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry.ok()?.file_name().to_str()?.to_string();
            let dir = user_dir(root, &id)?;
            let source = fs::read_to_string(dir.join(DECK_FILE)).ok()?;
            Some(summary(&id, &source, Some(&dir)))
        })
        .filter(|t| !t.slides.is_empty())
        .collect();
    own.sort_by_key(|t| t.title.to_lowercase());
    let builtins: Vec<TemplateSummary> = BUILTIN
        .iter()
        .filter(|(id, _)| !own.iter().any(|t| t.id == *id))
        .map(|(id, source)| summary(id, source, None))
        .collect();
    own.extend(builtins);
    own
}

/// Copies the template into the deck's internals for the agent to read; returns its
/// deck-relative path.
pub fn stage(deck_dir: &Path, root: &Path, id: &str) -> Result<String> {
    if !is_valid_id(id) {
        return Err(Error::msg(format!("invalid template id: {id}")));
    }
    let source = html::without_runtime(&source(root, id)?);
    let dir = deck_dir.join(INTERNAL_DIR).join(STAGED_DIR);
    fs::create_dir_all(&dir)?;
    let name = format!("{id}.html");
    if fs::read_to_string(dir.join(&name)).ok().as_deref() != Some(source.as_str()) {
        fs::write(dir.join(&name), source)?;
    }
    Ok(format!("{INTERNAL_DIR}/{STAGED_DIR}/{name}"))
}

/// A new deck.html in the template's style: its styles (and fonts) without its slides,
/// titled `title`, naming the template.
pub fn deck_shell(source: &str, id: &str, title: &str) -> String {
    let shell = html::strip_slides(source);
    let shell = review::write(&shell, &Default::default());
    let shell = html::set_title(&shell, title);
    html::ensure_runtime(&html::set_template(&shell, Some(id)))
}

/// Saves the deck in `deck_dir` as the user template `name`: a copy of its deck.html with
/// placeholder text in place of its content, and its assets. Returns the new template.
pub fn create_from_deck(deck_dir: &Path, root: &Path, name: &str) -> Result<TemplateSummary> {
    let name = name.trim();
    let name = if name.is_empty() { "My template" } else { name };
    let stem = html::slugify(name);
    let stem = if stem.is_empty() { "template" } else { &stem };
    let id = std::iter::once(stem.to_string())
        .chain((2..).map(|n| format!("{stem}-{n}")))
        .find(|id| !root.join(id).exists() && builtin(id).is_none())
        .expect("unbounded");
    let source = fs::read_to_string(deck_dir.join(DECK_FILE))?;
    if html::find_slides(&source).is_empty() {
        return Err(Error::msg(
            "The deck has no slides to make a template from.",
        ));
    }
    let cleaned = html::with_placeholder_text(&source);
    let cleaned = review::write(&cleaned, &Default::default());
    let cleaned = html::set_template(&html::set_title(&cleaned, name), None);
    let dir = root.join(&id);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(DECK_FILE), &cleaned)?;
    let assets = deck_dir.join("assets");
    if assets.is_dir() {
        copy_dir(&assets, &dir.join("assets"))?;
    }
    Ok(summary(&id, &cleaned, Some(&dir)))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Serves a template file to the editor's previews: deck.html with the player runtime, or
/// one of a user template's assets.
pub fn read_file(root: &Path, id: &str, rel: &str) -> Option<(&'static str, Vec<u8>)> {
    if rel == DECK_FILE {
        let source = source(root, id).ok()?;
        return Some((
            deck::mime_for(rel),
            html::ensure_runtime(&source).into_bytes(),
        ));
    }
    let file = deck::resolve_in_deck(&user_dir(root, id)?, rel).ok()?;
    Some((deck::mime_for(rel), fs::read(file).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint;

    /// A throwaway folder (the user's templates, or a deck).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("slopslide-tpl-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn add(&self, id: &str, html: &str) -> PathBuf {
            let dir = self.0.join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(DECK_FILE), html).unwrap();
            dir
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MINE: &str = "<!DOCTYPE html><html><head><title>Mine</title><style>.slide{}</style></head><body><main class=\"deck\">\n<section class=\"slide\" id=\"cover\"><h1>Cover</h1></section>\n<section class=\"slide\" id=\"list\"><p>List</p></section>\n</main></body></html>";

    const LAYOUTS: &[&str] = &[
        "title", "section", "bullets", "split", "stats", "quote", "closing",
    ];

    #[test]
    fn every_builtin_template_has_the_shared_layouts_and_passes_lint() {
        assert_eq!(BUILTIN.len(), 22);
        for (id, source) in BUILTIN {
            assert!(is_valid_id(id) && html::slugify(id) == *id, "{id}");
            let ids: Vec<_> = html::find_slides(source)
                .into_iter()
                .filter_map(|s| s.id)
                .collect();
            assert_eq!(ids, LAYOUTS, "{id}");
            assert!(html::title(source).is_some(), "{id} has a title");
            assert!(
                !source.contains("slopslide:runtime"),
                "{id}: the runtime is added when served"
            );
            let served = html::ensure_runtime(source);
            assert_eq!(lint::lint(&served, |_| false, &[]), vec![], "{id}");
            let shell = deck_shell(source, id, "Talk");
            assert_eq!(
                lint::lint(&shell, |_| false, &[]),
                vec![],
                "{id} as a new deck"
            );
        }
    }

    #[test]
    fn lists_user_templates_first_and_lets_them_replace_builtins() {
        let root = TempDir::new();
        root.add("mine", MINE);
        root.add("swiss", &MINE.replace("Mine", "My Swiss"));
        root.add("empty", "<html><head><title>Empty</title></head></html>");
        fs::create_dir_all(root.0.join("no-deck")).unwrap();
        root.add(".hidden", MINE);

        let all = list(&root.0);
        let ids: Vec<_> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(&ids[..2], ["mine", "swiss"], "by title: Mine, My Swiss");
        assert_eq!(all.len(), 2 + BUILTIN.len() - 1);
        assert_eq!(ids.iter().filter(|id| **id == "swiss").count(), 1);
        assert!(!ids.contains(&"empty") && !ids.contains(&"no-deck") && !ids.contains(&".hidden"));

        let mine = &all[0];
        assert_eq!(mine.title, "Mine");
        assert!(!mine.builtin);
        assert_eq!(mine.slides, ["cover", "list"]);
        assert_eq!(
            mine.path.as_deref(),
            Some(root.0.join("mine").to_str().unwrap())
        );
        assert_eq!(all[1].title, "My Swiss");

        let synthwave = all.iter().find(|t| t.id == "synthwave").unwrap();
        assert!(synthwave.builtin && synthwave.path.is_none());
        assert_eq!(synthwave.title, "Synthwave");
        assert_eq!(list(&root.0.join("missing")).len(), BUILTIN.len());
    }

    #[test]
    fn reads_sources_and_serves_files() {
        let root = TempDir::new();
        let dir = root.add("mine", MINE);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/logo.svg"), "<svg/>").unwrap();

        assert_eq!(source(&root.0, "mine").unwrap(), MINE);
        assert!(source(&root.0, "swiss")
            .unwrap()
            .contains("<title>Swiss Design</title>"));
        assert!(source(&root.0, "nope").is_err());
        assert!(source(&root.0, "../mine").is_err());

        let (mime, body) = read_file(&root.0, "swiss", "deck.html").unwrap();
        assert_eq!(mime, "text/html; charset=utf-8");
        assert!(String::from_utf8(body).unwrap().contains(html::JS_START));
        assert_eq!(
            read_file(&root.0, "mine", "assets/logo.svg"),
            Some(("image/svg+xml", b"<svg/>".to_vec()))
        );
        assert_eq!(read_file(&root.0, "mine", "../mine/deck.html"), None);
        assert_eq!(read_file(&root.0, "swiss", "assets/logo.svg"), None);
        assert_eq!(read_file(&root.0, "nope", "deck.html"), None);
    }

    #[test]
    fn validates_ids() {
        for ok in ["swiss", "my template", "Été"] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in [
            "", ".x", "a/b", "a\\b", "a\"b", "a<b", "a&b", "a?b", "a#b", "a%b",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn stages_a_copy_without_the_runtime_for_the_agent() {
        let root = TempDir::new();
        root.add("mine", &html::ensure_runtime(MINE));
        let deck = TempDir::new();
        let rel = stage(&deck.0, &root.0, "mine").unwrap();
        assert_eq!(rel, ".slopslide/templates/mine.html");
        let staged = fs::read_to_string(deck.0.join(&rel)).unwrap();
        assert!(!staged.contains("slopslide:runtime"));
        assert_eq!(html::find_slides(&staged).len(), 2);
        assert_eq!(stage(&deck.0, &root.0, "mine").unwrap(), rel, "idempotent");
        assert!(stage(&deck.0, &root.0, "bento-grid").is_ok());
        assert!(stage(&deck.0, &root.0, "../x").is_err());
        assert!(stage(&deck.0, &root.0, "nope").is_err());
    }

    #[test]
    fn a_deck_shell_has_the_styles_but_no_slides() {
        let source = BUILTIN[0].1;
        let shell = deck_shell(source, BUILTIN[0].0, "Q3 & beyond");
        assert!(html::find_slides(&shell).is_empty());
        assert_eq!(html::title(&shell).as_deref(), Some("Q3 & beyond"));
        assert_eq!(html::template(&shell).as_deref(), Some(BUILTIN[0].0));
        let style = &source[source.find("<style>").unwrap()..source.find("</style>").unwrap()];
        assert!(shell.contains(style));
        assert!(shell.contains(html::CSS_START) && shell.contains(html::JS_START));
    }

    #[test]
    fn saves_a_deck_as_a_template_with_placeholder_content() {
        let root = TempDir::new();
        let deck = TempDir::new();
        let marks = [(
            "cover".to_string(),
            vec![review::Stroke {
                tool: review::InkTool::Pen,
                color: "#ef4444".into(),
                points: vec![[0.5, 0.5]],
            }],
        )]
        .into();
        let source = html::set_template(
            &review::write(&html::ensure_runtime(MINE), &marks),
            Some("swiss"),
        );
        fs::write(deck.0.join(DECK_FILE), &source).unwrap();
        fs::create_dir_all(deck.0.join("assets/icons")).unwrap();
        fs::write(deck.0.join("assets/icons/a.svg"), "<svg/>").unwrap();
        fs::create_dir_all(deck.0.join(INTERNAL_DIR)).unwrap();
        fs::write(deck.0.join(INTERNAL_DIR).join("chat.json"), "[]").unwrap();

        let created = create_from_deck(&deck.0, &root.0, "  Quarterly Review ").unwrap();
        assert_eq!(created.id, "quarterly-review");
        assert_eq!(created.title, "Quarterly Review");
        assert!(!created.builtin);
        assert_eq!(created.slides, ["cover", "list"]);
        let dir = root.0.join("quarterly-review");
        let html = fs::read_to_string(dir.join(DECK_FILE)).unwrap();
        let slides = html::find_slides(&html);
        let text = &html[slides[0].range.start..slides[1].range.end];
        assert!(!text.contains("Cover") && !text.contains("List"), "{text}");
        assert!(!html.contains(review::START), "review marks are dropped");
        assert_eq!(html::template(&html), None);
        assert!(html.contains(html::JS_START), "the runtime stays");
        assert_eq!(
            fs::read_to_string(dir.join("assets/icons/a.svg")).unwrap(),
            "<svg/>"
        );
        assert!(
            !dir.join(INTERNAL_DIR).exists(),
            "app internals are not copied"
        );
        assert_eq!(
            fs::read_to_string(deck.0.join(DECK_FILE)).unwrap(),
            source,
            "the deck is untouched"
        );

        let again = create_from_deck(&deck.0, &root.0, "Quarterly review").unwrap();
        assert_eq!(again.id, "quarterly-review-2");
        assert_eq!(
            create_from_deck(&deck.0, &root.0, "Swiss").unwrap().id,
            "swiss-2",
            "never shadows a built-in"
        );
        assert_eq!(
            create_from_deck(&deck.0, &root.0, " ").unwrap().id,
            "my-template"
        );
        assert_eq!(list(&root.0).iter().filter(|t| !t.builtin).count(), 4);

        fs::write(
            deck.0.join(DECK_FILE),
            "<html><body><main class=\"deck\"></main></body></html>",
        )
        .unwrap();
        assert!(create_from_deck(&deck.0, &root.0, "Empty").is_err());
    }
}
