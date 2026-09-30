//! Static shell scan: which shell a checkout executes, how big and branchy it
//! is, and what ShellCheck says about it ([`contract_taskgraph::shell`]).
//!
//! Pure functions only — discovery filters, go-task template neutralising,
//! size/complexity metrics and ShellCheck `json1` mapping. Spawning `git`
//! and `shellcheck` is the CLI's job (`taskgraph shell-scan`).
//!
//! Metrics are deliberately lexical, so they are cheap, stable and explainable:
//! - `lines`: lines that are neither blank nor a `#` comment (shebang included
//!   in "comment").
//! - `branches`: whole words `if`, `elif`, `case`, `for`, `while`, `until`
//!   plus every `&&` and `||`, outside comment lines. No parsing: a keyword
//!   inside a quoted string counts, and a `case` counts once however many arms
//!   it has.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use contract_taskgraph::Graph;
use contract_taskgraph::shell::{
    FindingLevel, ShellFinding, ShellScan, ShellSource, ShellSourceKind,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// What a go-task template (`{{.VAR}}`, `{{if …}}`) becomes before linting:
/// one plain word, so the snippet stays syntactically what go-task will run
/// once it has rendered the template.
pub const TEMPLATE_PLACEHOLDER: &str = "__TPL__";

/// Prefix the Taskfile parser puts on `defer:` commands.
const DEFER_PREFIX: &str = "defer: ";

/// ShellCheck codes excluded for Taskfile snippets only (script files get
/// every check). Each is an artefact of linting a fragment out of context:
/// - SC2148 "add a shebang": a snippet has none; go-task decides the shell
///   and we pass `--shell=bash`.
/// - SC2154 "var is referenced but not assigned": go-task injects `env:`,
///   `dotenv:` and the caller's environment, none of which is visible in the
///   snippet.
/// - SC1091 "not following sourced file": the snippet is linted from a temp
///   file, so relative `source` paths (resolved against the task's `dir:`)
///   can never be followed.
pub const SNIPPET_EXCLUDES: [u32; 3] = [1091, 2148, 2154];

/// A piece of Taskfile shell ready for the linter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub source: ShellSource,
    /// Templates neutralised; this is what ShellCheck sees.
    pub lint_text: String,
}

impl Snippet {
    /// Whether `finding` touches a neutralised template. Such findings are
    /// about text go-task renders at run time (`for x in {{.LIST}}` "runs
    /// once", `case {{.X}} in` "is constant", `[ n -gt {{.MAX}} ]` "is not a
    /// number"), not about the shell as written, so the scan drops them.
    /// Columns are counted in characters, as ShellCheck reports them.
    pub fn is_template_artifact(&self, finding: &ShellFinding) -> bool {
        let start = (finding.line, finding.column);
        let end = (finding.end_line, finding.end_column);
        let width = u32::try_from(TEMPLATE_PLACEHOLDER.chars().count()).unwrap_or(u32::MAX);
        self.lint_text.lines().zip(1u32..).any(|(text, line)| {
            text.match_indices(TEMPLATE_PLACEHOLDER).any(|(byte, _)| {
                let col = u32::try_from(text[..byte].chars().count()).unwrap_or(u32::MAX) + 1;
                // Half-open ranges [col, col + width) and [start, end) intersect.
                (line, col) < end && (line, col + width) > start
            })
        })
    }
}

/// Whether a tracked file is a shell script: `*.sh`/`*.bash`, or no
/// extension and a `sh`/`bash` shebang (`#!/bin/sh`, `#!/usr/bin/env bash`,
/// `#!/usr/bin/env -S bash -eu`). `first_line` is only consulted for files
/// without an extension.
pub fn is_shell_script(path: &str, first_line: Option<&str>) -> bool {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some(ext) => ext == "sh" || ext == "bash",
        None => first_line.is_some_and(is_sh_shebang),
    }
}

fn is_sh_shebang(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("#!") else {
        return false;
    };
    let mut words = rest.split_whitespace();
    let Some(program) = words.next() else {
        return false;
    };
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let interpreter = if base(program) == "env" {
        match words.find(|w| !w.starts_with('-')) {
            Some(w) => base(w),
            None => return false,
        }
    } else {
        base(program)
    };
    interpreter == "sh" || interpreter == "bash"
}

