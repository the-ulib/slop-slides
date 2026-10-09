//! Single-file deck format. A deck is one HTML document whose top-level
//! `<section class="slide" id="…">` elements are the slides, in order. This module finds
//! those sections without a full HTML parser (it skips comments, `<script>`, `<style>`
//! and quoted attributes) and rewrites the document for slide-level operations.
//!
//! Between slides, `<div class="deck-section" data-title="…"></div>` markers start a named
//! section. They are shown in the editor's slide rail but are not slides: the player hides
//! them and nothing counts them.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::Range;

const RUNTIME_CSS: &str = include_str!("../assets/runtime.css");
const RUNTIME_JS: &str = include_str!("../assets/runtime.js");
pub(crate) const CSS_START: &str =
    "<!-- slopslide:runtime-css (managed by SlopSlide, do not edit) -->";
const CSS_END: &str = "<!-- /slopslide:runtime-css -->";
pub(crate) const JS_START: &str =
    "<!-- slopslide:runtime-js (managed by SlopSlide, do not edit) -->";
const JS_END: &str = "<!-- /slopslide:runtime-js -->";
/// Runs in `<head>` so the player's stylesheet applies before the first paint. Viewers without
/// JavaScript skip it and get the stylesheet's fallback: every slide, top to bottom.
pub(crate) const PLAYER_FLAG: &str =
    "<script>document.documentElement.setAttribute(\"data-slop-player\", \"\");</script>";

#[derive(Debug, Clone)]
pub struct SlideSpan {
    /// The whole `<section …>…</section>`.
    pub range: Range<usize>,
    pub id: Option<String>,
    /// Byte range of the id attribute's value, when present.
    pub id_value: Option<Range<usize>>,
    /// Byte offset just after `<section`.
    pub tag_name_end: usize,
    /// Byte range of the whole `data-hidden` attribute, when the slide is hidden.
    pub hidden: Option<Range<usize>>,
    /// Byte range of the whole `data-locked` attribute, when the slide is locked.
    pub locked: Option<Range<usize>>,
}

/// Marks a slide the player skips. The editor still shows it, muted.
pub const HIDDEN_ATTR: &str = "data-hidden";

/// Marks a slide neither the user nor the agent may change. The editor refuses edits to it,
/// and the app puts it back as it was if the agent changes or removes it in a turn.
pub const LOCKED_ATTR: &str = "data-locked";

/// Marks an element the user moved, rotated or scaled by hand in the editor; it carries inline
/// `translate` / `rotate` / `scale` styles until the agent tidies the slide's layout.
pub const MOVED_ATTR: &str = "data-moved";

/// Attributes the in-editor slide editor puts on elements while it works. They never belong
/// in deck.html; the editor strips them before saving.
pub const EDITOR_ATTRS: &[&str] = &[
    "contenteditable",
    "data-slop-selected",
    "data-slop-hover",
    "data-slop-editing",
    "data-slop-typing",
];

/// Class of the marker element that starts a section; its title is the `data-title` attribute.
pub const SECTION_CLASS: &str = "deck-section";
pub const SECTION_TITLE_ATTR: &str = "data-title";

/// Key of the section marker with this document-order index, as used by [`reorder`].
pub fn section_key(index: usize) -> String {
    format!("section:{index}")
}

#[derive(Debug, Clone)]
pub struct SectionSpan {
    /// The marker element, including an immediately following `</div>`.
    pub range: Range<usize>,
    /// Decoded `data-title`; empty when missing.
    pub title: String,
    /// Byte range of the title attribute's value, when present.
    pub title_value: Option<Range<usize>>,
    /// Byte offset just after `<div`.
    pub tag_name_end: usize,
    /// Number of slides that come before the marker.
    pub before: usize,
    /// Whether the marker has no content besides its own end tag.
    pub empty: bool,
}

pub(crate) struct Tag {
    pub name: String,
    pub closing: bool,
    /// (name, value, value range, whole attribute range)
    pub attrs: Vec<(String, String, Range<usize>, Range<usize>)>,
    pub name_end: usize,
    pub end: usize,
}

/// Every tag outside comments and raw-text elements (`<script>`, `<style>`, …), with the
/// byte offset it starts at.
fn tags(html: &str) -> impl Iterator<Item = (usize, Tag)> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || loop {
        let Some(found) = html.get(i..).and_then(|rest| rest.find('<')) else {
            i = html.len();
            return None;
        };
        i += found;
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map_or(html.len(), |e| i + e + 3);
            continue;
        }
        let Some(tag) = parse_tag(html, i) else {
            i += 1;
            continue;
        };
        if !tag.closing && matches!(tag.name.as_str(), "script" | "style" | "textarea" | "title") {
            let close = format!("</{}", tag.name);
            i = find_ci(html, tag.end, &close).unwrap_or(html.len());
            continue;
        }
        let start = i;
        i = tag.end;
        return Some((start, tag));
    })
}

/// Top-level slide sections in document order.
pub fn find_slides(html: &str) -> Vec<SlideSpan> {
    let mut slides = Vec::new();
    let mut open: Option<(SlideSpan, usize)> = None; // (slide, nested <section> depth)
    for (i, tag) in tags(html) {
        if tag.name != "section" {
            continue;
        }
        match (&mut open, tag.closing) {
            (Some((_, depth)), false) => *depth += 1,
            (Some((_, depth)), true) if *depth > 0 => *depth -= 1,
            (Some(_), true) => {
                let (mut slide, _) = open.take().unwrap();
                slide.range.end = tag.end;
                slides.push(slide);
            }
            (None, false) if has_class(&tag, "slide") => {
                let id = tag.attrs.iter().find(|(n, _, _, _)| n == "id");
                let flag = |attr: &str| {
                    tag.attrs
                        .iter()
                        .find(|(n, _, _, _)| n == attr)
                        .map(|(_, _, _, r)| r.clone())
                };
                open = Some((
                    SlideSpan {
                        range: i..i,
                        id: id.map(|(_, v, _, _)| v.clone()).filter(|v| !v.is_empty()),
                        id_value: id.map(|(_, _, r, _)| r.clone()),
                        tag_name_end: tag.name_end,
                        hidden: flag(HIDDEN_ATTR),
                        locked: flag(LOCKED_ATTR),
                    },
                    0,
                ));
            }
            _ => {}
        }
    }
    slides
}

/// Section markers between slides, in document order. Markers inside a slide are content
/// of that slide, not sections.
pub fn find_sections(html: &str) -> Vec<SectionSpan> {
    let slides = find_slides(html);
    let mut sections = Vec::new();
    for (i, tag) in tags(html) {
        if tag.closing
            || tag.name != "div"
            || !has_class(&tag, SECTION_CLASS)
            || slides.iter().any(|s| s.range.contains(&i))
        {
            continue;
        }
        let title = tag.attrs.iter().find(|(n, ..)| n == SECTION_TITLE_ATTR);
        let rest = &html[tag.end..];
        let after_ws = rest.trim_start();
        let closer = after_ws
            .get(..5)
            .filter(|c| c.eq_ignore_ascii_case("</div"))
            .and_then(|_| after_ws.find('>'))
            .map(|e| tag.end + (rest.len() - after_ws.len()) + e + 1);
        sections.push(SectionSpan {
            range: i..closer.unwrap_or(tag.end),
            title: title
                .map(|(_, v, ..)| decode_entities(v).trim().to_string())
                .unwrap_or_default(),
            title_value: title.map(|(_, _, r, _)| r.clone()),
            tag_name_end: tag.name_end,
            before: slides.iter().filter(|s| s.range.end <= i).count(),
            empty: closer.is_some(),
        });
    }
    sections
}

pub(crate) fn has_class(tag: &Tag, class: &str) -> bool {
    tag.attrs
        .iter()
        .any(|(n, v, _, _)| n == "class" && v.split_ascii_whitespace().any(|c| c == class))
}

pub(crate) fn find_ci(html: &str, from: usize, needle: &str) -> Option<usize> {
    html.get(from..)?
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
        .map(|p| from + p)
}

pub(crate) fn parse_tag(html: &str, start: usize) -> Option<Tag> {
    let bytes = html.as_bytes();
    let mut i = start + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = html[name_start..i].to_ascii_lowercase();
    let name_end = i;
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => break,
            _ => {}
        }
        let attr_start = i;
        while i < bytes.len()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
            && !bytes[i].is_ascii_whitespace()
        {
            i += 1;
        }
        let attr = html[attr_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            let name_end = attr_start + attr.len();
            attrs.push((attr, String::new(), i..i, attr_start..name_end));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = match bytes.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let end = html[i + 1..].find(q as char)? + i + 1;
                let range = i + 1..end;
                i = end + 1;
                range
            }
            _ => {
                let s = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                s..i
            }
        };
        attrs.push((
            attr,
            html[value.clone()].to_string(),
            value.clone(),
            attr_start..i,
        ));
    }
    Some(Tag {
        name,
        closing,
        attrs,
        name_end,
        end: i + 1,
    })
}

