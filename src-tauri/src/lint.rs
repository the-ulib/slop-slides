//! Lints deck.html: checks that the markup is well formed (every element closed, no stray
//! end tags) and that it follows the deck format the app and player rely on (see
//! `prompts/system.md`). Keep these rules in sync whenever the deck structure changes.

use std::collections::HashSet;

use serde::Serialize;

use crate::html::{
    self, find_ci, has_class, parse_tag, EDITOR_ATTRS, HIDDEN_ATTR, LOCKED_ATTR, MOVED_ATTR,
    SECTION_CLASS, SECTION_TITLE_ATTR,
};
use crate::review;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
    /// 1-based line in deck.html.
    pub line: usize,
    /// Id of the slide the issue is in, if any.
    pub slide: Option<String>,
}

/// Elements that never have content or an end tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
/// Elements whose end tag HTML lets you leave out.
const OPTIONAL_END: &[&str] = &[
    "html", "head", "body", "p", "li", "dt", "dd", "option", "optgroup", "tr", "td", "th", "thead",
    "tbody", "tfoot", "colgroup", "caption", "rb", "rt", "rtc", "rp",
];
/// Elements whose content is raw text, not markup.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];
/// Foreign content (SVG, MathML) allows self-closing tags such as `<path/>`.
const FOREIGN: &[&str] = &["svg", "math"];

struct Open {
    name: String,
    at: usize,
}

struct Linter<'a> {
    html: &'a str,
    line_starts: Vec<usize>,
    slides: Vec<html::SlideSpan>,
    issues: Vec<Issue>,
}

impl<'a> Linter<'a> {
    fn new(html: &'a str) -> Self {
        let line_starts = std::iter::once(0)
            .chain(html.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            html,
            line_starts,
            slides: html::find_slides(html),
            issues: Vec::new(),
        }
    }

    fn line(&self, at: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= at)
    }

    fn slide_at(&self, at: usize) -> Option<String> {
        self.slides
            .iter()
            .find(|s| s.range.contains(&at))
            .and_then(|s| s.id.clone())
    }

    fn report(&mut self, rule: &'static str, severity: Severity, at: usize, message: String) {
        self.issues.push(Issue {
            rule,
            severity,
            message,
            line: self.line(at),
            slide: self.slide_at(at),
        });
    }
}

/// Lints a deck. `asset_exists` answers whether a deck-relative `assets/…` path exists;
/// `locked` lists the locked slides as an agent turn started (empty outside a turn), and every
/// one the deck changed or removed is reported. Issues come back sorted by line.
pub fn lint(
    source: &str,
    asset_exists: impl Fn(&str) -> bool,
    locked: &[html::LockedSlide],
) -> Vec<Issue> {
    let mut l = Linter::new(source);
    check_markup(&mut l);
    check_document(&mut l);
    check_slides(&mut l);
    check_sections(&mut l);
    check_review(&mut l);
    check_template(&mut l);
    check_assets(&mut l, asset_exists);
    check_locked(&mut l, locked);
    l.issues.sort_by_key(|i| (i.line, i.severity));
    l.issues
}