/// Replace every `{{ … }}` with [`TEMPLATE_PLACEHOLDER`]. An unterminated
/// `{{` is left as written.
pub fn neutralize_templates(text: &str) -> Cow<'_, str> {
    if !text.contains("{{") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(TEMPLATE_PLACEHOLDER);
        rest = &rest[open + 2 + close + 2..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// `(lines, branches)` as defined in the module docs.
pub fn measure(text: &str) -> (u32, u32) {
    const KEYWORDS: [&str; 6] = ["if", "elif", "case", "for", "while", "until"];
    let mut lines = 0u32;
    let mut branches = 0u32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        lines += 1;
        let operators = line.matches("&&").count() + line.matches("||").count();
        let keywords = line
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| KEYWORDS.contains(w))
            .count();
        branches += u32::try_from(operators + keywords).unwrap_or(u32::MAX);
    }
    (lines, branches)
}

/// Hex SHA-256 of `text`.
pub fn digest(text: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// A script file's source record; `path` is relative to the scan root.
pub fn file_source(path: &str, text: &str) -> ShellSource {
    let (lines, branches) = measure(text);
    ShellSource {
        id: format!("file:{path}"),
        kind: ShellSourceKind::File,
        path: path.to_string(),
        task: None,
        index: None,
        lines,
        branches,
        digest: digest(text),
    }
}

/// Every non-empty `cmds:`/`status:`/`preconditions:` entry of every task.
///
/// `index` is the entry's position in its list as parsed (so a `defer:`
/// command keeps its slot); `digest` is over the text as written, `lines`
/// and `branches` over the neutralised text (a `{{if}}` is go-template, not
/// shell).
pub fn task_snippets(graph: &Graph) -> Vec<Snippet> {
    let mut out = Vec::new();
    for task in &graph.tasks {
        let lists = [
            (ShellSourceKind::TaskCmd, &task.cmds),
            (ShellSourceKind::TaskStatus, &task.status),
            (ShellSourceKind::TaskPrecondition, &task.preconditions),
        ];
        for (kind, entries) in lists {
            for (index, raw) in entries.iter().enumerate() {
                let text = match kind {
                    ShellSourceKind::TaskCmd => raw.strip_prefix(DEFER_PREFIX).unwrap_or(raw),
                    _ => raw.as_str(),
                };
                if text.trim().is_empty() {
                    continue;
                }
                let lint_text = neutralize_templates(text).into_owned();
                let (lines, branches) = measure(&lint_text);
                let index = u32::try_from(index).unwrap_or(u32::MAX);
                out.push(Snippet {
                    source: ShellSource {
                        id: format!("{}:{}:{}:{index}", kind_str(kind), task.taskfile, task.name),
                        kind,
                        path: task.taskfile.clone(),
                        task: Some(task.name.clone()),
                        index: Some(index),
                        lines,
                        branches,
                        digest: digest(text),
                    },
                    lint_text,
                });
            }
        }
    }
    out
}

fn kind_str(kind: ShellSourceKind) -> &'static str {
    match kind {
        ShellSourceKind::File => "file",
        ShellSourceKind::TaskCmd => "task_cmd",
        ShellSourceKind::TaskStatus => "task_status",
        ShellSourceKind::TaskPrecondition => "task_precondition",
    }
}

/// `shellcheck --version` → `shellcheck 0.10.0`.
pub fn tool_version(version_output: &str) -> Option<String> {
    version_output
        .lines()
        .find_map(|l| l.trim().strip_prefix("version:"))
        .map(|v| format!("shellcheck {}", v.trim()))
}