pub fn content_hash(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Hash of everything except the slides, section markers and review marks: shared styles,
/// fonts, runtime. A marker's (or the review block's) leading whitespace goes with it, so
/// adding or removing one leaves the hash alone.
pub fn shell_hash(html: &str, slides: &[SlideSpan], sections: &[SectionSpan]) -> String {
    let mut cut: Vec<Range<usize>> = slides.iter().map(|s| s.range.clone()).collect();
    let with_indent = |range: &Range<usize>| html[..range.start].trim_end().len()..range.end;
    cut.extend(sections.iter().map(|s| with_indent(&s.range)));
    cut.extend(crate::review::block_range(html).map(|r| with_indent(&r)));
    cut.sort_by_key(|r| r.start);
    let mut shell = String::with_capacity(html.len());
    let mut at = 0;
    for range in cut {
        if range.start >= at {
            shell.push_str(&html[at..range.start]);
        }
        at = at.max(range.end);
    }
    shell.push_str(&html[at..]);
    content_hash(&shell)
}

pub fn title(html: &str) -> Option<String> {
    let start = find_ci(html, 0, "<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let end = find_ci(html, open_end, "</title")?;
    let raw = html[open_end..end].trim();
    (!raw.is_empty()).then(|| decode_entities(raw))
}

pub fn set_title(html: &str, title: &str) -> String {
    let escaped = escape(title);
    if let Some(start) = find_ci(html, 0, "<title") {
        if let (Some(open), Some(end)) = (html[start..].find('>'), find_ci(html, start, "</title"))
        {
            return format!("{}{}{}", &html[..start + open + 1], escaped, &html[end..]);
        }
    }
    insert_after_head(html, &format!("\n  <title>{escaped}</title>"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn insert_after_head(html: &str, snippet: &str) -> String {
    let at = find_ci(html, 0, "<head")
        .and_then(|s| html[s..].find('>').map(|e| s + e + 1))
        .unwrap_or(0);
    format!("{}{}{}", &html[..at], snippet, &html[at..])
}

/// Installs (or refreshes) the player runtime: base CSS first in `<head>` so deck styles
/// override it, and the script at the end of `<body>`.
pub fn ensure_runtime(html: &str) -> String {
    let css =
        format!("{CSS_START}\n  <style>\n{RUNTIME_CSS}  </style>\n  {PLAYER_FLAG}\n  {CSS_END}");
    let js = format!("{JS_START}\n  <script>\n{RUNTIME_JS}  </script>\n  {JS_END}");
    let html = match replace_block(html, CSS_START, CSS_END, &css) {
        Some(updated) => updated,
        None => insert_after_head(html, &format!("\n  {css}")),
    };
    match replace_block(&html, JS_START, JS_END, &js) {
        Some(updated) => updated,
        None => {
            let at = html
                .to_ascii_lowercase()
                .rfind("</body")
                .unwrap_or(html.len());
            format!("{}  {js}\n{}", &html[..at], &html[at..])
        }
    }
}

fn replace_block(html: &str, start: &str, end: &str, block: &str) -> Option<String> {
    let s = html.find(start)?;
    let e = html[s..].find(end)? + s + end.len();
    Some(format!("{}{}{}", &html[..s], block, &html[e..]))
}

pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(48).collect();
    slug.trim_end_matches('-').to_string()
}

fn unique_id(taken: &HashSet<String>, base: &str) -> String {
    let base = if base.is_empty() { "slide" } else { base };
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c))
        .expect("unbounded")
}

/// Gives every slide a unique, non-empty id. Returns `None` when nothing changed.
pub fn normalize_ids(html: &str) -> Option<String> {
    let slides = find_slides(html);
    let mut taken = HashSet::new();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for (index, slide) in slides.iter().enumerate() {
        match &slide.id {
            Some(id) if taken.insert(id.clone()) => {}
            existing => {
                let base = existing
                    .as_deref()
                    .map(slugify)
                    .unwrap_or_else(|| format!("slide-{}", index + 1));
                let id = unique_id(&taken, &base);
                taken.insert(id.clone());
                match &slide.id_value {
                    Some(range) => edits.push((range.clone(), id)),
                    None => edits.push((
                        slide.tag_name_end..slide.tag_name_end,
                        format!(" id=\"{id}\""),
                    )),
                }
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut out = html.to_string();
    for (range, text) in edits.into_iter().rev() {
        out.replace_range(range, &text);
    }
    Some(out)
}

fn span_of<'a>(slides: &'a [SlideSpan], id: &str) -> Option<&'a SlideSpan> {
    slides.iter().find(|s| s.id.as_deref() == Some(id))
}

/// Whitespace between the previous line break and `at`, used to indent inserted slides.
fn indent_before(html: &str, at: usize) -> &str {
    let line_start = html[..at].rfind('\n').map_or(0, |p| p + 1);
    let indent = &html[line_start..at];
    if indent.trim().is_empty() {
        indent
    } else {
        "    "
    }
}

/// Reorders slides and section markers. `order` lists every slide id and every marker key
/// (see [`section_key`]) in the new order; the items trade places, everything between them
/// stays where it is.
pub fn reorder(html: &str, order: &[String]) -> Result<String, String> {
    let slides = find_slides(html);
    let sections = find_sections(html);
    let mut items: Vec<(Range<usize>, Option<String>)> = slides
        .iter()
        .map(|s| (s.range.clone(), s.id.clone()))
        .chain(
            sections
                .iter()
                .enumerate()
                .map(|(i, s)| (s.range.clone(), Some(section_key(i)))),
        )
        .collect();
    items.sort_by_key(|(range, _)| range.start);
    let mut current: Vec<_> = items.iter().filter_map(|(_, key)| key.clone()).collect();
    let mut proposed = order.to_vec();
    current.sort();
    proposed.sort();
    if current != proposed || current.len() != items.len() {
        return Err("The slides changed while reordering; try again.".into());
    }
    let range_of = |key: &str| {
        items
            .iter()
            .find(|(_, k)| k.as_deref() == Some(key))
            .map(|(range, _)| range.clone())
            .expect("keys were validated")
    };
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    for ((slot, _), key) in items.iter().zip(order) {
        out.push_str(&html[at..slot.start]);
        out.push_str(&html[range_of(key)]);
        at = slot.end;
    }
    out.push_str(&html[at..]);
    Ok(out)
}

pub fn delete(html: &str, id: &str) -> Result<String, String> {
    let slides = find_slides(html);
    let span = span_of(&slides, id).ok_or_else(|| format!("Slide not found: {id}"))?;
    let start = html[..span.range.start].trim_end().len();
    Ok(format!("{}{}", &html[..start], &html[span.range.end..]))
}

/// Where new content goes in a deck without slides: just before the end of `<main>`.
fn end_of_deck(html: &str) -> Result<usize, String> {
    let lower = html.to_ascii_lowercase();
    let at = lower
        .rfind("</main")
        .or_else(|| lower.rfind("</body"))
        .ok_or("The deck has no <main class=\"deck\"> container.")?;
    Ok(html[..at].trim_end().len())
}

/// Inserts `section` (with `{{ID}}` replaced) after slide `after`, after the last slide,
/// or at the end of the `.deck` container when the deck is empty. Returns the new id.
pub fn insert(
    html: &str,
    after: Option<&str>,
    section: &str,
    id_hint: &str,
) -> Result<(String, String), String> {
    let slides = find_slides(html);
    let taken: HashSet<String> = slides.iter().filter_map(|s| s.id.clone()).collect();
    let id = unique_id(&taken, &slugify(id_hint));
    let section = section.replace("{{ID}}", &id);
    let anchor = after.and_then(|a| span_of(&slides, a)).or(slides.last());
    let (at, indent) = match anchor {
        Some(span) => (
            span.range.end,
            indent_before(html, span.range.start).to_string(),
        ),
        None => (end_of_deck(html)?, "    ".to_string()),
    };
    let out = format!(
        "{}\n{}{}{}",
        &html[..at],
        indent,
        section.trim(),
        &html[at..]
    );
    Ok((out, id))
}

/// Adds or removes the slide's `data-hidden` attribute. Leaves the markup untouched when
/// the slide is already in the requested state.
pub fn set_hidden(html: &str, id: &str, hidden: bool) -> Result<String, String> {
    set_flag(html, id, HIDDEN_ATTR, hidden, |s| s.hidden.clone())
}

/// Adds or removes the slide's `data-locked` attribute, like [`set_hidden`].
pub fn set_locked(html: &str, id: &str, locked: bool) -> Result<String, String> {
    set_flag(html, id, LOCKED_ATTR, locked, |s| s.locked.clone())
}

fn set_flag(
    html: &str,
    id: &str,
    attr: &str,
    on: bool,
    current: impl Fn(&SlideSpan) -> Option<Range<usize>>,
) -> Result<String, String> {
    let slides = find_slides(html);
    let span = span_of(&slides, id).ok_or_else(|| format!("Slide not found: {id}"))?;
    Ok(match (current(span), on) {
        (None, true) => format!(
            "{} {attr}{}",
            &html[..span.tag_name_end],
            &html[span.tag_name_end..]
        ),
        (Some(range), false) => {
            let start = html[..range.start].trim_end().len();
            format!("{}{}", &html[..start], &html[range.end..])
        }
        _ => html.to_string(),
    })
}

/// Whether slide `id` exists and is locked.
pub fn is_locked(html: &str, id: &str) -> bool {
    span_of(&find_slides(html), id).is_some_and(|s| s.locked.is_some())
}

/// A locked slide as it was when a guard was set up: what it must stay like.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LockedSlide {
    pub id: String,
    pub markup: String,
    /// Id of the slide before it, where it goes back if it is removed; None when it was first.
    pub after: Option<String>,
}

/// Every locked slide of `html`, in order.
pub fn locked_slides(html: &str) -> Vec<LockedSlide> {
    let slides = find_slides(html);
    slides
        .iter()
        .enumerate()
        .filter(|(_, s)| s.locked.is_some())
        .filter_map(|(i, s)| {
            Some(LockedSlide {
                id: s.id.clone()?,
                markup: html[s.range.clone()].to_string(),
                after: i.checked_sub(1).and_then(|p| slides[p].id.clone()),
            })
        })
        .collect()
}

/// Ids of the `locked` slides that `html` changed or removed. Moving one is not a change.
pub fn changed_locked(html: &str, locked: &[LockedSlide]) -> Vec<String> {
    let slides = find_slides(html);
    locked
        .iter()
        .filter(|l| {
            span_of(&slides, &l.id).map(|s| &html[s.range.clone()]) != Some(l.markup.as_str())
        })
        .map(|l| l.id.clone())
        .collect()
}

/// `html` with every `locked` slide as it was: changed ones get their markup back, removed
/// ones go back after the slide they followed (or first). Returns the ids it put back.
pub fn restore_locked(html: &str, locked: &[LockedSlide]) -> (String, Vec<String>) {
    let mut out = html.to_string();
    let mut restored = Vec::new();
    for slide in locked {
        let slides = find_slides(&out);
        match span_of(&slides, &slide.id) {
            Some(span) if out[span.range.clone()] == slide.markup => continue,
            Some(span) => out.replace_range(span.range.clone(), &slide.markup),
            None => {
                let anchor = match &slide.after {
                    Some(after) => span_of(&slides, after).or(slides.last()),
                    None => None,
                };
                let first = slides.first().filter(|_| anchor.is_none());
                let (at, indent, before) = match (anchor, first) {
                    (Some(a), _) => (a.range.end, indent_before(&out, a.range.start), false),
                    (None, Some(f)) => (f.range.start, indent_before(&out, f.range.start), true),
                    (None, None) => match end_of_deck(&out) {
                        Ok(at) => (at, "    ", false),
                        Err(_) => continue,
                    },
                };
                let insert = if before {
                    format!("{}\n{indent}", slide.markup)
                } else {
                    format!("\n{indent}{}", slide.markup)
                };
                out.insert_str(at, &insert);
            }
        }
        restored.push(slide.id.clone());
    }
    (out, restored)
}

fn section_span(sections: &[SectionSpan], index: usize) -> Result<&SectionSpan, String> {
    sections
        .get(index)
        .ok_or_else(|| format!("Section not found: {index}"))
}

/// Starts a new section with `title` right before slide `before`, or after the last slide
/// when `before` is `None`.
pub fn add_section(html: &str, before: Option<&str>, title: &str) -> Result<String, String> {
    let slides = find_slides(html);
    let marker = format!(
        "<div class=\"{SECTION_CLASS}\" {SECTION_TITLE_ATTR}=\"{}\"></div>",
        escape_attr(title.trim())
    );
    match before {
        Some(id) => {
            let span = span_of(&slides, id).ok_or_else(|| format!("Slide not found: {id}"))?;
            let at = span.range.start;
            let indent = indent_before(html, at);
            Ok(format!("{}{marker}\n{indent}{}", &html[..at], &html[at..]))
        }
        None => {
            let (at, indent) = match slides.last() {
                Some(last) => (
                    last.range.end,
                    indent_before(html, last.range.start).to_string(),
                ),
                None => (end_of_deck(html)?, "    ".to_string()),
            };
            Ok(format!("{}\n{indent}{marker}{}", &html[..at], &html[at..]))
        }
    }
}

/// Sets the title of the section with this document-order index, keeping its other attributes.
pub fn rename_section(html: &str, index: usize, title: &str) -> Result<String, String> {
    let sections = find_sections(html);
    let span = section_span(&sections, index)?;
    let title = escape_attr(title.trim());
    Ok(match &span.title_value {
        Some(value) => format!("{}{title}{}", &html[..value.start], &html[value.end..]),
        None => format!(
            "{} {SECTION_TITLE_ATTR}=\"{title}\"{}",
            &html[..span.tag_name_end],
            &html[span.tag_name_end..]
        ),
    })
}

/// Removes a section marker; its slides stay and join the previous section.
pub fn delete_section(html: &str, index: usize) -> Result<String, String> {
    let sections = find_sections(html);
    let span = section_span(&sections, index)?;
    let start = html[..span.range.start].trim_end().len();
    Ok(format!("{}{}", &html[..start], &html[span.range.end..]))
}

/// Whether the slide markup contains an element moved by hand (see [`MOVED_ATTR`]).
pub fn has_moved(slide: &str) -> bool {
    tags(slide).any(|(_, tag)| !tag.closing && tag.attrs.iter().any(|a| a.0 == MOVED_ATTR))
}

/// Replaces slide `id` with `markup`, which must be one whole `<section class="slide">` with
/// the same id. Returns the new document and the slide's previous markup.
pub fn replace_slide(html: &str, id: &str, markup: &str) -> Result<(String, String), String> {
    let markup = markup.trim();
    let replacement = find_slides(markup);
    match replacement.as_slice() {
        [only] if only.range == (0..markup.len()) && only.id.as_deref() == Some(id) => {}
        _ => return Err(format!("The edited markup is not slide `{id}`.")),
    }
    let slides = find_slides(html);
    let span = span_of(&slides, id).ok_or_else(|| format!("Slide not found: {id}"))?;
    let previous = html[span.range.clone()].to_string();
    let out = format!(
        "{}{markup}{}",
        &html[..span.range.start],
        &html[span.range.end..]
    );
    Ok((out, previous))
}

/// Slide `id` of `html` with its id replaced by the `{{ID}}` placeholder [`insert`] fills in.
fn slide_with_id_placeholder(html: &str, id: &str) -> Result<String, String> {
    let slides = find_slides(html);
    let span = span_of(&slides, id).ok_or_else(|| format!("Slide not found: {id}"))?;
    let range = span
        .id_value
        .clone()
        .expect("slides with ids have id ranges");
    Ok(format!(
        "{}{{{{ID}}}}{}",
        &html[span.range.start..range.start],
        &html[range.end..span.range.end]
    ))
}

/// Inserts a copy of slide `id` right after it. The copy is not locked, so it can be changed.
pub fn duplicate(html: &str, id: &str) -> Result<(String, String), String> {
    let copy = slide_with_id_placeholder(html, id)?;
    let (out, new_id) = insert(html, Some(id), &copy, &format!("{id}-copy"))?;
    Ok((set_locked(&out, &new_id, false)?, new_id))
}

/// Inserts a copy of slide `slide` of the document `source` (a template) after `after`,
/// keeping its id when the deck does not use it yet. Returns the document and the new id.
pub fn copy_slide(
    html: &str,
    after: Option<&str>,
    source: &str,
    slide: &str,
) -> Result<(String, String), String> {
    let copy = slide_with_id_placeholder(source, slide)?;
    let (out, id) = insert(html, after, &copy, slide)?;
    Ok((set_locked(&out, &id, false)?, id))
}

/// Name of the `<meta>` naming the template a deck's design comes from:
/// `<meta name="slopslide-template" content="<template id>">`.
pub const TEMPLATE_META: &str = "slopslide-template";

/// The template `<meta>` tag: where it starts and its parsed tag.
fn template_meta(html: &str) -> Option<(usize, Tag)> {
    tags(html).find(|(_, tag)| {
        !tag.closing
            && tag.name == "meta"
            && tag
                .attrs
                .iter()
                .any(|(n, v, _, _)| n == "name" && v.trim().eq_ignore_ascii_case(TEMPLATE_META))
    })
}

/// Id of the template the deck's design comes from (see [`TEMPLATE_META`]).
pub fn template(html: &str) -> Option<String> {
    let (_, tag) = template_meta(html)?;
    let content = tag.attrs.iter().find(|(n, _, _, _)| n == "content")?;
    let id = decode_entities(content.1.trim());
    (!id.is_empty()).then_some(id)
}

/// Names the deck's template in its `<meta>` (right after `<title>`), or removes it for None.
pub fn set_template(html: &str, template: Option<&str>) -> String {
    let tag = template.map(|id| {
        format!(
            "<meta name=\"{TEMPLATE_META}\" content=\"{}\">",
            escape_attr(id)
        )
    });
    match (template_meta(html), tag) {
        (Some((at, old)), Some(tag)) => format!("{}{tag}{}", &html[..at], &html[old.end..]),
        (Some((at, old)), None) => {
            let start = html[..at].trim_end().len();
            format!("{}{}", &html[..start], &html[old.end..])
        }
        (None, Some(tag)) => match find_ci(html, 0, "</title") {
            Some(end) => {
                let at = html[end..].find('>').map_or(html.len(), |e| end + e + 1);
                format!("{}\n  {tag}{}", &html[..at], &html[at..])
            }
            None => insert_after_head(html, &format!("\n  {tag}")),
        },
        (None, None) => html.to_string(),
    }
}

/// `html` without its slides and section markers: the deck's shell (styles, runtime).
pub fn strip_slides(html: &str) -> String {
    let mut ranges: Vec<Range<usize>> = find_slides(html)
        .into_iter()
        .map(|s| s.range)
        .chain(find_sections(html).into_iter().map(|s| s.range))
        .collect();
    ranges.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    for range in ranges {
        let start = html[at..range.start].trim_end().len() + at;
        out.push_str(&html[at..start]);
        at = range.end;
    }
    out.push_str(&html[at..]);
    out
}

/// `html` without the player runtime blocks, e.g. for the agent to read a template.
pub fn without_runtime(html: &str) -> String {
    let mut out = html.to_string();
    for (start, end) in [(CSS_START, CSS_END), (JS_START, JS_END)] {
        if let Some(s) = out.find(start) {
            if let Some(e) = out[s..].find(end) {
                let from = out[..s].trim_end().len();
                out = format!("{}{}", &out[..from], &out[s + e + end.len()..]);
            }
        }
    }
    out
}

/// Replaces every word of the slides' text with placeholder (lorem ipsum) words of about
/// the same length and case, and every digit with another digit, keeping all markup,
/// styles, and entities. Turns a deck into a template without its content.
pub fn with_placeholder_text(html: &str) -> String {
    let mut words = PlaceholderWords::default();
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    for slide in find_slides(html) {
        out.push_str(&html[at..slide.range.start]);
        let s = &html[slide.range.clone()];
        let mut i = 0;
        while i < s.len() {
            let next = s[i..].find('<').map_or(s.len(), |e| i + e);
            out.push_str(&words.replace(&s[i..next]));
            if next == s.len() {
                break;
            }
            let end = if s[next..].starts_with("<!--") {
                s[next..].find("-->").map_or(s.len(), |e| next + e + 3)
            } else {
                match parse_tag(s, next) {
                    Some(tag)
                        if !tag.closing
                            && matches!(tag.name.as_str(), "script" | "style" | "textarea") =>
                    {
                        find_ci(s, tag.end, &format!("</{}", tag.name)).unwrap_or(s.len())
                    }
                    Some(tag) => tag.end,
                    None => next + 1,
                }
            };
            out.push_str(&s[next..end]);
            i = end;
        }
        at = slide.range.end;
    }
    out.push_str(&html[at..]);
    out
}

const LOREM: &[&str] = &[
    "a",
    "ad",
    "et",
    "in",
    "ut",
    "id",
    "non",
    "sed",
    "est",
    "sit",
    "amet",
    "elit",
    "enim",
    "quis",
    "nisi",
    "lorem",
    "ipsum",
    "dolor",
    "magna",
    "minim",
    "culpa",
    "labore",
    "dolore",
    "veniam",
    "fugiat",
    "tempor",
    "aliqua",
    "nostrud",
    "officia",
    "laboris",
    "commodo",
    "pariatur",
    "voluptate",
    "adipiscing",
    "incididunt",
    "consectetur",
    "exercitation",
    "reprehenderit",
];
const DIGITS: &[u8] = b"4827365190";

#[derive(Default)]
struct PlaceholderWords {
    next: usize,
}

impl PlaceholderWords {
    fn replace(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '&' {
                // Keep entities such as `&amp;` or `&#8212;` as they are.
                let entity = text[i..].find(';').filter(|&e| {
                    e > 1
                        && e <= 10
                        && text[i + 1..i + e]
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '#')
                });
                if let Some(e) = entity {
                    out.push_str(&text[i..=i + e]);
                    while chars.peek().is_some_and(|&(j, _)| j <= i + e) {
                        chars.next();
                    }
                    continue;
                }
            }
            if c.is_alphabetic() {
                let mut word = String::from(c);
                while let Some(&(_, next)) = chars.peek().filter(|(_, n)| n.is_alphabetic()) {
                    word.push(next);
                    chars.next();
                }
                out.push_str(&self.word(&word));
            } else if c.is_ascii_digit() {
                out.push(DIGITS[self.next % DIGITS.len()] as char);
                self.next += 1;
            } else {
                out.push(c);
            }
        }
        out
    }

    /// A placeholder word as long as `original` as the list allows, in the same case.
    fn word(&mut self, original: &str) -> String {
        let len = original.chars().count();
        let distance = |w: &&str| w.len().abs_diff(len);
        let best = LOREM.iter().map(distance).min().unwrap_or(0);
        let fits: Vec<&str> = LOREM
            .iter()
            .filter(|w| distance(w) == best)
            .copied()
            .collect();
        let word = fits[self.next % fits.len()];
        self.next += 1;
        let mut letters = original.chars();
        let first_upper = letters.next().is_some_and(char::is_uppercase);
        if first_upper && len > 1 && original.chars().all(char::is_uppercase) {
            word.to_uppercase()
        } else if first_upper {
            let mut w = word.chars();
            w.next()
                .map(|f| f.to_uppercase().chain(w).collect())
                .unwrap_or_default()
        } else {
            word.to_string()
        }
    }
}

