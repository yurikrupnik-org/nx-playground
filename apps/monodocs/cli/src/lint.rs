//! Documentation checks: the things that silently rot between renders.
//!
//! Every rule answers a question the rendered document cannot: does this link still point at a
//! file, does this `#anchor` still match a heading, can the fence actually be highlighted, is the
//! project documented at all. Findings are `path:line: message`, so editors and CI can parse them.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use eyre::{Context, Result, bail};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::{
    discover::{Project, discover, rel, slugify},
    fmt,
    highlight::{is_plain, known_languages, label_for},
    markdown::{has_scheme, normalize as normalize_path, options},
};

/// One problem, rendered as `path[:line]: message`.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    /// Workspace-relative path of the file (or project directory) at fault.
    pub path: String,
    /// 1-based line, when the rule knows one.
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{}: {}", self.path, line, self.message),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

/// `monodocs lint`. With `fix`, the formatting subset is repaired first; anything left is a
/// finding and makes the command fail.
pub fn run(root: &Path, fix: bool) -> Result<()> {
    let root = fs::canonicalize(root).wrap_err_with(|| format!("resolving {}", root.display()))?;

    if fix {
        let docs = fmt::discovered_docs(&root)?;
        for file in fmt::format_files(&docs, false)? {
            println!("fixed formatting: {}", rel(&root, &file));
        }
    }

    let projects = discover(&root)?;
    let findings = check(&root, &projects)?;
    for finding in &findings {
        println!("{finding}");
    }

    // The same file set `fmt` works on, so the two commands never disagree about scope.
    let docs = fmt::discovered_docs(&root)?.len();
    if findings.is_empty() {
        eprintln!(
            "no problems in {docs} markdown files across {} projects",
            projects.len()
        );
        return Ok(());
    }
    bail!(
        "{} problem{} in {docs} markdown files across {} projects",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" },
        projects.len()
    );
}

/// Every finding for `projects`, in project order and then document order.
pub fn check(root: &Path, projects: &[Project]) -> Result<Vec<Finding>> {
    let ignore = fmt::Ignore::load(root)?;
    let mut findings = Vec::new();
    let mut anchors = AnchorCache::default();
    for project in projects {
        if project.is_undocumented() {
            findings.push(Finding {
                path: project.rel_dir.clone(),
                line: None,
                message: format!("project `{}` has no README.md", project.name),
            });
        }
        for doc in &project.docs {
            if ignore.is_ignored(&doc.rel) {
                continue;
            }
            let text = fs::read_to_string(&doc.path)
                .wrap_err_with(|| format!("reading {}", doc.path.display()))?;
            check_doc(root, &doc.path, &text, &mut anchors, &mut findings);
        }
    }
    Ok(findings)
}

fn check_doc(
    root: &Path,
    path: &Path,
    text: &str,
    anchors: &mut AnchorCache,
    findings: &mut Vec<Finding>,
) {
    let rel_path = rel(root, path);
    let dir = path.parent().unwrap_or(Path::new("."));
    let lines = LineIndex::new(text);
    let mut push = |line: usize, message: String| {
        findings.push(Finding {
            path: rel_path.clone(),
            line: Some(line),
            message,
        });
    };

    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        let line = lines.line_of(range.start);
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some(problem) = check_target(dir, &dest_url, anchors, true) {
                    push(line, problem);
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if let Some(problem) = check_target(dir, &dest_url, anchors, false) {
                    push(line, problem);
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                if let Some(problem) = check_fence(&info) {
                    push(line, problem);
                }
            }
            _ => {}
        }
    }

    // Self-referencing anchors need this document's own headings, which the cache already has.
    anchors.insert(path, text);
    let own = anchors.get(path).cloned().unwrap_or_default();
    for (line, fragment) in own_anchor_links(text) {
        if !own.contains(&slugify(&fragment)) {
            findings.push(Finding {
                path: rel_path.clone(),
                line: Some(line),
                message: format!("no heading in this document matches `#{fragment}`"),
            });
        }
    }

    if fmt::needs_formatting(text) {
        findings.push(Finding {
            path: rel_path,
            line: None,
            message: "formatting differs — run `monodocs fmt`".to_string(),
        });
    }
}