#[derive(Deserialize)]
struct Json1 {
    comments: Vec<Json1Comment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Json1Comment {
    file: String,
    line: u32,
    end_line: u32,
    column: u32,
    end_column: u32,
    level: String,
    code: u32,
    message: String,
}

/// Map ShellCheck `-f json1` output to findings. `source_ids` maps the file
/// name as passed to ShellCheck to its source id; comments on other files
/// (a followed `source`) are dropped, as are unknown levels.
pub fn parse_json1(
    json: &str,
    source_ids: &HashMap<String, String>,
) -> Result<Vec<ShellFinding>, serde_json::Error> {
    let parsed: Json1 = serde_json::from_str(json)?;
    Ok(parsed
        .comments
        .into_iter()
        .filter_map(|c| {
            let level = match c.level.as_str() {
                "error" => FindingLevel::Error,
                "warning" => FindingLevel::Warning,
                "info" => FindingLevel::Info,
                "style" => FindingLevel::Style,
                _ => return None,
            };
            Some(ShellFinding {
                source_id: source_ids.get(&c.file)?.clone(),
                line: c.line,
                column: c.column,
                end_line: c.end_line,
                end_column: c.end_column,
                level,
                code: c.code,
                message: c.message,
            })
        })
        .collect())
}

/// Put `sources` and `findings` in the contract's order.
pub fn sort_scan(scan: &mut ShellScan) {
    scan.sources.sort_by(|a, b| a.id.cmp(&b.id));
    scan.findings.sort_by(|a, b| {
        (&a.source_id, a.line, a.column, a.code).cmp(&(&b.source_id, b.line, b.column, b.code))
    });
}

#[cfg(test)]
mod tests {
    use contract_taskgraph::TaskNode;

    use super::*;

    #[test]
    fn scripts_are_recognised_by_extension_or_sh_shebang() {
        assert!(is_shell_script("scripts/a.sh", None));
        assert!(is_shell_script(
            "scripts/a.bash",
            Some("#!/usr/bin/env python3")
        ));
        assert!(!is_shell_script("scripts/a.py", Some("#!/bin/bash")));
        assert!(!is_shell_script("scripts/a.zsh", Some("#!/bin/zsh")));
        assert!(is_shell_script("bin/tool", Some("#!/bin/sh")));
        assert!(is_shell_script("bin/tool", Some("#!/usr/bin/env bash")));
        assert!(is_shell_script(
            "bin/tool",
            Some("#!/usr/bin/env -S bash -eu")
        ));
        assert!(is_shell_script("bin/tool", Some("#! /bin/bash -e")));
        assert!(!is_shell_script("bin/tool", Some("#!/usr/bin/env zsh")));
        assert!(!is_shell_script("bin/tool", Some("#!/usr/bin/env node")));
        assert!(!is_shell_script("bin/tool", Some("echo hi")));
        assert!(!is_shell_script("bin/tool", None));
    }

    #[test]
    fn templates_become_one_word_and_unterminated_ones_stay() {
        assert_eq!(
            neutralize_templates(
                r#"docker build -t "{{.IMAGE}}:{{ .TAG }}" {{if .X}}--push{{end}} ."#
            ),
            r#"docker build -t "__TPL__:__TPL__" __TPL__--push__TPL__ ."#
        );
        assert_eq!(
            neutralize_templates("echo {{.A}} {{oops"),
            "echo __TPL__ {{oops"
        );
        assert!(matches!(
            neutralize_templates("plain"),
            Cow::Borrowed("plain")
        ));
    }

    #[test]
    fn metrics_skip_blank_and_comment_lines_and_count_decisions() {
        let script = "#!/bin/bash\n\n# if this were code\nif [ -f x ]; then\n  a && b || c\nelif true; then\n  for i in 1 2; do :; done\nfi\ncase $x in a) ;; esac\nwhile false; do :; done\nverify_iff notify\n";
        let (lines, branches) = measure(script);
        assert_eq!(lines, 8);
        // if, &&, ||, elif, for, case, while — `verify_iff`/`notify` are not keywords.
        assert_eq!(branches, 7);
    }

    fn task(name: &str, file: &str, cmds: &[&str], status: &[&str]) -> TaskNode {
        TaskNode {
            name: name.into(),
            desc: None,
            summary: None,
            aliases: vec![],
            taskfile: file.into(),
            internal: false,
            deps: vec![],
            calls: vec![],
            cmds: cmds.iter().map(|s| s.to_string()).collect(),
            sources: vec![],
            generates: vec![],
            status: status.iter().map(|s| s.to_string()).collect(),
            preconditions: vec![],
            requires: vec![],
            run: None,
            dir: None,
        }
    }

