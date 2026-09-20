//! Deterministic markdown normalisation, shared by `monodocs fmt` and `monodocs lint --fix`.
//!
//! The rewrite set is deliberately tiny: whitespace, ATX heading spacing and unordered list
//! markers. Everything else — wrapping, table alignment, emphasis style — is left alone, so the
//! tool never fights an author over taste. Fenced code blocks are copied through byte for byte;
//! a formatter that edits code samples is a formatter nobody runs.
//!
//! [`normalize`] is idempotent by construction: every rule maps its own output to itself.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use eyre::{Context, Result, bail};

use crate::discover::{discover, rel};

/// Normalise markdown. `normalize(normalize(x)) == normalize(x)` for every input.
///
/// - trailing whitespace stripped (including two-space hard breaks — use a trailing `\`),
/// - no leading blank lines, exactly one trailing newline,
/// - runs of three or more blank lines collapsed to one,
/// - `#`-runs followed by a single space,
/// - `*` / `+` unordered list markers rewritten to `-` at every nesting level.
///
/// Fenced code blocks, ordered lists and thematic breaks are never touched.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<Fence> = None;
    let mut blanks = 0usize;
    let mut wrote = false;

    for line in text.lines() {
        if let Some(open) = &fence {
            if is_closing_fence(line, open) {
                fence = None;
            }
            push_line(&mut out, line);
            continue;
        }
        if let Some(open) = opening_fence(line) {
            flush_blanks(&mut out, &mut blanks, wrote);
            push_line(&mut out, line);
            wrote = true;
            fence = Some(open);
            continue;
        }

        let line = line.trim_end();
        if line.is_empty() {
            blanks += 1;
            continue;
        }
        flush_blanks(&mut out, &mut blanks, wrote);
        push_line(&mut out, &normalize_line(line));
        wrote = true;
    }
    out
}

/// True when `fmt` would rewrite `text`.
pub fn needs_formatting(text: &str) -> bool {
    normalize(text) != text
}

/// Documents `fmt` and `lint` leave alone, listed one workspace-relative path per line in
/// `<root>/.monodocsignore`. A trailing `/` ignores everything under that directory.
///
/// This exists for *generated* markdown: `docs/pg-cli.md` comes from the clap derive and
/// `libs/pg-gen/README.md` from `cargo-readme`, so normalising them here would only make
/// `just docs-check` fail on the next regeneration. Generated documents are still rendered into
/// the site — they are excluded from rewriting and reporting, not from the document.
#[derive(Default)]
pub struct Ignore {
    entries: Vec<String>,
}

impl Ignore {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".monodocsignore");
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(Self::default());
        };
        let entries = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect();
        Ok(Self { entries })
    }

    /// `rel` is a workspace-relative path with `/` separators, as produced by [`rel`].
    pub fn is_ignored(&self, rel: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| match entry.strip_suffix('/') {
                Some(dir) => rel.starts_with(dir) && rel.as_bytes().get(dir.len()) == Some(&b'/'),
                None => rel == entry,
            })
    }
}

/// Every markdown file the discoverer attributes to a project — project `README.md`s plus the
/// `docs/*.md` beside them — minus anything `.monodocsignore` excludes. Sorted and de-duplicated,
/// so output order never depends on the filesystem.
pub fn discovered_docs(root: &Path) -> Result<Vec<PathBuf>> {
    let projects = discover(root)?;
    let ignore = Ignore::load(root)?;
    let mut docs = BTreeSet::new();
    for project in &projects {
        for doc in &project.docs {
            if ignore.is_ignored(&doc.rel) {
                continue;
            }
            docs.insert(doc.path.clone());
        }
    }
    Ok(docs.into_iter().collect())
}

/// Resolve the `PATHS` operands: markdown files as given, directories expanded to the markdown
/// they contain.
pub fn explicit_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = BTreeSet::new();
    for path in paths {
        if path.is_dir() {
            collect_markdown(path, &mut out)?;
        } else if is_markdown(path) {
            out.insert(path.clone());
        } else if path.exists() {
            bail!("{} is not a markdown file", path.display());
        } else {
            bail!("{} does not exist", path.display());
        }
    }
    Ok(out.into_iter().collect())
}

