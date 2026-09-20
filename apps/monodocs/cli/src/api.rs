//! The API surface of a Rust crate, read back out of the rustdoc tree `--cargo-doc` produced.
//!
//! rustdoc already indexes every module for its own sidebar — `sidebar-items.js` is a JSON object
//! of item kind to item names — and puts a one-line summary for each item in the module's
//! `index.html`. Reading those two files is what lets the document list modules, structs, enums
//! and functions inline instead of only linking to them, and it keeps rustdoc the single source
//! of truth: there is no second Rust parser here to drift from the compiler, and private items
//! show up for binaries exactly as rustdoc decided they should.
//!
//! Item pages are named after their kind (`struct.Cli.html`, `fn.main.html`) and the sidebar key
//! *is* that prefix, so one rule builds every link; modules are the single exception
//! (`cache/index.html`).

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

use serde_json::Value;

/// Item kinds in the order rustdoc lists them, with the heading each one gets here. A kind added
/// to a later rustdoc is still listed — under its own key — because unknown keys are never
/// dropped.
const KINDS: &[(&str, &str)] = &[
    ("primitive", "Primitives"),
    ("macro", "Macros"),
    ("derive", "Derive macros"),
    ("attr", "Attribute macros"),
    ("struct", "Structs"),
    ("union", "Unions"),
    ("enum", "Enums"),
    ("constant", "Constants"),
    ("static", "Statics"),
    ("trait", "Traits"),
    ("traitalias", "Trait aliases"),
    ("fn", "Functions"),
    ("type", "Type aliases"),
    ("foreigntype", "Foreign types"),
    ("keyword", "Keywords"),
];

/// Longest a summary may run before it is cut at a word boundary.
const SUMMARY_LIMIT: usize = 200;

/// One module: the items it declares, grouped by kind, and its child modules.
#[derive(Debug)]
pub struct Module {
    pub name: String,
    /// This module's own page, relative to the crate directory.
    pub href: String,
    /// First line of the module's documentation, when rustdoc wrote one.
    pub summary: Option<String>,
    pub groups: Vec<Group>,
    pub modules: Vec<Module>,
}

/// Every item of one kind in one module.
#[derive(Debug)]
pub struct Group {
    /// Heading for the kind (`Structs`).
    pub label: String,
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub struct Item {
    pub name: String,
    /// The item's page, relative to the crate directory.
    pub href: String,
    pub summary: Option<String>,
}

impl Module {
    /// Items declared here and in every descendant, child modules counted as items themselves.
    pub fn count(&self) -> usize {
        self.groups
            .iter()
            .map(|group| group.items.len())
            .sum::<usize>()
            + self.modules.len()
            + self.modules.iter().map(Module::count).sum::<usize>()
    }

