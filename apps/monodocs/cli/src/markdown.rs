//! Markdown -> HTML for a single document, with the bookkeeping the single-file output needs:
//! namespaced heading ids, a table of contents, highlighted fences, cross-document links
//! rewritten to in-page anchors, and local images inlined as data URIs.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};

use crate::{
    discover::slugify,
    highlight::{highlight, label_for},
};

/// One heading in the navigation tree.
pub struct TocEntry {
    pub level: u8,
    pub id: String,
    pub text: String,
}

pub struct Rendered {
    pub html: String,
    pub toc: Vec<TocEntry>,
}

/// Everything a document needs to resolve ids and links.
pub struct DocContext<'a> {
    /// Directory containing the markdown file (relative links resolve against it).
    pub doc_dir: &'a Path,
    /// Directory the HTML file is written to (out-of-document links resolve against it).
    pub out_dir: &'a Path,
    /// Id namespace, normally the project slug.
    pub prefix: &'a str,
    /// Id forced onto the document's first `#` heading, so nav links target the section.
    pub section_id: Option<&'a str>,
    /// Canonical markdown path -> in-page anchor, used to turn `../other/README.md` into `#id`.
    pub anchors: &'a HashMap<PathBuf, String>,
}

/// Hands out document-unique element ids.
#[derive(Default)]
pub struct IdAllocator {
    used: HashSet<String>,
}

impl IdAllocator {
    pub fn alloc(&mut self, candidate: &str) -> String {
        let base = if candidate.is_empty() {
            "section".to_string()
        } else {
            candidate.to_string()
        };
        if self.used.insert(base.clone()) {
            return base;
        }
        for n in 2.. {
            let candidate = format!("{base}-{n}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
        }
        unreachable!("integer overflow before a free id")
    }
}

/// Parser extensions, shared with [`crate::lint`] so both read the same document.
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
}

/// Render `text`, demoting every heading one level so the page `<h1>` stays unique.
pub fn render(text: &str, ctx: &DocContext<'_>, ids: &mut IdAllocator) -> Rendered {
    let mut events: Vec<Event<'static>> = Vec::new();
    let mut toc = Vec::new();
    let mut parser = Parser::new_ext(text, options());
    let mut first_heading = true;

    while let Some(event) = parser.next() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::Text(text) => code.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                events.push(Event::Html(code_block(&lang, &code).into()));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let mut inner: Vec<Event<'static>> = Vec::new();
                let mut plain = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::Heading(_)) => break,
                        Event::Text(ref text) => {
                            plain.push_str(text);
                            inner.push(into_static(event));
                        }
                        Event::Code(ref text) => {
                            plain.push_str(text);
                            inner.push(into_static(event));
                        }
                        other => inner.push(into_static(other)),
                    }
                }
                let raw = level as u8;
                let id = match (first_heading, raw, ctx.section_id) {
                    (true, 1, Some(section)) => section.to_string(),
                    _ => ids.alloc(&format!("{}--{}", ctx.prefix, slugify(&plain))),
                };
                if raw == 1 {
                    first_heading = false;
                }
                if raw <= 3 {
                    toc.push(TocEntry {
                        level: raw,
                        id: id.clone(),
                        text: plain,
                    });
                }
                let shown = (raw + 1).min(6);
                events.push(Event::Html(
                    format!("<h{shown} id=\"{id}\" class=\"doc-h\">").into(),
                ));
                events.extend(inner);
                events.push(Event::Html(
                    format!(
                        "<a class=\"hash\" href=\"#{id}\" aria-label=\"Permalink\">#</a></h{shown}>"
                    )
                    .into(),
                ));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = resolve_link(&dest_url, ctx);
                events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: dest.into(),
                    title: title.into_string().into(),
                    id: id.into_string().into(),
                }));
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = inline_image(&dest_url, ctx);
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: dest.into(),
                    title: title.into_string().into(),
                    id: id.into_string().into(),
                }));
            }
            other => events.push(into_static(other)),
        }
    }

    let mut out = String::with_capacity(text.len() * 2);
    html::push_html(&mut out, events.into_iter());
    Rendered { html: out, toc }
}