/// Fragment-only links (`#usage`), which [`check_target`] deliberately skips because they need
/// the current document rather than a target file.
fn own_anchor_links(text: &str) -> Vec<(usize, String)> {
    let lines = LineIndex::new(text);
    let mut out = Vec::new();
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        let Event::Start(Tag::Link { dest_url, .. }) = event else {
            continue;
        };
        if let Some(fragment) = dest_url.strip_prefix('#')
            && !fragment.is_empty()
        {
            out.push((lines.line_of(range.start), fragment.to_string()));
        }
    }
    out
}

/// A relative link or image: does the file exist, and does the `#fragment` match a heading?
fn check_target(
    dir: &Path,
    dest: &str,
    anchors: &mut AnchorCache,
    is_link: bool,
) -> Option<String> {
    if dest.is_empty() || dest.starts_with('#') || has_scheme(dest) {
        return None;
    }
    let (path_part, fragment) = match dest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (dest, None),
    };
    if path_part.is_empty() {
        return None;
    }
    let kind = if is_link { "link" } else { "image" };
    let target = normalize_path(&dir.join(path_part));
    if !target.exists() {
        return Some(format!("{kind} target does not exist: {dest}"));
    }

    let fragment = fragment.filter(|fragment| !fragment.is_empty())?;
    let headings = anchors.load(&target)?;
    if headings.contains(&slugify(fragment)) {
        return None;
    }
    Some(format!("no heading in {path_part} matches `#{fragment}`"))
}

/// Fences the renderer cannot highlight show up as plain text, which is nearly always a typo
/// (`javascrip`, `shell-session`) rather than a deliberate choice — `text` says so on purpose.
fn check_fence(info: &str) -> Option<String> {
    let name = info.trim();
    if name.is_empty() {
        return Some("fenced code block has no language".to_string());
    }
    if label_for(name).is_some() || is_plain(name) {
        return None;
    }
    let mut known = String::new();
    for lang in known_languages() {
        let _ = write!(known, "{lang}, ");
    }
    known.push_str("text");
    Some(format!(
        "fence language `{name}` is not highlighted (known: {known})"
    ))
}

/// Heading anchors per markdown file, parsed once.
#[derive(Default)]
struct AnchorCache {
    files: HashMap<PathBuf, HashSet<String>>,
}

impl AnchorCache {
    /// Anchors of `path`, or `None` when it is not markdown that can be checked.
    fn load(&mut self, path: &Path) -> Option<&HashSet<String>> {
        let is_markdown = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
            .unwrap_or(false);
        if !is_markdown {
            return None;
        }
        if !self.files.contains_key(path) {
            let text = fs::read_to_string(path).ok()?;
            self.insert(path, &text);
        }
        self.files.get(path)
    }

    fn insert(&mut self, path: &Path, text: &str) {
        if self.files.contains_key(path) {
            return;
        }
        self.files.insert(path.to_path_buf(), anchors_of(text));
    }

    fn get(&self, path: &Path) -> Option<&HashSet<String>> {
        self.files.get(path)
    }
}