    /// `3 modules · 2 structs · 1 function` — this module only, not its descendants.
    pub fn tally(&self) -> String {
        let mut parts = Vec::new();
        if !self.modules.is_empty() {
            parts.push(quantity(self.modules.len(), "Modules"));
        }
        for group in &self.groups {
            parts.push(quantity(group.items.len(), &group.label));
        }
        parts.join(" · ")
    }
}

/// Read the rustdoc tree of one crate (`<out>/api/<crate>`). `None` when rustdoc has not run for
/// it, or when it documents nothing at all.
pub fn outline(crate_dir: &Path, crate_name: &str) -> Option<Module> {
    if !crate_dir.join("index.html").is_file() {
        return None;
    }
    let module = read_module(crate_dir, crate_name, "");
    (module.count() > 0).then_some(module)
}

/// `prefix` is the href of `dir` relative to the crate directory: `""` at the crate root,
/// `"cache/"` one level down.
fn read_module(dir: &Path, name: &str, prefix: &str) -> Module {
    let mut kinds = sidebar_items(dir);
    let summaries = fs::read_to_string(dir.join("index.html"))
        .map(|html| summaries(&html))
        .unwrap_or_default();

    let modules = kinds
        .remove("mod")
        .unwrap_or_default()
        .into_iter()
        .map(|child| {
            let mut module = read_module(&dir.join(&child), &child, &format!("{prefix}{child}/"));
            module.summary = summaries.get(&format!("{child}/index.html")).cloned();
            module
        })
        .collect();

    let mut groups = Vec::new();
    for (kind, label) in KINDS {
        if let Some(names) = kinds.remove(*kind) {
            groups.push(group(
                (*label).to_string(),
                kind,
                &names,
                prefix,
                &summaries,
            ));
        }
    }
    for (kind, names) in kinds {
        let label = capitalize(&kind);
        groups.push(group(label, &kind, &names, prefix, &summaries));
    }

    Module {
        name: name.to_string(),
        href: format!("{prefix}index.html"),
        summary: None,
        groups,
        modules,
    }
}

fn group(
    label: String,
    kind: &str,
    names: &[String],
    prefix: &str,
    summaries: &HashMap<String, String>,
) -> Group {
    let items = names
        .iter()
        .map(|name| {
            let file = format!("{kind}.{name}.html");
            Item {
                summary: summaries.get(&file).cloned(),
                href: format!("{prefix}{file}"),
                name: name.clone(),
            }
        })
        .collect();
    Group { label, items }
}

fn sidebar_items(dir: &Path) -> BTreeMap<String, Vec<String>> {
    fs::read_to_string(dir.join("sidebar-items.js"))
        .map(|text| parse_sidebar(&text))
        .unwrap_or_default()
}

/// `window.SIDEBAR_ITEMS = {"fn":["main"],"macro":[["shout",1]]};` — kind to sorted names.
fn parse_sidebar(text: &str) -> BTreeMap<String, Vec<String>> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return BTreeMap::new();
    };
    if start >= end {
        return BTreeMap::new();
    }
    let raw: BTreeMap<String, Vec<Value>> =
        serde_json::from_str(&text[start..=end]).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(kind, entries)| {
            let mut names: Vec<String> = entries.iter().filter_map(entry_name).collect();
            names.sort();
            names.dedup();
            (!names.is_empty()).then_some((kind, names))
        })
        .collect()
}

/// An entry is the item name, or `["name", flags]` for macros.
fn entry_name(entry: &Value) -> Option<String> {
    match entry {
        Value::String(name) => Some(name.clone()),
        Value::Array(parts) => parts.first()?.as_str().map(str::to_string),
        _ => None,
    }
}

/// `href` -> one-line summary, from the `<dl class="item-table">` blocks rustdoc writes into every
/// module page: `<dt><a href="struct.Cli.html">Cli</a></dt><dd>Parsed command line.</dd>`.
///
/// Tolerant by design. A rustdoc that changes this markup costs the document its one-line
/// descriptions; the item list itself comes from the machine-readable sidebar and is unaffected.
fn summaries(html: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut rest = html;
    while let Some((term, tail)) = element(rest, "<dt", "</dt>") {
        rest = tail;
        let Some(href) = attribute(term, "href") else {
            continue;
        };
        let tail = tail.trim_start();
        if !tail.starts_with("<dd") {
            continue;
        }
        let Some((description, _)) = element(tail, "<dd", "</dd>") else {
            continue;
        };
        let summary = truncate(&text_of(description));
        if !summary.is_empty() {
            map.entry(href).or_insert(summary);
        }
    }
    map
}

/// Inner HTML of the first `<tag …>…</close>`, plus everything after it.
fn element<'a>(html: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str)> {
    let start = html.find(open)?;
    let after = &html[start + open.len()..];
    let inner = &after[after.find('>')? + 1..];
    let end = inner.find(close)?;
    Some((&inner[..end], &inner[end + close.len()..]))
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let value = &tag[start..];
    Some(value[..value.find('"')?].to_string())
}