/// Quoted deck-relative `assets/…` references in document order: the byte range of each
/// reference (including a leading `./`) and the path without query or fragment.
pub fn asset_refs(html: &str) -> Vec<(Range<usize>, &str)> {
    let mut refs = Vec::new();
    let mut at = 0;
    for (pos, _) in html.match_indices("assets/") {
        if pos < at {
            continue;
        }
        let mut start = pos;
        if html[..pos].ends_with("./") {
            start -= 2;
        }
        if !html[..start].ends_with(['"', '\'', '(']) {
            continue;
        }
        let end = html[pos..]
            .find(|c: char| matches!(c, '"' | '\'' | ')' | '?' | '#') || c.is_whitespace())
            .map_or(html.len(), |e| pos + e);
        refs.push((start..end, &html[pos..end]));
        at = end;
    }
    refs
}

/// Replaces deck-relative `assets/…` references with data URIs so the file stands alone.
pub fn inline_assets(
    html: &str,
    mut load: impl FnMut(&str) -> Option<(String, Vec<u8>)>,
) -> String {
    use base64::Engine;
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    for (range, path) in asset_refs(html) {
        let Some((mime, bytes)) = load(path) else {
            continue;
        };
        out.push_str(&html[at..range.start]);
        out.push_str(&format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ));
        at = range.end;
    }
    out.push_str(&html[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECK: &str = r#"<!DOCTYPE html><html><head><title>Q3 &amp; more</title>
<style>.slide { color: red } /* <section class="slide"> */</style></head>
<body>
  <main class="deck">
    <section class="slide intro" id="intro"><h1>Hi</h1><section class="inner">x</section></section>
    <!-- <section class="slide" id="ghost"></section> -->
    <section class='slide' id=plan><p>Plan</p></section>
    <section class="slide"><p>No id</p></section>
  </main>
  <script>const s = '<section class="slide" id="fake">';</script>
</body></html>"#;

    /// Cases shared with the frontend's copy of the slide finder (src/lib/slideSpans.ts),
    /// so the two cannot drift apart.
    #[test]
    fn matches_shared_slide_fixtures() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/slide-spans.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let html = case["html"].as_str().unwrap();
            let actual: Vec<(String, &str)> = find_slides(html)
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let id = s.id.clone().unwrap_or_else(|| format!("#{}", i + 1));
                    (id, &html[s.range.clone()])
                })
                .collect();
            let expected: Vec<(String, &str)> = case["slides"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    (
                        s["id"].as_str().unwrap().to_string(),
                        s["source"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(actual, expected, "fixture: {name}");
        }
    }

    fn ids(html: &str) -> Vec<String> {
        find_slides(html)
            .into_iter()
            .map(|s| s.id.unwrap_or_default())
            .collect()
    }

    #[test]
    fn replaces_one_slide_and_returns_the_old_markup() {
        let html = "<main class=\"deck\">\n  <section class=\"slide\" id=\"a\"><p>A</p></section>\n  <section class=\"slide\" id=\"b\">B</section>\n</main>";
        let edited = "  <section class=\"slide\" id=\"a\"><p style=\"translate: 4px 2px\" data-moved=\"\">A!</p></section>\n";
        let (out, previous) = replace_slide(html, "a", edited).unwrap();
        assert_eq!(
            previous,
            "<section class=\"slide\" id=\"a\"><p>A</p></section>"
        );
        assert_eq!(
            out,
            "<main class=\"deck\">\n  <section class=\"slide\" id=\"a\"><p style=\"translate: 4px 2px\" data-moved=\"\">A!</p></section>\n  <section class=\"slide\" id=\"b\">B</section>\n</main>"
        );
        // Restoring the previous markup undoes the edit exactly.
        assert_eq!(replace_slide(&out, "a", &previous).unwrap().0, html);
    }

    #[test]
    fn refuses_markup_that_is_not_the_slide() {
        let html = "<main class=\"deck\"><section class=\"slide\" id=\"a\">A</section></main>";
        for bad in [
            "<section class=\"slide\" id=\"b\">A</section>",
            "<section class=\"slide\">A</section>",
            "<div>A</div>",
            "<section class=\"slide\" id=\"a\">A</section><section class=\"slide\" id=\"a\">A</section>",
            "<section class=\"slide\" id=\"a\">A</section><p>trailing</p>",
            "<section class=\"slide\" id=\"a\">unclosed",
            "",
        ] {
            assert!(replace_slide(html, "a", bad).is_err(), "accepted {bad:?}");
        }
        assert!(
            replace_slide(html, "zz", "<section class=\"slide\" id=\"zz\"></section>").is_err()
        );
    }

    #[test]
    fn detects_hand_moved_elements() {
        assert!(has_moved(
            r#"<section class="slide" id="a"><p data-moved style="translate: 1px 2px">A</p></section>"#
        ));
        assert!(!has_moved(
            r#"<section class="slide" id="a"><p>data-moved</p><!-- <p data-moved> --></section>"#
        ));
    }

    #[test]
    fn finds_top_level_slides_only() {
        assert_eq!(ids(DECK), ["intro", "plan", ""]);
        let first = &find_slides(DECK)[0];
        assert!(DECK[first.range.clone()].ends_with("x</section></section>"));
    }

    #[test]
    fn normalizes_missing_and_duplicate_ids() {
        let fixed = normalize_ids(DECK).unwrap();
        assert_eq!(ids(&fixed), ["intro", "plan", "slide-3"]);
        let dup = r#"<main class="deck"><section class="slide" id="a"></section><section class="slide" id="a"></section></main>"#;
        assert_eq!(ids(&normalize_ids(dup).unwrap()), ["a", "a-2"]);
        assert!(normalize_ids(&fixed).is_none());
    }

    #[test]
    fn reorders_deletes_duplicates_and_inserts() {
        let deck = normalize_ids(DECK).unwrap();
        let order = vec![
            "slide-3".to_string(),
            "intro".to_string(),
            "plan".to_string(),
        ];
        assert_eq!(ids(&reorder(&deck, &order).unwrap()), order);
        assert!(reorder(&deck, &order[..2]).is_err());

        assert_eq!(ids(&delete(&deck, "plan").unwrap()), ["intro", "slide-3"]);

        let (copied, id) = duplicate(&deck, "intro").unwrap();
        assert_eq!(id, "intro-copy");
        assert_eq!(ids(&copied), ["intro", "intro-copy", "plan", "slide-3"]);

        let (added, id) = insert(
            &deck,
            Some("intro"),
            r#"<section class="slide" id="{{ID}}"></section>"#,
            "slide",
        )
        .unwrap();
        assert_eq!(id, "slide");
        assert_eq!(ids(&added), ["intro", "slide", "plan", "slide-3"]);
    }

    fn locked(html: &str) -> Vec<bool> {
        find_slides(html)
            .into_iter()
            .map(|s| s.locked.is_some())
            .collect()
    }

    const LOCKED_DECK: &str = "<main class=\"deck\">\n    <section class=\"slide\" id=\"a\">A</section>\n    <section class=\"slide\" id=\"b\" data-locked>B</section>\n    <section class=\"slide\" id=\"c\">C</section>\n    <section class=\"slide\" id=\"d\" data-locked>D</section>\n  </main>";

    #[test]
    fn locks_and_unlocks_slides() {
        let deck = normalize_ids(DECK).unwrap();
        let locked_deck = set_locked(&deck, "plan", true).unwrap();
        assert_eq!(locked(&locked_deck), [false, true, false]);
        assert!(locked_deck.contains("<section data-locked class='slide' id=plan>"));
        assert!(is_locked(&locked_deck, "plan") && !is_locked(&locked_deck, "intro"));
        assert!(!is_locked(&locked_deck, "missing"));
        assert_eq!(set_locked(&locked_deck, "plan", true).unwrap(), locked_deck);
        assert_eq!(set_locked(&locked_deck, "plan", false).unwrap(), deck);
        assert!(set_locked(&deck, "missing", true).is_err());

        let both = set_hidden(&locked_deck, "plan", true).unwrap();
        assert_eq!(
            (hidden(&both), locked(&both)),
            (vec![false, true, false], vec![false, true, false])
        );
        let unlocked = set_locked(&both, "plan", false).unwrap();
        assert_eq!(locked(&unlocked), [false, false, false]);
        assert_eq!(
            hidden(&unlocked),
            [false, true, false],
            "unlocking keeps it hidden"
        );
    }

    #[test]
    fn copies_of_locked_slides_are_unlocked() {
        let deck = set_locked(&normalize_ids(DECK).unwrap(), "intro", true).unwrap();
        let (copied, id) = duplicate(&deck, "intro").unwrap();
        assert_eq!(id, "intro-copy");
        assert_eq!(locked(&copied), [true, false, false, false]);

        let (copied, id) = copy_slide(&normalize_ids(DECK).unwrap(), None, &deck, "intro").unwrap();
        assert_eq!(id, "intro-2");
        assert_eq!(locked(&copied), [false, false, false, false]);
    }

    #[test]
    fn lists_locked_slides_with_their_predecessor() {
        let locked = locked_slides(LOCKED_DECK);
        assert_eq!(
            locked,
            [
                LockedSlide {
                    id: "b".into(),
                    markup: "<section class=\"slide\" id=\"b\" data-locked>B</section>".into(),
                    after: Some("a".into()),
                },
                LockedSlide {
                    id: "d".into(),
                    markup: "<section class=\"slide\" id=\"d\" data-locked>D</section>".into(),
                    after: Some("c".into()),
                },
            ]
        );
        let first = LOCKED_DECK.replace("id=\"a\">", "id=\"a\" data-locked>");
        assert_eq!(locked_slides(&first)[0].after, None);
        assert!(locked_slides(DECK).is_empty());
    }

    #[test]
    fn detects_changed_locked_slides() {
        let locked = locked_slides(LOCKED_DECK);
        assert!(changed_locked(LOCKED_DECK, &locked).is_empty());
        let edited_others = LOCKED_DECK.replace(">A<", ">A!<").replace(">C<", ">C!<");
        assert!(changed_locked(&edited_others, &locked).is_empty());
        let moved = reorder(LOCKED_DECK, &["d", "c", "b", "a"].map(String::from)).unwrap();
        assert!(
            changed_locked(&moved, &locked).is_empty(),
            "moving is no change"
        );

        assert_eq!(
            changed_locked(&LOCKED_DECK.replace(">B<", ">B!<"), &locked),
            ["b"]
        );
        assert_eq!(
            changed_locked(
                &LOCKED_DECK.replace(" id=\"d\" data-locked", " id=\"d\""),
                &locked
            ),
            ["d"],
            "unlocking is a change"
        );
        assert_eq!(
            changed_locked(&delete(LOCKED_DECK, "b").unwrap(), &locked),
            ["b"]
        );
    }

    #[test]
    fn restores_changed_and_removed_locked_slides() {
        let locked = locked_slides(LOCKED_DECK);
        let (same, restored) = restore_locked(LOCKED_DECK, &locked);
        assert_eq!((same.as_str(), restored.len()), (LOCKED_DECK, 0));

        let edited = LOCKED_DECK
            .replace(">A<", ">A!<")
            .replace(">B<", ">B!<")
            .replace(" id=\"d\" data-locked", " id=\"d\"");
        let (out, restored) = restore_locked(&edited, &locked);
        assert_eq!(restored, ["b", "d"]);
        assert_eq!(out, LOCKED_DECK.replace(">A<", ">A!<"), "other edits stay");

        let removed = delete(&delete(LOCKED_DECK, "b").unwrap(), "d").unwrap();
        let (out, restored) = restore_locked(&removed, &locked);
        assert_eq!(restored, ["b", "d"]);
        assert_eq!(
            out, LOCKED_DECK,
            "back in place, indented like its neighbors"
        );

        // Its predecessor is gone too: it goes to the end.
        let gone = delete(&delete(LOCKED_DECK, "a").unwrap(), "b").unwrap();
        let (out, _) = restore_locked(&gone, &locked);
        assert_eq!(ids(&out), ["c", "d", "b"]);

        // A locked first slide goes back first.
        let first = LOCKED_DECK.replace("id=\"a\">", "id=\"a\" data-locked>");
        let locked = locked_slides(&first);
        let (out, _) = restore_locked(&delete(&first, "a").unwrap(), &locked);
        assert_eq!(out, first);

        // Into a deck the agent emptied.
        let empty = "<main class=\"deck\">\n  </main>";
        let (out, restored) = restore_locked(empty, &locked);
        assert_eq!(restored, ["a", "b", "d"]);
        assert_eq!(ids(&out), ["a", "b", "d"]);
    }

    fn hidden(html: &str) -> Vec<bool> {
        find_slides(html)
            .into_iter()
            .map(|s| s.hidden.is_some())
            .collect()
    }

    #[test]
    fn detects_hidden_slides() {
        let html = r#"<main class="deck">
<section class="slide" id="a" data-hidden></section>
<section data-hidden="" class="slide" id="b"><section data-hidden>nested</section></section>
<section class="slide" id="c" data-hidden-not="x"></section>
</main>"#;
        assert_eq!(hidden(html), [true, true, false]);
        assert_eq!(ids(html), ["a", "b", "c"]);
    }

    #[test]
    fn hides_and_unhides_slides() {
        let deck = normalize_ids(DECK).unwrap();
        let hid = set_hidden(&deck, "plan", true).unwrap();
        assert_eq!(hidden(&hid), [false, true, false]);
        assert!(hid.contains("<section data-hidden class='slide' id=plan>"));
        assert_eq!(set_hidden(&hid, "plan", true).unwrap(), hid, "idempotent");

        let shown = set_hidden(&hid, "plan", false).unwrap();
        assert_eq!(shown, deck, "unhiding restores the original markup");
        assert_eq!(set_hidden(&shown, "plan", false).unwrap(), deck);

        let valued = deck.replace("id=plan>", r#"id=plan data-hidden="true">"#);
        assert_eq!(set_hidden(&valued, "plan", false).unwrap(), deck);

        assert!(set_hidden(&deck, "missing", true).is_err());
    }

    #[test]
    fn duplicating_a_hidden_slide_keeps_it_hidden() {
        let deck = set_hidden(&normalize_ids(DECK).unwrap(), "intro", true).unwrap();
        let (copied, _) = duplicate(&deck, "intro").unwrap();
        assert_eq!(hidden(&copied), [true, true, false, false]);
    }

    #[test]
    fn inserts_into_empty_deck() {
        let empty = "<html><body>\n  <main class=\"deck\">\n  </main>\n</body></html>";
        let (out, id) = insert(
            empty,
            None,
            r#"<section class="slide" id="{{ID}}"></section>"#,
            "Title",
        )
        .unwrap();
        assert_eq!(id, "title");
        assert_eq!(ids(&out), ["title"]);
        assert!(out.find("id=\"title\"").unwrap() < out.find("</main>").unwrap());
    }

    #[test]
    fn reads_and_writes_titles() {
        assert_eq!(title(DECK).as_deref(), Some("Q3 & more"));
        let renamed = set_title(DECK, "A <b> deck");
        assert_eq!(title(&renamed).as_deref(), Some("A <b> deck"));
        assert!(renamed.contains("<title>A &lt;b&gt; deck</title>"));
    }

    #[test]
    fn runtime_is_installed_once_and_refreshed() {
        let once = ensure_runtime(DECK);
        let twice = ensure_runtime(&once);
        assert_eq!(once, twice);
        assert_eq!(once.matches(CSS_START).count(), 1);
        assert!(once.find(CSS_START).unwrap() < once.find("<title>").unwrap());
        assert!(once.find(JS_START).unwrap() > once.find("</main>").unwrap());
        assert_eq!(ids(&once), ["intro", "plan", ""]);
    }

    #[test]
    fn inlines_referenced_assets() {
        let html = r#"<img src="assets/a.png"><div style="background:url('./assets/b.jpg')"></div> see assets/a.png"#;
        let out = inline_assets(html, |p| Some(("image/png".into(), p.as_bytes().to_vec())));
        assert!(out.contains(r#"src="data:image/png;base64,"#));
        assert!(out.contains("url('data:image/png;base64,"));
        assert!(
            out.ends_with("see assets/a.png"),
            "bare text is not a reference"
        );
    }

    #[test]
    fn records_id_value_ranges_and_tag_name_end() {
        let html = r#"<SECTION CLASS="slide" ID="Up"></SECTION><section class="slide"></section>"#;
        let slides = find_slides(html);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].id.as_deref(), Some("Up"));
        assert_eq!(&html[slides[0].id_value.clone().unwrap()], "Up");
        assert_eq!(
            &html[slides[0].range.start..slides[0].tag_name_end],
            "<SECTION"
        );
        assert!(slides[1].id_value.is_none());
        assert_eq!(
            &html[slides[1].range.clone()],
            r#"<section class="slide"></section>"#
        );
    }

    #[test]
    fn empty_id_attribute_counts_as_missing_but_keeps_its_range() {
        let html = r#"<section class="slide" id=""></section>"#;
        let slide = &find_slides(html)[0];
        assert_eq!(slide.id, None);
        let range = slide.id_value.clone().unwrap();
        assert!(range.is_empty());
        assert_eq!(&html[..range.start], r#"<section class="slide" id=""#);
    }

    #[test]
    fn unterminated_raw_text_and_tags_end_the_scan() {
        assert_eq!(
            ids(
                r#"<section class="slide" id="a"></section><textarea><section class="slide" id="b"></section>"#
            ),
            ["a"]
        );
        assert_eq!(
            ids(r#"<section class="slide" id="a"></section><section class="slide" id="b""#),
            ["a"]
        );
        assert!(find_slides("<").is_empty());
        assert!(find_slides("<<<>>>").is_empty());
    }

    #[test]
    fn handles_multibyte_text_around_slides() {
        let html = "<p>héllo 👋</p><section class=\"slide\" id=\"ü\">日本</section>";
        let slide = &find_slides(html)[0];
        assert_eq!(slide.id.as_deref(), Some("ü"));
        assert_eq!(
            &html[slide.range.clone()],
            "<section class=\"slide\" id=\"ü\">日本</section>"
        );
    }

    #[test]
    fn content_hash_is_stable_and_sensitive() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
        assert!(content_hash("").chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn shell_hash_ignores_slide_edits_but_not_shell_edits() {
        let deck = normalize_ids(DECK).unwrap();
        let hash = |html: &str| shell_hash(html, &find_slides(html), &find_sections(html));
        let slide_edit = deck.replace("<p>Plan</p>", "<p>New plan</p>");
        assert_eq!(hash(&deck), hash(&slide_edit));
        let style_edit = deck.replace("color: red", "color: blue");
        assert_ne!(hash(&deck), hash(&style_edit));
        let reordered = reorder(
            &deck,
            &[
                "plan".to_string(),
                "intro".to_string(),
                "slide-3".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(hash(&deck), hash(&reordered));
    }

    #[test]
    fn title_edge_cases() {
        assert_eq!(title("<html><head></head></html>"), None);
        assert_eq!(title("<title>   </title>"), None);
        assert_eq!(
            title("<TITLE lang=en> Spaced </TITLE>").as_deref(),
            Some("Spaced")
        );
        assert_eq!(
            title("<title>&lt;a&gt; &quot;b&quot; &#39;c&#39; &amp;amp;</title>").as_deref(),
            Some("<a> \"b\" 'c' &amp;"),
            "&amp; is decoded last so it cannot create new entities"
        );
    }

    #[test]
    fn set_title_inserts_a_title_when_missing() {
        let html = "<html><head><meta charset=\"utf-8\"></head><body></body></html>";
        let out = set_title(html, "Fresh & new");
        assert_eq!(title(&out).as_deref(), Some("Fresh & new"));
        assert!(out.find("<title>").unwrap() > out.find("<head>").unwrap());
        assert!(out.find("<title>").unwrap() < out.find("</head>").unwrap());
        // No <head> at all: the title goes first.
        assert!(set_title("<p>x</p>", "T").starts_with("\n  <title>T</title>"));
    }

    #[test]
    fn set_title_round_trips_special_characters() {
        for name in [
            "Q3 & Q4",
            "a < b > c",
            "  trimmed?  ",
            "émoji 🎉",
            "&amp; literal",
        ] {
            let out = set_title(DECK, name);
            assert_eq!(title(&out).as_deref(), Some(name.trim()), "{name}");
            assert_eq!(ids(&out), ids(DECK), "slides untouched for {name}");
        }
    }

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(
            slugify("  --Leading and trailing--  "),
            "leading-and-trailing"
        );
        assert_eq!(slugify("Q3   2024 / Plan"), "q3-2024-plan");
        assert_eq!(slugify("ÜBER café"), "ber-caf");
        assert_eq!(slugify("日本語"), "");
        assert_eq!(slugify(""), "");
        let long = slugify(&"word ".repeat(30));
        assert!(long.len() <= 48, "{long}");
        assert!(!long.ends_with('-'), "{long}");
    }

    #[test]
    fn unique_id_appends_the_first_free_number() {
        let taken: HashSet<String> = ["slide", "slide-2", "slide-4"].map(String::from).into();
        assert_eq!(unique_id(&taken, "slide"), "slide-3");
        assert_eq!(unique_id(&taken, "other"), "other");
        assert_eq!(
            unique_id(&taken, ""),
            "slide-3",
            "empty base falls back to 'slide'"
        );
    }

    #[test]
    fn normalize_ids_slugifies_duplicates_and_fills_empty_ids_in_place() {
        let html = r#"<main><section class="slide" id="Big Idea"></section><section class="slide" id="Big Idea"></section><section class="slide" id=""></section></main>"#;
        let out = normalize_ids(html).unwrap();
        assert_eq!(ids(&out), ["Big Idea", "big-idea", "slide-3"]);
        assert!(
            !out.contains(r#"id="""#),
            "empty id replaced, not duplicated"
        );
        assert_eq!(out.matches(" id=").count(), 3);
    }

    #[test]
    fn normalize_ids_never_collides_with_later_ids() {
        // Slide 1 has no id; "slide-1" is not taken yet when it is assigned, but slide 2
        // already claims it, so the second gets a suffix.
        let html =
            r#"<section class="slide"></section><section class="slide" id="slide-1"></section>"#;
        let out = normalize_ids(html).unwrap();
        let found = ids(&out);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0], found[1]);
    }

    #[test]
    fn normalize_ids_leaves_the_rest_of_the_document_alone() {
        let html = "<html>\n<section class=\"slide\">A</section>\n<p>tail</p></html>";
        let out = normalize_ids(html).unwrap();
        assert_eq!(
            out,
            "<html>\n<section id=\"slide-1\" class=\"slide\">A</section>\n<p>tail</p></html>"
        );
    }

    #[test]
    fn reorder_rejects_unknown_or_missing_ids() {
        let deck = normalize_ids(DECK).unwrap();
        let order = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(reorder(&deck, &order(&["intro", "plan", "nope"])).is_err());
        assert!(reorder(&deck, &order(&["intro", "plan", "slide-3", "extra"])).is_err());
        assert!(reorder(&deck, &order(&["intro", "intro", "plan"])).is_err());
        // A slide without an id cannot be addressed, so reordering is refused.
        assert!(reorder(DECK, &order(&["intro", "plan"])).is_err());
    }

    #[test]
    fn reorder_keeps_whitespace_and_surrounding_markup() {
        let deck = normalize_ids(DECK).unwrap();
        let same = reorder(&deck, &["intro".into(), "plan".into(), "slide-3".into()]).unwrap();
        assert_eq!(same, deck);
        let swapped = reorder(&deck, &["plan".into(), "intro".into(), "slide-3".into()]).unwrap();
        assert_eq!(swapped.len(), deck.len());
        assert_eq!(
            shell_hash(&swapped, &find_slides(&swapped), &find_sections(&swapped)),
            shell_hash(&deck, &find_slides(&deck), &find_sections(&deck))
        );
    }

    #[test]
    fn delete_removes_the_slide_and_its_leading_whitespace() {
        let html = "<main>\n  <section class=\"slide\" id=\"a\">A</section>\n  <section class=\"slide\" id=\"b\">B</section>\n</main>";
        assert_eq!(
            delete(html, "b").unwrap(),
            "<main>\n  <section class=\"slide\" id=\"a\">A</section>\n</main>"
        );
        assert_eq!(
            delete(html, "a").unwrap(),
            "<main>\n  <section class=\"slide\" id=\"b\">B</section>\n</main>"
        );
        let err = delete(html, "zzz").unwrap_err();
        assert!(err.contains("zzz"), "{err}");
    }

    #[test]
    fn duplicate_copies_markup_with_a_fresh_unique_id() {
        let deck = normalize_ids(DECK).unwrap();
        let (once, first) = duplicate(&deck, "plan").unwrap();
        let (twice, second) = duplicate(&once, "plan").unwrap();
        assert_eq!(first, "plan-copy");
        assert_eq!(second, "plan-copy-2");
        assert_eq!(
            ids(&twice),
            ["intro", "plan", "plan-copy-2", "plan-copy", "slide-3"]
        );
        let copy = find_slides(&twice)
            .into_iter()
            .find(|s| s.id.as_deref() == Some("plan-copy-2"))
            .unwrap();
        assert_eq!(
            &twice[copy.range],
            "<section class='slide' id=plan-copy-2><p>Plan</p></section>"
        );
        assert!(duplicate(&deck, "missing").is_err());
    }

    #[test]
    fn duplicate_keeps_nested_sections() {
        let deck = normalize_ids(DECK).unwrap();
        let (out, id) = duplicate(&deck, "intro").unwrap();
        let copy = find_slides(&out)
            .into_iter()
            .find(|s| s.id.as_deref() == Some(id.as_str()))
            .unwrap();
        assert!(out[copy.range].contains(r#"<section class="inner">x</section>"#));
    }

    #[test]
    fn insert_matches_the_indentation_of_the_anchor_slide() {
        let html = "<main>\n      <section class=\"slide\" id=\"a\"></section>\n</main>";
        let (out, _) = insert(
            html,
            Some("a"),
            "<section class=\"slide\" id=\"{{ID}}\"></section>\n",
            "b",
        )
        .unwrap();
        assert_eq!(
            out,
            "<main>\n      <section class=\"slide\" id=\"a\"></section>\n      <section class=\"slide\" id=\"b\"></section>\n</main>"
        );
    }

    #[test]
    fn insert_after_unknown_slide_appends_at_the_end() {
        let deck = normalize_ids(DECK).unwrap();
        let section = r#"<section class="slide" id="{{ID}}"></section>"#;
        let (out, id) = insert(&deck, Some("ghost"), section, "intro").unwrap();
        assert_eq!(id, "intro-2", "hint collides with an existing id");
        assert_eq!(ids(&out), ["intro", "plan", "slide-3", "intro-2"]);
        let (out, _) = insert(&deck, None, section, "x").unwrap();
        assert_eq!(ids(&out).last().map(String::as_str), Some("x"));
    }

    #[test]
    fn insert_into_empty_deck_falls_back_to_body_or_fails() {
        let section = r#"<section class="slide" id="{{ID}}"></section>"#;
        let (out, id) = insert("<html><BODY>\n</BODY></html>", None, section, "").unwrap();
        assert_eq!(id, "slide");
        assert!(out.find(&format!("id=\"{id}\"")).unwrap() < out.find("</BODY>").unwrap());
        assert!(insert("<p>no container</p>", None, section, "x").is_err());
    }

    #[test]
    fn ensure_runtime_works_without_head_or_body() {
        let out = ensure_runtime(r#"<section class="slide" id="a"></section>"#);
        assert!(out.starts_with("\n  ") && out.contains(CSS_START));
        assert!(out.contains(JS_END));
        assert!(out.find(CSS_END).unwrap() < out.find("<section").unwrap());
        assert!(out.find(JS_START).unwrap() > out.find("</section>").unwrap());
        assert_eq!(
            ids(&out),
            ["a"],
            "runtime script is not mistaken for slides"
        );
    }

    #[test]
    fn ensure_runtime_replaces_a_stale_or_edited_runtime() {
        let stale = DECK
            .replace(
                "</head>",
                &format!("{CSS_START}<style>old</style>{CSS_END}</head>"),
            )
            .replace(
                "</body>",
                &format!("{JS_START}<script>tampered()</script>{JS_END}</body>"),
            );
        let out = ensure_runtime(&stale);
        assert!(!out.contains("old</style>"));
        assert!(!out.contains("tampered()"));
        assert_eq!(out.matches(JS_START).count(), 1);
        assert!(out.contains(RUNTIME_JS));
        // Refreshed in place: the block stays after the deck's own <style>.
        assert!(out.find(CSS_START).unwrap() > out.find(".slide { color: red }").unwrap());
        assert_eq!(out, ensure_runtime(&out));
    }

    #[test]
    fn ensure_runtime_flags_the_player_inside_the_css_block() {
        let out = ensure_runtime(DECK);
        let flag = out.find(PLAYER_FLAG).expect("player flag installed");
        assert!(out.find(CSS_START).unwrap() < flag && flag < out.find(CSS_END).unwrap());
        assert!(
            flag < out.find("<body").unwrap(),
            "runs before the body renders"
        );
        assert_eq!(out.matches(PLAYER_FLAG).count(), 1);
        // Decks saved before the flag existed get it on refresh.
        let old = DECK.replace(
            "</head>",
            &format!("{CSS_START}<style>{RUNTIME_CSS}</style>{CSS_END}</head>"),
        );
        assert_eq!(ensure_runtime(&old).matches(PLAYER_FLAG).count(), 1);
    }

    #[test]
    fn ensure_runtime_embeds_the_current_assets() {
        let out = ensure_runtime(DECK);
        assert!(out.contains(RUNTIME_CSS));
        assert!(out.contains(RUNTIME_JS));
    }

    #[test]
    fn inlines_cropped_css_backgrounds() {
        // Crops are CSS backgrounds with offsets; the same image can back several crops.
        let html = r#"<style>
#title .hero { background: url("assets/photo.png") 0 -1459px / 1920px auto no-repeat; }
#closing .photo { background-image: url("assets/photo.png"); background-position: -619px -1032px; }
</style>"#;
        let out = inline_assets(html, |p| {
            (p == "assets/photo.png").then(|| ("image/png".into(), b"ok".to_vec()))
        });
        assert_eq!(
            out.matches(r#"url("data:image/png;base64,b2s=")"#).count(),
            2,
            "{out}"
        );
        assert!(out.contains(") 0 -1459px / 1920px auto no-repeat;"));
        assert!(out.contains("background-position: -619px -1032px;"));
        assert!(!out.contains("assets/"));
    }

    #[test]
    fn inline_assets_stops_at_query_and_fragment() {
        let html = r##"<img src="assets/a.png?v=2"><use href="assets/icons.svg#star"/>"##;
        let mut seen = Vec::new();
        let out = inline_assets(html, |p| {
            seen.push(p.to_string());
            Some(("x/y".into(), vec![1, 2, 3]))
        });
        assert_eq!(seen, ["assets/a.png", "assets/icons.svg"]);
        assert_eq!(
            out,
            r##"<img src="data:x/y;base64,AQID?v=2"><use href="data:x/y;base64,AQID#star"/>"##
        );
    }

    #[test]
    fn inline_assets_leaves_unloadable_and_unquoted_references() {
        let html =
            r#"<img src="assets/missing.png"><a href=assets/x.png>x</a><img src='assets/ok.png'>"#;
        let out = inline_assets(html, |p| {
            (p == "assets/ok.png").then(|| ("image/png".into(), b"ok".to_vec()))
        });
        assert_eq!(
            out,
            r#"<img src="assets/missing.png"><a href=assets/x.png>x</a><img src='data:image/png;base64,b2s='>"#
        );
    }

    #[test]
    fn inline_assets_ignores_lookalike_paths() {
        let html =
            r#"<img src="myassets/a.png"><img src="/assets/a.png"><img src="../assets/a.png">"#;
        let out = inline_assets(html, |_| Some(("image/png".into(), vec![0])));
        assert_eq!(
            out, html,
            "only deck-relative assets/ references are inlined"
        );
    }

    const SECTIONED: &str = r#"<main class="deck">
    <section class="slide" id="a">A</section>
    <div class="deck-section" data-title="Intro &amp; &quot;goals&quot;"></div>
    <section class="slide" id="b">B<div class="deck-section" data-title="inside"></div></section>
    <section class="slide" id="c">C</section>
    <!-- <div class="deck-section" data-title="ghost"></div> -->
    <div class="deck-section" data-title="Wrap up" ></div>
    <section class="slide" id="d">D</section>
  </main>"#;

    fn section_titles(html: &str) -> Vec<(String, usize)> {
        find_sections(html)
            .into_iter()
            .map(|s| (s.title, s.before))
            .collect()
    }

    #[test]
    fn finds_section_markers_between_slides_only() {
        assert_eq!(
            section_titles(SECTIONED),
            [
                ("Intro & \"goals\"".to_string(), 1),
                ("Wrap up".to_string(), 3)
            ]
        );
        assert_eq!(ids(SECTIONED), ["a", "b", "c", "d"]);
        assert!(find_sections("<main class=\"deck\"></main>").is_empty());
    }

    #[test]
    fn section_markers_are_not_slides_and_cover_their_end_tag() {
        let sections = find_sections(SECTIONED);
        assert!(SECTIONED[sections[0].range.clone()].ends_with("></div>"));
        assert!(sections.iter().all(|s| s.empty));
        let open = "<div class=\"deck-section\" data-title=\"x\"><p>hi</p></div>";
        let found = find_sections(open);
        assert!(!found[0].empty);
        assert_eq!(
            &open[found[0].range.clone()],
            &open[..open.find('>').unwrap() + 1]
        );
        let upper = "<DIV CLASS=\"deck-section\" DATA-TITLE=\"Up\"> </DIV>";
        let found = find_sections(upper);
        assert_eq!(found[0].title, "Up");
        assert_eq!(found[0].range, 0..upper.len());
        assert!(found[0].empty);
    }

    #[test]
    fn section_without_a_title_has_an_empty_one() {
        let found = find_sections("<div class=\"deck-section\"></div>");
        assert_eq!(found[0].title, "");
        assert!(found[0].title_value.is_none());
        let found =
            find_sections("<div class=\"other deck-section\" data-title=\"  Padded  \"></div>");
        assert_eq!(found[0].title, "Padded");
    }

    #[test]
    fn section_markers_in_scripts_and_styles_are_ignored() {
        let html = "<script>x = '<div class=\"deck-section\" data-title=\"js\"></div>'</script>\
            <div class=\"deck-section\" data-title=\"real\"></div>";
        assert_eq!(section_titles(html), [("real".to_string(), 0)]);
    }

    #[test]
    fn adds_a_section_before_a_slide_with_matching_indent() {
        let out = add_section(THREE_SLIDES, Some("b"), "Part \"2\" & more").unwrap();
        assert_eq!(
            out,
            THREE_SLIDES.replace(
                "  <section class=\"slide\" id=\"b\">",
                "  <div class=\"deck-section\" data-title=\"Part &quot;2&quot; &amp; more\"></div>\n  <section class=\"slide\" id=\"b\">"
            )
        );
        assert_eq!(section_titles(&out), [("Part \"2\" & more".to_string(), 1)]);
        assert_eq!(ids(&out), ["a", "b", "c"]);
    }

    #[test]
    fn adds_a_section_at_the_end_or_into_an_empty_deck() {
        let out = add_section(THREE_SLIDES, None, "End").unwrap();
        assert_eq!(section_titles(&out), [("End".to_string(), 3)]);
        assert!(out.ends_with("</div>\n</main></body></html>"));
        let empty = "<main class=\"deck\">\n</main>";
        let out = add_section(empty, None, "Only").unwrap();
        assert_eq!(section_titles(&out), [("Only".to_string(), 0)]);
        assert!(add_section("<p>x</p>", None, "x").is_err());
        assert!(add_section(THREE_SLIDES, Some("nope"), "x").is_err());
    }

    #[test]
    fn renames_a_section_and_keeps_other_attributes() {
        let out = rename_section(SECTIONED, 1, "Closing <b>").unwrap();
        assert_eq!(section_titles(&out)[1], ("Closing <b>".to_string(), 3),);
        assert!(out.contains("data-title=\"Closing &lt;b&gt;\" ></div>"));
        assert_eq!(section_titles(&out)[0].0, "Intro & \"goals\"");
        let bare = "<div class=\"deck-section\" id=\"s\"></div>";
        let out = rename_section(bare, 0, "New").unwrap();
        assert_eq!(
            out,
            "<div data-title=\"New\" class=\"deck-section\" id=\"s\"></div>"
        );
        assert!(rename_section(SECTIONED, 2, "x").is_err());
    }

    #[test]
    fn deletes_only_the_marker() {
        let out = delete_section(SECTIONED, 0).unwrap();
        assert_eq!(section_titles(&out), [("Wrap up".to_string(), 3)]);
        assert_eq!(ids(&out), ["a", "b", "c", "d"]);
        assert!(!out.contains("Intro"));
        assert!(out.contains("<section class=\"slide\" id=\"b\">"));
        assert!(delete_section(SECTIONED, 5).is_err());
    }

    #[test]
    fn reorder_moves_markers_together_with_slides() {
        let html = "<main>\n<section class=\"slide\" id=\"a\">A</section>\n<div class=\"deck-section\" data-title=\"One\"></div>\n<section class=\"slide\" id=\"b\">B</section>\n<section class=\"slide\" id=\"c\">C</section>\n</main>";
        let order = |keys: &[&str]| keys.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        let out = reorder(html, &order(&["a", "b", "section:0", "c"])).unwrap();
        assert_eq!(section_titles(&out), [("One".to_string(), 2)]);
        assert_eq!(ids(&out), ["a", "b", "c"]);
        let out = reorder(html, &order(&["section:0", "c", "b", "a"])).unwrap();
        assert_eq!(section_titles(&out), [("One".to_string(), 0)]);
        assert_eq!(ids(&out), ["c", "b", "a"]);
        assert!(out.starts_with("<main>\n<div class=\"deck-section\""));
        assert!(out.ends_with("</section>\n</main>"));
    }

    #[test]
    fn reorder_requires_every_marker() {
        let err = reorder(SECTIONED, &["a".into(), "b".into(), "c".into(), "d".into()]);
        assert!(err.is_err(), "markers missing from the order");
        let all: Vec<String> = ["a", "section:0", "b", "c", "section:1", "d"]
            .map(String::from)
            .into();
        assert_eq!(reorder(SECTIONED, &all).unwrap(), SECTIONED);
    }

    #[test]
    fn shell_hash_ignores_review_marks() {
        let hash = |html: &str| shell_hash(html, &find_slides(html), &find_sections(html));
        let deck = ensure_runtime(
            "<html><head></head><body><main class=\"deck\"><section class=\"slide\" id=\"a\"></section></main></body></html>",
        );
        let stroke = crate::review::Stroke {
            tool: crate::review::InkTool::Pen,
            color: "#000".into(),
            points: vec![[0.5, 0.5]],
        };
        let marked = crate::review::write(&deck, &[("a".to_string(), vec![stroke])].into());
        assert_ne!(marked, deck);
        assert_eq!(hash(&marked), hash(&deck));
    }

    #[test]
    fn shell_hash_ignores_section_markers() {
        let hash = |html: &str| shell_hash(html, &find_slides(html), &find_sections(html));
        let base = hash(SECTIONED);
        assert_eq!(base, hash(&rename_section(SECTIONED, 0, "Other").unwrap()));
        assert_eq!(base, hash(&delete_section(SECTIONED, 1).unwrap()));
        assert_eq!(
            base,
            hash(&add_section(SECTIONED, Some("d"), "More").unwrap())
        );
        assert_ne!(
            base,
            hash(&SECTIONED.replace("<main", "<header></header><main"))
        );
    }

    #[test]
    fn inserting_and_duplicating_slides_keeps_markers() {
        let (out, new) = duplicate(SECTIONED, "a").unwrap();
        assert_eq!(new, "a-copy");
        assert_eq!(
            section_titles(&out)
                .iter()
                .map(|(_, before)| *before)
                .collect::<Vec<_>>(),
            [2, 4]
        );
        let (out, _) = insert(
            SECTIONED,
            Some("c"),
            "<section class=\"slide\" id=\"{{ID}}\"></section>",
            "new",
        )
        .unwrap();
        assert_eq!(section_titles(&out)[1].1, 4);
    }

    const THREE_SLIDES: &str = "<html><body><main class=\"deck\">\n  <section class=\"slide\" id=\"a\">A</section>\n  <section class=\"slide\" id=\"b\">B</section>\n  <section class=\"slide\" id=\"c\">C</section>\n</main></body></html>";

    const STYLED: &str = "<!DOCTYPE html><html><head>\n  <title>Talk</title>\n  <style>.x {}</style>\n</head><body>\n  <main class=\"deck\">\n    <section class=\"slide\" id=\"a\"><h1>Big &amp; bold</h1><p>Revenue grew 42% in Q3.</p></section>\n    <div class=\"deck-section\" data-title=\"Part\"></div>\n    <section class=\"slide\" id=\"b\"><style>.y { content: \"keep\" }</style><!-- note --><p>NASA said: hello</p></section>\n  </main>\n</body></html>";

    #[test]
    fn reads_and_writes_the_template_meta() {
        assert_eq!(template(STYLED), None);
        let named = set_template(STYLED, Some("swiss"));
        assert!(
            named.contains(
                "<title>Talk</title>\n  <meta name=\"slopslide-template\" content=\"swiss\">"
            ),
            "{named}"
        );
        assert_eq!(template(&named).as_deref(), Some("swiss"));
        let renamed = set_template(&named, Some("a\"b"));
        assert_eq!(renamed.matches(TEMPLATE_META).count(), 1);
        assert_eq!(template(&renamed).as_deref(), Some("a\"b"));
        assert_eq!(set_template(&named, None), STYLED, "removing undoes adding");
        assert_eq!(set_template(STYLED, None), STYLED);
        assert_eq!(
            template("<head><META NAME=\"SlopSlide-Template\" content=\" bento-grid \"></head>")
                .as_deref(),
            Some("bento-grid")
        );
        assert_eq!(
            template("<meta name=\"slopslide-template\" content=\"\">"),
            None
        );
        let untitled = set_template("<html><head></head></html>", Some("x"));
        assert_eq!(
            untitled,
            "<html><head>\n  <meta name=\"slopslide-template\" content=\"x\"></head></html>"
        );
    }

    #[test]
    fn strips_slides_and_sections_but_keeps_the_shell() {
        let shell = strip_slides(STYLED);
        assert!(find_slides(&shell).is_empty());
        assert!(find_sections(&shell).is_empty());
        assert!(shell.contains("<style>.x {}</style>"));
        assert!(
            shell.contains("<main class=\"deck\">\n  </main>"),
            "{shell}"
        );
    }

    #[test]
    fn removes_the_runtime_blocks() {
        let with = ensure_runtime(STYLED);
        assert!(with.contains(CSS_START) && with.contains(JS_START));
        let without = without_runtime(&with);
        assert!(!without.contains("slopslide:runtime"));
        assert_eq!(without_runtime(STYLED), STYLED);
        assert_eq!(find_slides(&without).len(), 2);
    }

    #[test]
    fn copies_a_slide_from_another_document() {
        let deck = "<main class=\"deck\">\n    <section class=\"slide\" id=\"a\">A</section>\n    <section class=\"slide\" id=\"z\">Z</section>\n</main>";
        let (out, id) = copy_slide(deck, Some("a"), STYLED, "b").unwrap();
        assert_eq!(id, "b", "keeps the template's id when free");
        let ids: Vec<_> = find_slides(&out).into_iter().filter_map(|s| s.id).collect();
        assert_eq!(ids, ["a", "b", "z"]);
        assert!(out.contains("NASA said"));
        let (again, second) = copy_slide(&out, None, STYLED, "b").unwrap();
        assert_eq!(second, "b-2");
        assert_eq!(
            find_slides(&again).last().unwrap().id.as_deref(),
            Some("b-2")
        );
        assert!(copy_slide(deck, None, STYLED, "missing").is_err());
    }

    #[test]
    fn replaces_slide_text_with_placeholder_words() {
        let out = with_placeholder_text(STYLED);
        // Everything outside the slides, and all markup, styles and comments stay.
        assert!(out.starts_with("<!DOCTYPE html><html><head>\n  <title>Talk</title>"));
        assert!(out.contains("data-title=\"Part\""));
        assert!(out.contains("<style>.y { content: \"keep\" }</style><!-- note -->"));
        assert!(out.contains("<section class=\"slide\" id=\"a\"><h1>"));
        assert!(out.contains(" &amp; "), "entities stay: {out}");
        for word in ["Big", "bold", "Revenue", "grew", "NASA", "hello", "42"] {
            assert!(!out.contains(word), "{word} left in {out}");
        }
        assert_eq!(find_slides(&out).len(), 2);
        // Case and punctuation follow the original.
        let slides = find_slides(&out);
        let b = &out[slides[1].range.clone()];
        let text = b.split("<p>").nth(1).unwrap().split("</p>").next().unwrap();
        let words: Vec<&str> = text.split(' ').collect();
        assert_eq!(words.len(), 3, "{text}");
        assert!(
            words[0].len() > 1 && words[0].chars().all(char::is_uppercase),
            "{text}"
        );
        assert!(words[1].ends_with(':'), "{text}");
        assert!(words[2].chars().all(char::is_lowercase), "{text}");
        let a = &out[slides[0].range.clone()];
        assert!(a.contains("% "), "digits and symbols keep their shape: {a}");
        assert_eq!(
            with_placeholder_text(&out).len(),
            out.len(),
            "same shape again"
        );
    }
}