/// `pulldown_cmark` borrows from the source string; the transformed stream outlives it.
fn into_static(event: Event<'_>) -> Event<'static> {
    match event {
        Event::Text(text) => Event::Text(text.into_string().into()),
        Event::Code(text) => Event::Code(text.into_string().into()),
        Event::Html(text) => Event::Html(text.into_string().into()),
        Event::InlineHtml(text) => Event::InlineHtml(text.into_string().into()),
        Event::FootnoteReference(text) => Event::FootnoteReference(text.into_string().into()),
        Event::Start(tag) => Event::Start(static_tag(tag)),
        Event::End(tag) => Event::End(tag),
        Event::SoftBreak => Event::SoftBreak,
        Event::HardBreak => Event::HardBreak,
        Event::Rule => Event::Rule,
        Event::TaskListMarker(done) => Event::TaskListMarker(done),
        Event::InlineMath(text) => Event::InlineMath(text.into_string().into()),
        Event::DisplayMath(text) => Event::DisplayMath(text.into_string().into()),
    }
}

fn static_tag(tag: Tag<'_>) -> Tag<'static> {
    match tag {
        Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        } => Tag::Link {
            link_type,
            dest_url: dest_url.into_string().into(),
            title: title.into_string().into(),
            id: id.into_string().into(),
        },
        Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        } => Tag::Image {
            link_type,
            dest_url: dest_url.into_string().into(),
            title: title.into_string().into(),
            id: id.into_string().into(),
        },
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
            Tag::CodeBlock(CodeBlockKind::Fenced(info.into_string().into()))
        }
        Tag::CodeBlock(CodeBlockKind::Indented) => Tag::CodeBlock(CodeBlockKind::Indented),
        Tag::FootnoteDefinition(text) => Tag::FootnoteDefinition(text.into_string().into()),
        Tag::Heading {
            level,
            id,
            classes,
            attrs,
        } => Tag::Heading {
            level,
            id: id.map(|id| id.into_string().into()),
            classes: classes
                .into_iter()
                .map(|class| class.into_string().into())
                .collect(),
            attrs: attrs
                .into_iter()
                .map(|(key, value)| {
                    (
                        key.into_string().into(),
                        value.map(|value| value.into_string().into()),
                    )
                })
                .collect(),
        },
        Tag::Paragraph => Tag::Paragraph,
        Tag::BlockQuote(kind) => Tag::BlockQuote(kind),
        Tag::HtmlBlock => Tag::HtmlBlock,
        Tag::List(start) => Tag::List(start),
        Tag::Item => Tag::Item,
        Tag::Table(alignments) => Tag::Table(alignments),
        Tag::TableHead => Tag::TableHead,
        Tag::TableRow => Tag::TableRow,
        Tag::TableCell => Tag::TableCell,
        Tag::Emphasis => Tag::Emphasis,
        Tag::Strong => Tag::Strong,
        Tag::Strikethrough => Tag::Strikethrough,
        Tag::MetadataBlock(kind) => Tag::MetadataBlock(kind),
        Tag::DefinitionList => Tag::DefinitionList,
        Tag::DefinitionListTitle => Tag::DefinitionListTitle,
        Tag::DefinitionListDefinition => Tag::DefinitionListDefinition,
        Tag::Superscript => Tag::Superscript,
        Tag::Subscript => Tag::Subscript,
    }
}

/// A fenced block: language chip, copy button, highlighted body.
fn code_block(lang: &str, code: &str) -> String {
    let label = label_for(lang);
    let body = highlight(lang, code);
    let chip = match label {
        Some(label) => format!("<span class=\"lang\">{label}</span>"),
        None => String::new(),
    };
    let class = label.unwrap_or("text");
    format!(
        "<figure class=\"code\" data-lang=\"{class}\">\
<figcaption>{chip}<button class=\"copy\" type=\"button\">copy</button></figcaption>\
<pre><code>{body}</code></pre></figure>"
    )
}

/// Turn a markdown link into something that works inside the single-file document.
fn resolve_link(dest: &str, ctx: &DocContext<'_>) -> String {
    if dest.is_empty() {
        return String::new();
    }
    if let Some(fragment) = dest.strip_prefix('#') {
        return format!("#{}--{}", ctx.prefix, slugify(fragment));
    }
    if has_scheme(dest) {
        return dest.to_string();
    }

    let (path_part, fragment) = match dest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (dest, None),
    };
    let target = normalize(&ctx.doc_dir.join(path_part));

    if let Some(anchor) = ctx.anchors.get(&target) {
        return format!("#{anchor}");
    }
    match relative_to(ctx.out_dir, &target) {
        Some(path) => match fragment {
            Some(fragment) => format!("{path}#{fragment}"),
            None => path,
        },
        None => dest.to_string(),
    }
}