/// Plain text of a summary cell: tags dropped, entities decoded, whitespace collapsed.
fn text_of(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        match rest[open..].find('>') {
            Some(close) => rest = &rest[open + close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    text.push_str(rest);
    let decoded = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut an over-long summary at a word boundary. rustdoc's own first-line summaries are short; a
/// hand-written first line need not be.
fn truncate(text: &str) -> String {
    let Some((cut, _)) = text.char_indices().nth(SUMMARY_LIMIT) else {
        return text.to_string();
    };
    let head = &text[..cut];
    let head = head.rsplit_once(' ').map_or(head, |(before, _)| before);
    format!(
        "{}…",
        head.trim_end_matches(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
    )
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `1 struct`, `4 structs` — the plural is the heading, the singular drops its `s`.
fn quantity(count: usize, label: &str) -> String {
    let label = label.to_lowercase();
    if count == 1 {
        format!("1 {}", label.strip_suffix('s').unwrap_or(&label))
    } else {
        format!("{count} {label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_entries_are_names_or_name_flag_pairs() {
        let items = parse_sidebar(
            r#"window.SIDEBAR_ITEMS = {"fn":["run","main"],"macro":[["shout",1]],"mod":[]};"#,
        );
        assert_eq!(items["fn"], vec!["main".to_string(), "run".to_string()]);
        assert_eq!(items["macro"], vec!["shout".to_string()]);
        // A kind with no entries is not a group.
        assert!(!items.contains_key("mod"));
    }

    #[test]
    fn summaries_pair_each_item_with_the_description_beside_it() {
        let html = concat!(
            r#"<dl class="item-table">"#,
            r#"<dt><a class="struct" href="struct.Cli.html" title="struct x::Cli">Cli</a></dt>"#,
            r#"<dd>Parsed <code>--flags</code> &amp; args.</dd>"#,
            r#"<dt><a class="fn" href="fn.run.html">run</a></dt>"#,
            r#"<dt><a class="fn" href="fn.main.html">main</a></dt><dd>Entry point.</dd></dl>"#,
        );
        let summaries = summaries(html);
        assert_eq!(summaries["struct.Cli.html"], "Parsed --flags & args.");
        assert_eq!(summaries["fn.main.html"], "Entry point.");
        // `run` has no <dd>: the next item's description must not be attributed to it.
        assert!(!summaries.contains_key("fn.run.html"));
    }

    #[test]
    fn long_summaries_are_cut_at_a_word_boundary() {
        let text = "word ".repeat(80);
        let cut = truncate(&text);
        assert!(cut.len() <= SUMMARY_LIMIT + 4, "{cut}");
        assert!(cut.ends_with("word…"), "{cut}");
    }

    #[test]
    fn crate_outline_is_read_from_the_rustdoc_tree() {
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot/site/api/rust_app");
        let outline = outline(&dir, "rust-app").expect("fixture rustdoc tree");

        let labels: Vec<&str> = outline
            .groups
            .iter()
            .map(|group| group.label.as_str())
            .collect();
        // rustdoc's own order, not the alphabetical order of the JSON keys.
        assert_eq!(labels, ["Macros", "Structs", "Enums", "Functions"]);
        assert_eq!(
            outline.tally(),
            "1 module · 1 macro · 1 struct · 1 enum · 2 functions"
        );

        let structs = &outline.groups[1].items[0];
        assert_eq!(structs.name, "Cli");
        assert_eq!(structs.href, "struct.Cli.html");
        assert_eq!(
            structs.summary.as_deref(),
            Some("Parsed command line & flags.")
        );

        let cache = &outline.modules[0];
        assert_eq!(cache.name, "cache");
        assert_eq!(cache.href, "cache/index.html");
        assert_eq!(
            cache.summary.as_deref(),
            Some("Content-addressed task cache.")
        );
        // Child hrefs stay relative to the crate directory, not to their own module.
        assert_eq!(cache.groups[0].items[0].href, "cache/struct.Cache.html");
        // 5 crate-root items + the `cache` module + its 2 items.
        assert_eq!(outline.count(), 8);
    }

    #[test]
    fn no_rustdoc_tree_is_not_an_error() {
        assert!(outline(Path::new("/nonexistent/api/x"), "x").is_none());
    }
}