/// Rewrite `files` in place. Returns the files that changed (or, with `check`, that would).
pub fn format_files(files: &[PathBuf], check: bool) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for file in files {
        let text =
            fs::read_to_string(file).wrap_err_with(|| format!("reading {}", file.display()))?;
        let formatted = normalize(&text);
        if formatted == text {
            continue;
        }
        changed.push(file.clone());
        if !check {
            fs::write(file, &formatted).wrap_err_with(|| format!("writing {}", file.display()))?;
        }
    }
    Ok(changed)
}

/// `monodocs fmt` end to end: resolve the file set, rewrite or report, summarise.
pub fn run(root: &Path, check: bool, paths: &[PathBuf]) -> Result<()> {
    let root = fs::canonicalize(root).wrap_err_with(|| format!("resolving {}", root.display()))?;
    let files = if paths.is_empty() {
        discovered_docs(&root)?
    } else {
        explicit_paths(paths)?
    };
    if files.is_empty() {
        eprintln!("no markdown files found");
        return Ok(());
    }

    let changed = format_files(&files, check)?;
    for file in &changed {
        println!("{}", rel(&root, file));
    }
    let plural = if files.len() == 1 { "" } else { "s" };
    if check {
        if !changed.is_empty() {
            bail!(
                "{} of {} markdown file{plural} need formatting — run `monodocs fmt`",
                changed.len(),
                files.len()
            );
        }
        eprintln!("{} markdown file{plural} already formatted", files.len());
    } else {
        eprintln!(
            "{} of {} markdown file{plural} formatted",
            changed.len(),
            files.len()
        );
    }
    Ok(())
}

/// An open fence: its delimiter character and length, which the closing fence must match.
struct Fence {
    marker: char,
    len: usize,
}

fn opening_fence(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed
        .chars()
        .next()
        .filter(|ch| *ch == '`' || *ch == '~')?;
    let len = trimmed.chars().take_while(|ch| *ch == marker).count();
    if len < 3 {
        return None;
    }
    // A ``` fence's info string may not contain a backtick.
    if marker == '`' && trimmed[len..].contains('`') {
        return None;
    }
    Some(Fence { marker, len })
}

fn is_closing_fence(line: &str, open: &Fence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let len = trimmed.chars().take_while(|ch| *ch == open.marker).count();
    len >= open.len && trimmed[len..].trim().is_empty()
}

fn normalize_line(line: &str) -> String {
    if let Some(heading) = normalize_heading(line) {
        return heading;
    }
    normalize_bullet(line).unwrap_or_else(|| line.to_string())
}

/// `##   Title` -> `## Title`. Only rewrites what is already an ATX heading: a `#` run that is
/// not followed by whitespace is not a heading, and turning it into one would change the document.
fn normalize_heading(line: &str) -> Option<String> {
    let indent = leading_spaces(line)?;
    let rest = &line[indent.len()..];
    let hashes = rest.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let body = rest[hashes..].trim_start_matches([' ', '\t']);
    if body.len() == rest.len() - hashes {
        return None; // no whitespace after the `#` run: not a heading
    }
    if body.is_empty() {
        return None; // `#` with only trailing spaces; already stripped by the caller
    }
    Some(format!("{indent}{} {body}", &rest[..hashes]))
}

/// `* item` / `+ item` -> `- item`, preserving indentation and the gap after the marker.
fn normalize_bullet(line: &str) -> Option<String> {
    let indent = leading_whitespace(line);
    let rest = &line[indent.len()..];
    let marker = rest.chars().next().filter(|ch| *ch == '*' || *ch == '+')?;
    let after = &rest[marker.len_utf8()..];
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    if is_thematic_break(rest) {
        return None;
    }
    Some(format!("{indent}-{after}"))
}

/// `***`, `* * *`, `---`, `___`: a list marker rewrite here would change the rendering.
fn is_thematic_break(rest: &str) -> bool {
    let Some(marker @ ('*' | '-' | '_')) = rest.chars().next() else {
        return false;
    };
    let mut count = 0;
    for ch in rest.chars() {
        if ch == marker {
            count += 1;
        } else if ch != ' ' && ch != '\t' {
            return false;
        }
    }
    count >= 3
}

/// Up to three leading spaces, the most an ATX heading may be indented by.
fn leading_spaces(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    (indent <= 3).then(|| &line[..indent])
}

fn leading_whitespace(line: &str) -> &str {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..indent]
}