/// Walks every tag: balance, stray end tags, duplicate attributes, and what sits where.
fn check_markup(l: &mut Linter) {
    let html = l.html;
    let bytes = html.as_bytes();
    let mut stack: Vec<Open> = Vec::new();
    let mut deck_depth: Option<usize> = None; // stack depth of <main class="deck">
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            if let Some(depth) = deck_depth {
                if stack.len() == depth && !bytes[i].is_ascii_whitespace() {
                    let end = html[i..].find('<').map_or(html.len(), |e| i + e);
                    l.report(
                        "deck-stray-content",
                        Severity::Warning,
                        i,
                        format!(
                            "Text \"{}\" sits directly in <main class=\"deck\">; put it inside a slide.",
                            truncate(html[i..end].trim(), 40)
                        ),
                    );
                    i = end;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            match html[i..].find("-->") {
                Some(e) => i += e + 3,
                None => {
                    l.report(
                        "unclosed-comment",
                        Severity::Error,
                        i,
                        "This comment is never closed with -->; everything after it is hidden."
                            .into(),
                    );
                    i = bytes.len();
                }
            }
            continue;
        }
        let next = bytes.get(i + 1).copied().unwrap_or(b' ');
        let looks_like_tag = next.is_ascii_alphabetic() || next == b'/';
        let Some(tag) = parse_tag(html, i) else {
            if looks_like_tag {
                l.report(
                    "unterminated-tag",
                    Severity::Error,
                    i,
                    "This tag is never closed with > (or has an unterminated quote).".into(),
                );
            }
            i += 1;
            continue;
        };
        let self_closing = html[..tag.end].ends_with("/>");

        if !tag.closing {
            let mut seen = HashSet::new();
            for (name, ..) in &tag.attrs {
                if !seen.insert(name.as_str()) {
                    l.report(
                        "duplicate-attribute",
                        Severity::Error,
                        i,
                        format!("<{}> has the attribute `{name}` more than once.", tag.name),
                    );
                }
            }
            if deck_depth == Some(stack.len()) {
                let is_slide = tag.name == "section" && has_class(&tag, "slide");
                let is_marker = tag.name == "div" && has_class(&tag, SECTION_CLASS);
                if !is_slide && !is_marker && !matches!(tag.name.as_str(), "script" | "template") {
                    l.report(
                        "deck-stray-content",
                        Severity::Warning,
                        i,
                        format!(
                            "<{}> sits directly in <main class=\"deck\">; only <section class=\"slide\"> belongs there.",
                            tag.name
                        ),
                    );
                }
            }
            if tag.name == "div"
                && has_class(&tag, SECTION_CLASS)
                && deck_depth != Some(stack.len())
            {
                l.report(
                    "section-marker-misplaced",
                    Severity::Warning,
                    i,
                    "Section markers must be direct children of <main class=\"deck\">, between slides."
                        .into(),
                );
            }
            if tag.name == "section" && has_class(&tag, "slide") && deck_depth != Some(stack.len())
            {
                // Nested slides are content of their parent slide, not slides of their own.
                let nested = l
                    .slides
                    .iter()
                    .any(|s| s.range.start < i && i < s.range.end);
                if !nested {
                    l.report(
                        "slide-outside-deck",
                        Severity::Error,
                        i,
                        "Slides must be direct children of <main class=\"deck\">.".into(),
                    );
                }
            }
            if l.slides.iter().any(|s| s.range.contains(&i)) {
                if tag.name == "style" {
                    l.report(
                        "style-in-slide",
                        Severity::Warning,
                        i,
                        "Move this <style> into the single <style> in <head>, scoped by the slide id.".into(),
                    );
                }
                if tag.name == "script" {
                    l.report(
                        "script-in-slide",
                        Severity::Warning,
                        i,
                        "Slides should not contain <script>; use CSS for slide visuals.".into(),
                    );
                }
                if let Some((name, ..)) = tag
                    .attrs
                    .iter()
                    .find(|(n, ..)| EDITOR_ATTRS.contains(&n.as_str()))
                {
                    l.report(
                        "editor-leftover",
                        Severity::Warning,
                        i,
                        format!(
                            "<{}> has `{name}`, left over from editing in the app; remove it.",
                            tag.name
                        ),
                    );
                }
                if tag.attrs.iter().any(|(n, ..)| n == MOVED_ATTR) {
                    l.report(
                        "moved-element",
                        Severity::Warning,
                        i,
                        format!(
                            "<{}> was moved, rotated, tilted or scaled by hand ({MOVED_ATTR} with an inline `translate`, `rotate`, `scale` or tilt `transform`). Rework the slide's layout so it sits where it appears now without the offset, turn a scale into real sizes, keep an intended rotation or tilt in the slide's styles, then remove the attribute and the inline transforms.",
                            tag.name
                        ),
                    );
                }
            }
        }

        let in_foreign = stack.iter().any(|o| FOREIGN.contains(&o.name.as_str()));
        if tag.closing {
            close(l, &mut stack, &tag.name, i);
            if deck_depth.is_some_and(|d| stack.len() < d) {
                deck_depth = None;
            }
        } else if VOID.contains(&tag.name.as_str()) {
            // No content, no end tag.
        } else if self_closing {
            if !in_foreign && !FOREIGN.contains(&tag.name.as_str()) {
                l.report(
                    "self-closing-tag",
                    Severity::Error,
                    i,
                    format!(
                        "<{0}/> does not close the element in HTML; write <{0}></{0}>.",
                        tag.name
                    ),
                );
                // Browsers ignore the slash, so the element stays open.
                stack.push(Open {
                    name: tag.name.clone(),
                    at: i,
                });
            }
        } else {
            // Opening a <p> or <li> implicitly ends a previous one.
            if OPTIONAL_END.contains(&tag.name.as_str())
                && stack.last().is_some_and(|o| o.name == tag.name)
            {
                stack.pop();
            }
            if tag.name == "main" && has_class(&tag, "deck") && deck_depth.is_none() {
                deck_depth = Some(stack.len() + 1);
            }
            stack.push(Open {
                name: tag.name.clone(),
                at: i,
            });
            if RAW_TEXT.contains(&tag.name.as_str()) {
                i = find_ci(html, tag.end, &format!("</{}", tag.name)).unwrap_or(bytes.len());
                continue;
            }
        }
        i = tag.end;
    }
    for open in stack.into_iter().rev() {
        if !OPTIONAL_END.contains(&open.name.as_str()) {
            l.report(
                "unclosed-tag",
                Severity::Error,
                open.at,
                format!("<{}> is never closed.", open.name),
            );
        }
    }
}