/// The anchors a document offers: one per heading, slugified exactly as the renderer does, plus
/// any explicit `id=`/`name=` attribute in raw HTML.
fn anchors_of(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut heading = None;
    for event in Parser::new_ext(text, options()) {
        match event {
            Event::Start(Tag::Heading { .. }) => heading = Some(String::new()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(plain) = heading.as_mut() {
                    plain.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(plain) = heading.take() {
                    out.insert(slugify(&plain));
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                for id in html_ids(&html) {
                    out.insert(slugify(&id));
                    out.insert(id);
                }
            }
            _ => {}
        }
    }
    out
}

/// `id="x"` / `name='x'` values in a raw HTML chunk; hand-written anchors are legitimate targets.
fn html_ids(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["id=", "name="] {
        let mut rest = html;
        while let Some(at) = rest.find(attr) {
            rest = &rest[at + attr.len()..];
            let Some(quote @ ('"' | '\'')) = rest.chars().next() else {
                continue;
            };
            let Some(end) = rest[1..].find(quote) else {
                continue;
            };
            out.push(rest[1..1 + end].to_string());
            rest = &rest[1 + end..];
        }
    }
    out
}

/// Byte offset -> 1-based line number.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.char_indices()
                .filter(|(_, ch)| *ch == '\n')
                .map(|(index, _)| index + 1),
        );
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(index) => index + 1,
            Err(index) => index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn findings_for(name: &str) -> Vec<Finding> {
        let root = fixture(name);
        let projects = discover(&root).expect("discovery");
        check(&root, &projects).expect("lint")
    }

    fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(Finding::to_string).collect()
    }

    #[test]
    fn line_numbers_are_one_based() {
        let index = LineIndex::new("a\nbb\n\nc");
        assert_eq!(index.line_of(0), 1);
        assert_eq!(index.line_of(2), 2);
        assert_eq!(index.line_of(3), 2);
        assert_eq!(index.line_of(6), 4);
    }

    #[test]
    fn clean_fixture_reports_only_the_missing_readme() {
        let messages = messages(&findings_for("polyglot"));
        assert_eq!(
            messages,
            vec!["no-readme: project `no-readme` has no README.md".to_string()],
            "the polyglot fixture is clean apart from its deliberate gap"
        );
    }

    #[test]
    fn every_rule_fires_on_the_messy_fixture() {
        let messages = messages(&findings_for("messy"));
        let expect = [
            "kcl-broken/README.md:9: link target does not exist: ./missing.md",
            "kcl-broken/README.md:11: image target does not exist: ./missing.png",
            "kcl-broken/README.md:13: no heading in docs/guide.md matches `#nowhere`",
            "kcl-broken/README.md:17: no heading in this document matches `#absent`",
            "kcl-broken/README.md:19: fenced code block has no language",
            "kcl-broken/README.md: formatting differs — run `monodocs fmt`",
            "undocumented: project `undocumented` has no README.md",
        ];
        for expected in expect {
            assert!(
                messages.iter().any(|message| message == expected),
                "missing finding {expected:?}\nfound: {messages:#?}"
            );
        }
        // Unhighlightable fences name the language and list the ones that work.
        assert!(
            messages.iter().any(|message| message
                .contains("fence language `brainfuck` is not highlighted")
                && message.contains("kcl")),
            "found: {messages:#?}"
        );
        // References that do resolve stay silent: the `#usage` heading anchor, the
        // `docs/guide.md#install` anchor, and anything carrying a scheme.
        assert!(
            !messages.iter().any(|message| message.contains("#usage")
                || message.contains("#install")
                || message.contains("https://")),
            "false positive in {messages:#?}"
        );
        // The supplementary document is clean, so it contributes nothing at all.
        assert!(
            !messages
                .iter()
                .any(|message| message.starts_with("kcl-broken/docs/guide.md")),
            "false positive in {messages:#?}"
        );
    }

    #[test]
    fn fix_repairs_formatting_and_leaves_the_rest_reported() {
        let root = fmt::scratch_dir("lint");
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("package dir");
        fs::write(root.join("README.md"), "# Root\n\nClean.\n").expect("root readme");
        fs::write(pkg.join("kcl.mod"), "[package]\nname = \"pkg\"\n").expect("manifest");
        let readme = pkg.join("README.md");
        fs::write(&readme, "# Pkg\n\n* [gone](./gone.md)   \n").expect("readme");

        let before = messages(&check(&root, &discover(&root).expect("discovery")).expect("lint"));
        assert!(
            before.contains(&"pkg/README.md: formatting differs — run `monodocs fmt`".to_string())
                && before.contains(
                    &"pkg/README.md:3: link target does not exist: ./gone.md".to_string()
                ),
            "{before:#?}"
        );

        fmt::format_files(&fmt::discovered_docs(&root).expect("docs"), false).expect("fix");

        let after = messages(&check(&root, &discover(&root).expect("discovery")).expect("lint"));
        assert_eq!(
            after,
            vec!["pkg/README.md:3: link target does not exist: ./gone.md".to_string()],
            "--fix repairs formatting and nothing else"
        );
        assert_eq!(
            fs::read_to_string(&readme).expect("read"),
            "# Pkg\n\n- [gone](./gone.md)\n"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn html_anchors_count_as_headings() {
        let anchors = anchors_of("<a id=\"manual\"></a>\n\n# Real Heading\n");
        assert!(anchors.contains("manual"));
        assert!(anchors.contains("real-heading"));
    }

    #[test]
    fn known_fences_pass_unknown_ones_fail() {
        assert_eq!(check_fence("rust"), None);
        assert_eq!(check_fence("ts"), None);
        assert_eq!(check_fence("kcl,no_run"), None);
        assert!(check_fence("").is_some());
        assert!(check_fence("cobol").is_some());
    }
}