fn flush_blanks(out: &mut String, blanks: &mut usize, wrote: bool) {
    let count = std::mem::take(blanks);
    if !wrote {
        return; // leading blank lines are dropped
    }
    for _ in 0..if count >= 3 { 1 } else { count } {
        out.push('\n');
    }
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

fn collect_markdown(dir: &Path, out: &mut BTreeSet<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).wrap_err_with(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        if entry.file_type()?.is_dir() {
            collect_markdown(&path, out)?;
        } else if is_markdown(&path) {
            out.insert(path);
        }
    }
    Ok(())
}

/// A unique empty directory under the system temp dir, for tests that need real files on disk.
#[cfg(test)]
pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("monodocs-{tag}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSY: &str =
        "\n\n#   Title  \n\n\n\n* one   \n* two\n  + nested\n\n1. ordered\n\ntext\n";

    #[test]
    fn normalizes_whitespace_headings_and_bullets() {
        assert_eq!(
            normalize(MESSY),
            "# Title\n\n- one\n- two\n  - nested\n\n1. ordered\n\ntext\n"
        );
    }

    #[test]
    fn formatting_is_idempotent() {
        let once = normalize(MESSY);
        assert_eq!(normalize(&once), once);
        // And on a document that exercises every rule at once.
        let gnarly =
            "###heading\n##  heading\n\n\n\n\n+ a\n\t* b\n***\n```\n* not a bullet   \n```\n\n\n";
        let once = normalize(gnarly);
        assert_eq!(normalize(&once), once);
    }

    #[test]
    fn fence_contents_are_untouched() {
        let text =
            "# T\n\n```md\n*  star bullet   \n\n\n\n###heading\n```\n\n~~~\n*  keep   \n~~~\n";
        assert_eq!(normalize(text), text);
    }

    #[test]
    fn fences_close_only_on_a_matching_delimiter() {
        // The inner ``` run is shorter than the ````` opener, so it stays inside the block.
        let text = "````\n```\n*  kept   \n```\n````\n\n*  fixed\n";
        assert_eq!(
            normalize(text),
            "````\n```\n*  kept   \n```\n````\n\n-  fixed\n"
        );
    }

    #[test]
    fn thematic_breaks_and_non_headings_survive() {
        let text = "***\n* * *\n#hashtag not a heading\n*emphasis* first\n";
        assert_eq!(normalize(text), text);
    }

    #[test]
    fn two_blank_lines_are_kept_but_three_collapse() {
        assert_eq!(normalize("a\n\n\nb\n"), "a\n\n\nb\n");
        assert_eq!(normalize("a\n\n\n\nb\n"), "a\n\nb\n");
    }

    #[test]
    fn blank_documents_normalise_to_nothing() {
        assert_eq!(normalize("\n\n  \n"), "");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn eof_gets_exactly_one_newline() {
        assert_eq!(normalize("a"), "a\n");
        assert_eq!(normalize("a\n\n\n"), "a\n");
    }

    #[test]
    fn check_reports_without_writing_and_rewriting_is_a_one_time_change() {
        let dir = scratch_dir("fmt");
        let file = dir.join("README.md");
        fs::write(&file, MESSY).expect("write");

        let would = format_files(std::slice::from_ref(&file), true).expect("check");
        assert_eq!(would, vec![file.clone()], "--check reports the file");
        assert_eq!(
            fs::read_to_string(&file).expect("read"),
            MESSY,
            "--check must not write"
        );

        let changed = format_files(std::slice::from_ref(&file), false).expect("format");
        assert_eq!(changed, vec![file.clone()]);
        assert_eq!(fs::read_to_string(&file).expect("read"), normalize(MESSY));
        assert!(
            format_files(std::slice::from_ref(&file), false)
                .expect("format")
                .is_empty(),
            "a formatted file is left alone"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignore_matches_files_and_directories_but_not_prefixes() {
        let root = scratch_dir("ignore");
        fs::write(
            root.join(".monodocsignore"),
            "# generated\ndocs/pg-cli.md\nvendor/\n\n",
        )
        .expect("write");
        let ignore = Ignore::load(&root).expect("load");

        assert!(ignore.is_ignored("docs/pg-cli.md"));
        assert!(ignore.is_ignored("vendor/nested/README.md"));
        assert!(!ignore.is_ignored("docs/pg-cli.md.bak"));
        assert!(!ignore.is_ignored("vendored/README.md"));
        assert!(!ignore.is_ignored("README.md"));
        // A missing ignore file is not an error, and excludes nothing.
        let none = Ignore::load(&root.join("empty")).expect("load");
        assert!(!none.is_ignored("docs/pg-cli.md"));

        fs::remove_dir_all(&root).ok();
    }
}