fn close(l: &mut Linter, stack: &mut Vec<Open>, name: &str, at: usize) {
    let Some(pos) = stack.iter().rposition(|o| o.name == name) else {
        if !VOID.contains(&name) {
            l.report(
                "stray-end-tag",
                Severity::Error,
                at,
                format!("</{name}> has no matching <{name}>."),
            );
        }
        return;
    };
    let closed: Vec<Open> = stack.drain(pos..).collect();
    for open in closed.into_iter().skip(1).rev() {
        if !OPTIONAL_END.contains(&open.name.as_str()) {
            let message = format!(
                "<{}> is never closed (</{name}> on line {} ends it implicitly).",
                open.name,
                l.line(at)
            );
            l.report("unclosed-tag", Severity::Error, open.at, message);
        }
    }
}

/// Document-level structure: doctype, title, deck container, runtime blocks.
fn check_document(l: &mut Linter) {
    let html = l.html;
    if !html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<!doctype html")
    {
        l.report(
            "doctype",
            Severity::Warning,
            0,
            "deck.html should start with <!DOCTYPE html>.".into(),
        );
    }
    if html::title(html).is_none() {
        l.report(
            "title",
            Severity::Warning,
            0,
            "The deck needs a non-empty <title> in <head>.".into(),
        );
    }
    if !has_deck_container(html) {
        l.report(
            "deck-container",
            Severity::Error,
            0,
            "There is no <main class=\"deck\"> element holding the slides.".into(),
        );
    }
    if !html.contains(html::CSS_START) || !html.contains(html::JS_START) {
        l.report(
            "runtime-missing",
            Severity::Warning,
            0,
            "The slopslide:runtime-css / runtime-js blocks are missing; keep them intact.".into(),
        );
    }
}

fn has_deck_container(html: &str) -> bool {
    let mut from = 0;
    while let Some(at) = find_ci(html, from, "<main") {
        if parse_tag(html, at).is_some_and(|t| t.name == "main" && has_class(&t, "deck")) {
            return true;
        }
        from = at + 1;
    }
    false
}

/// Slide ids: present, unique, kebab-case.
fn check_slides(l: &mut Linter) {
    let mut seen = HashSet::new();
    let slides = l.slides.clone();
    for (index, slide) in slides.iter().enumerate() {
        let at = slide.range.start;
        match &slide.id {
            None => l.report(
                "slide-id-missing",
                Severity::Error,
                at,
                format!("Slide {} has no id.", index + 1),
            ),
            Some(id) if !seen.insert(id.clone()) => l.report(
                "slide-id-duplicate",
                Severity::Error,
                at,
                format!("The slide id `{id}` is used more than once."),
            ),
            Some(id) if !is_kebab_case(id) => l.report(
                "slide-id-format",
                Severity::Warning,
                at,
                format!("The slide id `{id}` should be kebab-case (e.g. `pricing-tiers`)."),
            ),
            Some(_) => {}
        }
        let valued = |range: &Option<std::ops::Range<usize>>| {
            range.as_ref().is_some_and(|r| {
                let attr = &l.html[r.clone()];
                attr.contains('=') && !attr.ends_with("\"\"") && !attr.ends_with("''")
            })
        };
        if valued(&slide.hidden) {
            l.report(
                "hidden-value",
                Severity::Warning,
                at,
                format!("Write `{HIDDEN_ATTR}` without a value; any value still hides the slide."),
            );
        }
        if valued(&slide.locked) {
            l.report(
                "locked-value",
                Severity::Warning,
                at,
                format!("Write `{LOCKED_ATTR}` without a value; any value still locks the slide."),
            );
        }
    }
}