    #[test]
    fn snippets_keep_list_positions_strip_defer_and_digest_the_raw_text() {
        let graph = Graph {
            id: "g".into(),
            taskfile: "/repo/Taskfile.yml".into(),
            host: "h".into(),
            digest: "d".into(),
            tasks: vec![task(
                "db:fresh",
                "scripts/tasks/db.yml",
                &["echo {{.DB}}", "  ", "defer: rm -f {{.TMP}}"],
                &["test -f x && test -f y"],
            )],
            includes: vec![],
            warnings: vec![],
        };
        let snippets = task_snippets(&graph);
        let ids: Vec<&str> = snippets.iter().map(|s| s.source.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "task_cmd:scripts/tasks/db.yml:db:fresh:0",
                "task_cmd:scripts/tasks/db.yml:db:fresh:2",
                "task_status:scripts/tasks/db.yml:db:fresh:0",
            ]
        );
        assert_eq!(snippets[1].lint_text, "rm -f __TPL__");
        assert_eq!(snippets[1].source.digest, digest("rm -f {{.TMP}}"));
        assert_eq!(snippets[2].source.branches, 1);
        assert_eq!(snippets[0].source.path, "scripts/tasks/db.yml");
    }

    #[test]
    fn json1_maps_known_files_and_levels() {
        let json = r#"{"comments":[
            {"file":"/tmp/x/0.bash","line":1,"endLine":1,"column":6,"endColumn":13,"level":"warning","code":2086,"message":"Double quote","fix":null},
            {"file":"scripts/a.sh","line":3,"endLine":3,"column":1,"endColumn":2,"level":"style","code":2250,"message":"Prefer braces"},
            {"file":"lib/sourced.sh","line":1,"endLine":1,"column":1,"endColumn":2,"level":"error","code":1000,"message":"x"},
            {"file":"scripts/a.sh","line":4,"endLine":4,"column":1,"endColumn":2,"level":"bogus","code":1,"message":"y"}
        ]}"#;
        let ids = HashMap::from([
            (
                "/tmp/x/0.bash".to_string(),
                "task_cmd:Taskfile.yml:a:0".to_string(),
            ),
            ("scripts/a.sh".to_string(), "file:scripts/a.sh".to_string()),
        ]);
        let findings = parse_json1(json, &ids).expect("parse");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].source_id, "task_cmd:Taskfile.yml:a:0");
        assert_eq!(
            (
                findings[0].level,
                findings[0].code,
                findings[0].column,
                findings[0].end_column
            ),
            (FindingLevel::Warning, 2086, 6, 13)
        );
        assert_eq!(findings[1].level, FindingLevel::Style);
        assert!(parse_json1("not json", &ids).is_err());
    }

    #[test]
    fn version_line_becomes_the_tool_name() {
        let out = "ShellCheck - shell script analysis tool\nversion: 0.10.0\nlicense: GPL\n";
        assert_eq!(tool_version(out).as_deref(), Some("shellcheck 0.10.0"));
        assert_eq!(tool_version("garbage"), None);
    }

    #[test]
    fn findings_on_a_template_are_artifacts_and_others_are_not() {
        let graph = Graph {
            id: "g".into(),
            taskfile: "/repo/Taskfile.yml".into(),
            host: "h".into(),
            digest: "d".into(),
            tasks: vec![task(
                "t",
                "Taskfile.yml",
                &["x=1\nfor kv in {{.ENV}}; do echo $kv; done"],
                &[],
            )],
            includes: vec![],
            warnings: vec![],
        };
        let snippet = &task_snippets(&graph)[0];
        let at = |line, column, end_column| ShellFinding {
            source_id: snippet.source.id.clone(),
            line,
            column,
            end_line: line,
            end_column,
            level: FindingLevel::Warning,
            code: 2043,
            message: String::new(),
        };
        // `__TPL__` spans columns 11..18 of line 2.
        assert!(snippet.is_template_artifact(&at(2, 11, 18)));
        assert!(
            snippet.is_template_artifact(&at(2, 17, 20)),
            "overlapping tail"
        );
        assert!(
            !snippet.is_template_artifact(&at(2, 18, 20)),
            "adjacent is not overlapping"
        );
        assert!(
            !snippet.is_template_artifact(&at(2, 28, 31)),
            "`$kv` is real shell"
        );
        assert!(!snippet.is_template_artifact(&at(1, 11, 18)), "other line");
    }
}