/// Inline small local images so the document stays a single portable file.
fn inline_image(dest: &str, ctx: &DocContext<'_>) -> String {
    const MAX_INLINE: u64 = 2 * 1024 * 1024;

    if has_scheme(dest) || dest.starts_with("data:") {
        return dest.to_string();
    }
    let path = normalize(&ctx.doc_dir.join(dest));
    let Ok(metadata) = fs::metadata(&path) else {
        return dest.to_string();
    };
    if metadata.len() > MAX_INLINE {
        return relative_to(ctx.out_dir, &path).unwrap_or_else(|| dest.to_string());
    }
    let Ok(bytes) = fs::read(&path) else {
        return dest.to_string();
    };
    let mime = match path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => return relative_to(ctx.out_dir, &path).unwrap_or_else(|| dest.to_string()),
    };
    format!("data:{mime};base64,{}", base64(&bytes))
}

/// True for `https:`, `mailto:` and friends — anything that is not a repo-relative path.
pub fn has_scheme(value: &str) -> bool {
    matches!(
        value.split_once(':'),
        Some((scheme, _)) if scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    )
}

/// Lexical path normalisation: no filesystem access, so missing targets still resolve.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `target` expressed relative to `from`, using `..` as needed.
fn relative_to(from: &Path, target: &Path) -> Option<String> {
    let from = normalize(from);
    let target = normalize(target);
    let mut from_parts = from.components().peekable();
    let mut target_parts = target.components().peekable();
    while from_parts.peek().is_some() && from_parts.peek() == target_parts.peek() {
        from_parts.next();
        target_parts.next();
    }
    let ups = from_parts.count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), ups).collect();
    parts
        .extend(target_parts.map(|component| component.as_os_str().to_string_lossy().into_owned()));
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Minimal base64 encoder; avoids a dependency for the rare inlined image.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (triple >> (18 - 6 * i)) & 0b111111;
                out.push(ALPHABET[index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(anchors: &'a HashMap<PathBuf, String>) -> DocContext<'a> {
        DocContext {
            doc_dir: Path::new("/repo/libs/pg-gen"),
            out_dir: Path::new("/repo/dist/docs"),
            prefix: "pg-gen",
            section_id: Some("proj-pg-gen"),
            anchors,
        }
    }

    #[test]
    fn headings_are_demoted_namespaced_and_collected() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render("# pg-gen\n\n## Usage\n", &ctx(&anchors), &mut ids);
        assert!(out.html.contains("<h2 id=\"proj-pg-gen\""));
        assert!(out.html.contains("<h3 id=\"pg-gen--usage\""));
        let toc: Vec<_> = out.toc.iter().map(|e| (e.level, e.id.as_str())).collect();
        assert_eq!(toc, vec![(1, "proj-pg-gen"), (2, "pg-gen--usage")]);
    }

    #[test]
    fn duplicate_headings_get_unique_ids() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render("## Usage\n\n## Usage\n", &ctx(&anchors), &mut ids);
        assert!(out.html.contains("id=\"pg-gen--usage\""));
        assert!(out.html.contains("id=\"pg-gen--usage-2\""));
    }

    #[test]
    fn cross_project_readme_links_become_anchors() {
        let mut anchors = HashMap::new();
        anchors.insert(
            PathBuf::from("/repo/apps/clis/pg-cli/README.md"),
            "proj-pg-cli".to_string(),
        );
        let mut ids = IdAllocator::default();
        let out = render(
            "See [pg-cli](../../apps/clis/pg-cli/README.md).",
            &ctx(&anchors),
            &mut ids,
        );
        assert!(out.html.contains("href=\"#proj-pg-cli\""), "{}", out.html);
    }

    #[test]
    fn unknown_relative_links_point_back_at_the_repo_file() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render("[src](src/lib.rs)", &ctx(&anchors), &mut ids);
        assert!(
            out.html.contains("href=\"../../libs/pg-gen/src/lib.rs\""),
            "{}",
            out.html
        );
    }

    #[test]
    fn external_and_fragment_links_survive() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render(
            "[ext](https://example.com/a#b) [local](#usage)",
            &ctx(&anchors),
            &mut ids,
        );
        assert!(out.html.contains("href=\"https://example.com/a#b\""));
        assert!(out.html.contains("href=\"#pg-gen--usage\""));
    }

    #[test]
    fn fences_are_highlighted_and_labelled() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render("```nu\nlet x = 1\n```\n", &ctx(&anchors), &mut ids);
        assert!(out.html.contains("data-lang=\"nushell\""));
        assert!(out.html.contains("<span class=\"tok-kw\">let</span>"));
    }

    #[test]
    fn tables_render() {
        let anchors = HashMap::new();
        let mut ids = IdAllocator::default();
        let out = render(
            "| a | b |\n|---|---|\n| 1 | 2 |\n",
            &ctx(&anchors),
            &mut ids,
        );
        assert!(out.html.contains("<table>"));
        assert!(out.html.contains("<td>1</td>"));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