/// Locked slides the agent changed or removed this turn; the app puts them back after it.
fn check_locked(l: &mut Linter, locked: &[html::LockedSlide]) {
    for id in html::changed_locked(l.html, locked) {
        let span = l.slides.iter().find(|s| s.id.as_deref() == Some(&id));
        let (at, message) = match span {
            Some(span) => (
                span.range.start,
                format!(
                    "The slide `{id}` is locked ({LOCKED_ATTR}) but was changed. Undo every \
                     change to it: the user locked it, and the app restores it after your turn."
                ),
            ),
            None => (
                0,
                format!(
                    "The locked slide `{id}` was removed or renamed. Put it back unchanged: \
                     the user locked it, and the app restores it after your turn."
                ),
            ),
        };
        l.report("locked-slide-changed", Severity::Error, at, message);
    }
}

/// Section markers between slides: each needs a title and no content of its own.
fn check_sections(l: &mut Linter) {
    for (index, section) in html::find_sections(l.html).iter().enumerate() {
        let at = section.range.start;
        if section.title.is_empty() {
            l.report(
                "section-title-missing",
                Severity::Warning,
                at,
                format!(
                    "Section {} needs a non-empty {SECTION_TITLE_ATTR} attribute.",
                    index + 1
                ),
            );
        }
        if !section.empty {
            l.report(
                "section-marker-content",
                Severity::Warning,
                at,
                "A section marker must be empty: <div class=\"deck-section\" data-title=\"…\"></div>."
                    .into(),
            );
        }
    }
}

/// The app-managed review block (the user's marks on slides): readable, and outside the slides.
fn check_review(l: &mut Linter) {
    let html = l.html;
    let Some(at) = html.find(review::START) else {
        return;
    };
    if let Err(problem) = review::parse(html) {
        l.report(
            "review-invalid",
            Severity::Warning,
            at,
            format!(
                "{problem} The slopslide:review block holds the user's review marks and is managed by the app: restore it as it was, or delete the whole block (from its start comment to its end comment) if the user asked to clear the review."
            ),
        );
    }
    if l.slides.iter().any(|s| s.range.contains(&at)) {
        l.report(
            "review-misplaced",
            Severity::Warning,
            at,
            "The slopslide:review block belongs at the end of <body>, outside the slides; move it back there unchanged.".into(),
        );
    }
}

/// The template `<meta>`: at most one, in `<head>`, naming a template.
fn check_template(l: &mut Linter) {
    let html = l.html;
    let body = find_ci(html, 0, "<body").unwrap_or(html.len());
    let mut seen = 0;
    let mut from = 0;
    while let Some(at) = find_ci(html, from, "<meta") {
        from = at + 1;
        let Some(tag) = parse_tag(html, at) else {
            continue;
        };
        let names_template = tag
            .attrs
            .iter()
            .any(|(n, v, _, _)| n == "name" && v.trim().eq_ignore_ascii_case(html::TEMPLATE_META));
        if tag.name != "meta" || !names_template || !in_markup(l, at) {
            continue;
        }
        seen += 1;
        let content = tag.attrs.iter().find(|(n, _, _, _)| n == "content");
        if !content.is_some_and(|(_, v, _, _)| !v.trim().is_empty()) {
            l.report(
                "template-meta-empty",
                Severity::Warning,
                at,
                format!(
                    "<meta name=\"{}\"> needs the template's id in `content`; remove the tag if the deck follows no template.",
                    html::TEMPLATE_META
                ),
            );
        }
        if seen > 1 {
            l.report(
                "template-meta-duplicate",
                Severity::Warning,
                at,
                format!("Keep only one <meta name=\"{}\">.", html::TEMPLATE_META),
            );
        }
        if at > body {
            l.report(
                "template-meta-misplaced",
                Severity::Warning,
                at,
                format!("<meta name=\"{}\"> belongs in <head>.", html::TEMPLATE_META),
            );
        }
    }
}

fn is_kebab_case(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Images and media: attached files exist, nothing is hotlinked, images have alt text.
fn check_assets(l: &mut Linter, asset_exists: impl Fn(&str) -> bool) {
    let html = l.html;
    for (range, path) in html::asset_refs(html) {
        if !asset_exists(path) {
            l.report(
                "missing-asset",
                Severity::Error,
                range.start,
                format!("`{path}` does not exist in the deck's assets folder."),
            );
        }
    }
    let mut from = 0;
    while let Some(at) = find_ci(html, from, "<img") {
        from = at + 4;
        let Some(tag) = parse_tag(html, at).filter(|t| t.name == "img") else {
            continue;
        };
        if !in_markup(l, at) {
            continue;
        }
        let attr = |name: &str| {
            tag.attrs
                .iter()
                .find(|(n, ..)| n == name)
                .map(|a| a.1.as_str())
        };
        if attr("src").is_some_and(|s| s.starts_with("http://") || s.starts_with("https://")) {
            l.report(
                "remote-image",
                Severity::Warning,
                at,
                "Images should be attached files (assets/…), not hotlinked URLs.".into(),
            );
        }
        if attr("alt").is_none() {
            l.report(
                "img-alt",
                Severity::Warning,
                at,
                "<img> needs an alt attribute (use alt=\"\" for decoration).".into(),
            );
        }
    }
}

/// Whether `at` is in markup rather than inside a comment, script, or style.
fn in_markup(l: &Linter, at: usize) -> bool {
    let before = &l.html[..at];
    let lower = before.to_ascii_lowercase();
    let open_comment = before
        .rfind("<!--")
        .is_some_and(|c| before[c..].find("-->").is_none());
    let raw = ["script", "style"].iter().any(|t| {
        lower
            .rfind(&format!("<{t}"))
            .is_some_and(|o| lower[o..].find(&format!("</{t}")).is_none())
    });
    !open_comment && !raw
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// Plain-text report for the agent's lint tool.
pub fn format_report(issues: &[Issue]) -> String {
    if issues.is_empty() {
        return "deck.html passes lint: no issues.".into();
    }
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues.len() - errors;
    let mut out = format!("deck.html has {errors} error(s) and {warnings} warning(s):\n");
    for issue in issues {
        let severity = match issue.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let slide = issue
            .slide
            .as_ref()
            .map(|s| format!(" (slide `{s}`)"))
            .unwrap_or_default();
        out.push_str(&format!(
            "- line {} {severity} [{}]{slide}: {}\n",
            issue.line, issue.rule, issue.message
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNTIME: &str = "<!-- slopslide:runtime-css (managed by SlopSlide, do not edit) -->\
        <!-- slopslide:runtime-js (managed by SlopSlide, do not edit) -->";

    fn deck(body: &str) -> String {
        format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<title>Talk</title>\n{RUNTIME}\n<style>.slide {{}}</style>\n</head>\n<body>\n<main class=\"deck\">\n{body}\n</main>\n</body>\n</html>\n"
        )
    }

    fn rules(html: &str) -> Vec<&'static str> {
        lint(html, |p| p == "assets/logo.png", &[])
            .into_iter()
            .map(|i| i.rule)
            .collect()
    }

    #[test]
    fn a_clean_deck_passes() {
        let html = deck(
            r#"<section class="slide" id="intro">
  <h1 class="reveal">Hi</h1>
  <p>Unclosed paragraphs are fine<p>and so is this
  <ul><li>one<li>two</ul>
  <img src="assets/logo.png" alt="Logo"><br>
  <svg viewBox="0 0 10 10"><path d="M0 0"/><circle r="1" /></svg>
  <aside class="notes">Say hi</aside>
</section>
<!-- <div> commented out -->
<section class="slide" id="plan-2025" data-hidden><p>Plan</p></section>"#,
        );
        assert_eq!(lint(&html, |p| p == "assets/logo.png", &[]), vec![]);
    }

    fn stroke() -> review::Stroke {
        review::Stroke {
            tool: review::InkTool::Pen,
            color: "#ef4444".into(),
            points: vec![[0.5, 0.5]],
        }
    }

    #[test]
    fn accepts_review_marks() {
        let marks = [("intro".to_string(), vec![stroke()])].into();
        let html = review::write(
            &deck(r#"<section class="slide" id="intro"><p>Hi</p></section>"#),
            &marks,
        );
        assert!(html.contains(review::START));
        assert_eq!(rules(&html), Vec::<&str>::new());
    }

    #[test]
    fn flags_damaged_or_misplaced_review_marks() {
        let marks = [("intro".to_string(), vec![stroke()])].into();
        let html = review::write(
            &deck(r#"<section class="slide" id="intro"><p>Hi</p></section>"#),
            &marks,
        );
        assert_eq!(rules(&html.replace("#ef4444", "red")), ["review-invalid"]);
        assert_eq!(rules(&html.replace(review::END, "")), ["review-invalid"]);
        let block = &html[review::block_range(&html).unwrap()];
        let inside = deck(&format!(
            r#"<section class="slide" id="intro"><p>Hi</p>{block}</section>"#
        ));
        let found = rules(&inside);
        assert!(found.contains(&"review-misplaced"), "{found:?}");
        let issue = lint(&html.replace("[[", "["), |_| true, &[]).remove(0);
        assert_eq!(
            issue.line,
            html[..html.find(review::START).unwrap()].lines().count()
        );
    }

    #[test]
    fn the_app_templates_pass() {
        let template = include_str!("../assets/deck-template.html").replace("{{TITLE}}", "Talk");
        let blank = include_str!("../assets/blank-slide.html").replace("{{ID}}", "untitled");
        let with_slide = template.replace(
            "<main class=\"deck\">",
            &format!("<main class=\"deck\">\n    {blank}"),
        );
        let html = html::ensure_runtime(&with_slide);
        assert_eq!(rules(&html), Vec::<&str>::new());
    }

    #[test]
    fn accepts_the_player_flag_script_in_the_runtime_css_block() {
        let html = html::ensure_runtime(&deck(
            r#"<section class="slide" id="intro"><p>Hi</p></section>"#,
        ));
        let head = &html[..html.find("</head>").unwrap()];
        assert!(
            head.contains(html::PLAYER_FLAG),
            "flag script sits in <head>"
        );
        assert_eq!(rules(&html), Vec::<&str>::new());
    }

    #[test]
    fn reports_unclosed_and_stray_tags_with_lines() {
        let html =
            deck("<section class=\"slide\" id=\"a\">\n<div><span>x</div>\n</section>\n</em>");
        let issues = lint(&html, |_| true, &[]);
        let unclosed = issues.iter().find(|i| i.rule == "unclosed-tag").unwrap();
        assert!(unclosed.message.contains("<span>"));
        assert_eq!(unclosed.line, 11);
        assert_eq!(unclosed.slide.as_deref(), Some("a"));
        let stray = issues.iter().find(|i| i.rule == "stray-end-tag").unwrap();
        assert!(stray.message.contains("</em>"));
        assert_eq!(stray.line, 13);
    }

    #[test]
    fn reports_elements_left_open_at_the_end() {
        let html = deck("<section class=\"slide\" id=\"a\"><div>never closed</section>");
        assert!(rules(&html).contains(&"unclosed-tag"));
        let html = "<!DOCTYPE html><html><head><title>T</title></head><body><main class=\"deck\"><section class=\"slide\" id=\"a\">";
        let found = rules(html);
        assert_eq!(
            found.iter().filter(|r| **r == "unclosed-tag").count(),
            2,
            "{found:?}"
        );
    }

    #[test]
    fn reports_unterminated_tags_and_comments() {
        assert!(rules(&deck(
            "<section class=\"slide\" id=\"a\"><img src=\"x></section>"
        ))
        .contains(&"unterminated-tag"));
        assert!(rules(&deck(
            "<!-- open forever <section class=\"slide\" id=\"a\"></section>"
        ))
        .contains(&"unclosed-comment"));
        // A lone `<` in text is not a tag.
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\"><p>1 < 2</p></section>"
            )),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn rejects_self_closing_html_elements_outside_svg() {
        let found = rules(&deck("<section class=\"slide\" id=\"a\"><div/></section>"));
        assert!(found.contains(&"self-closing-tag"));
        assert_eq!(rules(&deck("<section class=\"slide\" id=\"a\"><br/><img src=\"assets/logo.png\" alt=\"\"/></section>")), Vec::<&str>::new());
    }

    #[test]
    fn ignores_markup_inside_scripts_styles_and_comments() {
        let html = deck("<section class=\"slide\" id=\"a\"></section>\n<script>const s = '<div><img src=x>';</script>")
            .replace(".slide {}", ".slide {} /* <div> <img src=x> */");
        assert_eq!(rules(&html), Vec::<&str>::new());
    }

    #[test]
    fn reports_duplicate_attributes() {
        assert!(rules(&deck(
            "<section class=\"slide\" id=\"a\"><p class=\"x\" class=\"y\">x</p></section>"
        ))
        .contains(&"duplicate-attribute"));
    }

    #[test]
    fn checks_document_structure() {
        let found = rules(
            "<html><head></head><body><section class=\"slide\" id=\"a\"></section></body></html>",
        );
        for rule in [
            "doctype",
            "title",
            "deck-container",
            "runtime-missing",
            "slide-outside-deck",
        ] {
            assert!(found.contains(&rule), "{rule} missing from {found:?}");
        }
    }

    #[test]
    fn checks_slide_ids() {
        let found = rules(&deck(
            r#"<section class="slide"></section>
<section class="slide" id="a"></section>
<section class="slide" id="a"></section>
<section class="slide" id="Big_Title"></section>"#,
        ));
        assert_eq!(
            found,
            ["slide-id-missing", "slide-id-duplicate", "slide-id-format"]
        );
        assert!(is_kebab_case("q3-plan-2"));
        assert!(!is_kebab_case("-a") && !is_kebab_case("a--b") && !is_kebab_case("A"));
    }

    #[test]
    fn flags_valued_hidden_attributes() {
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\" data-hidden=\"\"></section>"
            )),
            Vec::<&str>::new()
        );
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\" data-hidden=\"false\"></section>"
            )),
            ["hidden-value"]
        );
    }

    #[test]
    fn flags_valued_locked_attributes() {
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\" data-locked></section>"
            )),
            Vec::<&str>::new()
        );
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\" data-locked=\"false\"></section>"
            )),
            ["locked-value"]
        );
    }

    #[test]
    fn flags_locked_slides_changed_during_a_turn() {
        let before = deck(
            "<section class=\"slide\" id=\"a\" data-locked>A</section>\n<section class=\"slide\" id=\"b\" data-locked>B</section>\n<section class=\"slide\" id=\"c\">C</section>",
        );
        let locked = html::locked_slides(&before);
        assert_eq!(lint(&before, |_| true, &locked), vec![]);
        let edited = before.replace(">C<", ">C!<");
        assert_eq!(
            lint(&edited, |_| true, &locked),
            vec![],
            "other slides are free"
        );

        let changed = html::delete(&before.replace(">A<", ">A!<"), "b").unwrap();
        let issues = lint(&changed, |_| true, &locked);
        let found: Vec<_> = issues
            .iter()
            .map(|i| (i.rule, i.severity, i.slide.as_deref(), i.line))
            .collect();
        assert_eq!(
            found,
            [
                ("locked-slide-changed", Severity::Error, None, 1),
                ("locked-slide-changed", Severity::Error, Some("a"), 10),
            ]
        );
        assert!(issues[0].message.contains("`b` was removed"));
        assert!(issues[1].message.contains("`a` is locked"));
    }

    #[test]
    fn flags_content_outside_slides_in_the_deck() {
        let found = rules(&deck("<div>loose</div>\nloose text\n<section class=\"slide\" id=\"a\"><div>fine</div></section>"));
        assert_eq!(found, ["deck-stray-content", "deck-stray-content"]);
        assert_eq!(rules(&deck("<section class=\"slide\" id=\"a\"><section class=\"slide\">nested</section></section>")), Vec::<&str>::new());
    }

    #[test]
    fn flags_styles_and_scripts_inside_slides() {
        let found = rules(&deck(
            "<section class=\"slide\" id=\"a\"><style>p{}</style><script>1</script></section>",
        ));
        assert_eq!(found, ["style-in-slide", "script-in-slide"]);
    }

    #[test]
    fn flags_hand_edits_awaiting_cleanup() {
        let found = lint(
            &deck(
                "<section class=\"slide\" id=\"a\">\n<h2 data-moved style=\"translate: 40px -12px\">Moved</h2>\n<img data-moved style=\"translate: 0px 12px; rotate: 15deg; scale: 1.5 0.8\" src=\"a.png\" alt=\"\">\n<div data-moved style=\"transform: perspective(1000px) rotateX(10deg) rotateY(30deg) !important\">Tilted</div>\n<p contenteditable=\"true\" data-slop-selected>Left over</p>\n</section>",
            ),
            |_| true,
            &[],
        );
        let found: Vec<_> = found.iter().map(|i| (i.rule, i.slide.as_deref())).collect();
        assert_eq!(
            found,
            [
                ("moved-element", Some("a")),
                ("moved-element", Some("a")),
                ("moved-element", Some("a")),
                ("editor-leftover", Some("a"))
            ]
        );
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\" data-slop-typing><p>Hi</p></section>"
            )),
            ["editor-leftover"]
        );
        // Outside slides (or as text) the attributes mean nothing to the editor.
        assert_eq!(
            rules(&deck(
                "<section class=\"slide\" id=\"a\"><p>data-moved</p></section>"
            )),
            Vec::<&str>::new()
        );
        assert_eq!(
            rules(&format!(
                "{}<div data-moved contenteditable></div>",
                deck("<section class=\"slide\" id=\"a\"></section>")
            )),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn checks_images_and_assets() {
        let found = rules(&deck(
            r#"<section class="slide" id="a">
<img src="assets/missing.png" alt="">
<img src="https://example.com/x.png" alt="x">
<img src="assets/logo.png">
<div style="background: url(./assets/logo.png)"></div>
</section>"#,
        ));
        assert_eq!(found, ["missing-asset", "remote-image", "img-alt"]);
    }

    #[test]
    fn sorts_by_line_and_formats_a_report() {
        let html = deck("<section class=\"slide\" id=\"Bad\"><span></section>");
        let issues = lint(&html, |_| true, &[]);
        assert!(issues.windows(2).all(|w| w[0].line <= w[1].line));
        let report = format_report(&issues);
        assert!(
            report.starts_with("deck.html has 1 error(s) and 1 warning(s):"),
            "{report}"
        );
        assert!(
            report.contains("line 10 error [unclosed-tag] (slide `Bad`): <span> is never closed")
        );
        assert_eq!(format_report(&[]), "deck.html passes lint: no issues.");
    }

    #[test]
    fn serializes_for_the_frontend() {
        let issue = Issue {
            rule: "title",
            severity: Severity::Warning,
            message: "m".into(),
            line: 1,
            slide: None,
        };
        assert_eq!(
            serde_json::to_value(&issue).unwrap(),
            serde_json::json!({"rule":"title","severity":"warning","message":"m","line":1,"slide":null})
        );
    }

    #[test]
    fn accepts_section_markers_between_slides() {
        let html = deck(
            r#"<section class="slide" id="a">A</section>
<div class="deck-section" data-title="Part two"></div>
<section class="slide" id="b">B</section>"#,
        );
        assert_eq!(rules(&html), Vec::<&str>::new());
    }

    #[test]
    fn flags_section_markers_without_title_or_with_content() {
        let found = rules(&deck(
            r#"<div class="deck-section"></div>
<div class="deck-section" data-title="  "></div>
<section class="slide" id="a">A</section>
<div class="deck-section" data-title="Ok"><p>x</p></div>"#,
        ));
        assert_eq!(
            found,
            [
                "section-title-missing",
                "section-title-missing",
                "section-marker-content"
            ]
        );
        let issues = lint(
            &deck(
                "<div class=\"deck-section\"></div>\n<section class=\"slide\" id=\"a\">A</section>",
            ),
            |_| true,
            &[],
        );
        assert_eq!(issues[0].line, 10);
        assert!(issues[0].message.contains("Section 1"));
    }

    #[test]
    fn flags_section_markers_outside_the_deck_level() {
        let inside = rules(&deck(
            r#"<section class="slide" id="a"><div class="deck-section" data-title="X"></div></section>"#,
        ));
        assert_eq!(inside, ["section-marker-misplaced"]);
        let wrapped = rules(&deck(
            r#"<div class="wrap"><div class="deck-section" data-title="X"></div></div>
<section class="slide" id="a">A</section>"#,
        ));
        assert!(wrapped.contains(&"section-marker-misplaced"));
        assert!(wrapped.contains(&"deck-stray-content"));
    }

    #[test]
    fn checks_the_template_meta() {
        let named = html::set_template(
            &deck(r#"<section class="slide" id="a"></section>"#),
            Some("swiss"),
        );
        assert_eq!(rules(&named), Vec::<&str>::new());
        let empty = named.replace("content=\"swiss\"", "content=\" \"");
        assert_eq!(rules(&empty), ["template-meta-empty"]);
        let no_content = named.replace(" content=\"swiss\"", "");
        assert_eq!(rules(&no_content), ["template-meta-empty"]);
        let twice = named.replace(
            "<title>Talk</title>",
            "<title>Talk</title><meta name=\"slopslide-template\" content=\"bohemian\">",
        );
        assert_eq!(rules(&twice), ["template-meta-duplicate"]);
        let in_body = deck(
            r#"<section class="slide" id="a"><meta name="slopslide-template" content="x"></section>"#,
        );
        assert_eq!(rules(&in_body), ["template-meta-misplaced"]);
        let commented = deck(r#"<!-- <meta name="slopslide-template" content=""> -->"#);
        assert_eq!(rules(&commented), Vec::<&str>::new());
        let other = deck("").replace("<title>", "<meta name=\"description\" content=\"\"><title>");
        assert_eq!(rules(&other), Vec::<&str>::new());
    }
}
