//! `taskgraph shell-scan`: lint every piece of shell this checkout runs and
//! write a [`ShellScan`].
//!
//! Scripts are the files `git ls-files` tracks under the root Taskfile's
//! directory that [`shell::is_shell_script`] accepts; Taskfile shell is every
//! `cmds:`/`status:`/`preconditions:` entry of the parsed graph, templates
//! neutralised and written to a temporary directory so ShellCheck can lint
//! them as bash files. Two ShellCheck invocations (scripts, snippets) run
//! side by side. Findings never fail the command; a missing or broken
//! ShellCheck does.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::Utc;
use contract_taskgraph::shell::{
    FindingLevel, ShellFinding, ShellScan, ShellSource, ShellSourceKind,
};
use domain_taskgraph::origin::{detect_ci, git_head};
use domain_taskgraph::shell::{self, SNIPPET_EXCLUDES};
use eyre::{Result, WrapErr, bail, eyre};
use tracing::warn;
use uuid::Uuid;

use crate::context::{Ctx, root_dir};

/// Files per ShellCheck invocation, far below any platform's argv limit.
const CHUNK: usize = 400;

pub struct ScanOptions {
    pub out: Option<PathBuf>,
    pub shellcheck: String,
}

pub fn shell_scan(ctx: &Ctx, opts: &ScanOptions) -> Result<()> {
    let tool = shellcheck_version(&opts.shellcheck)?;
    let graph = ctx.parse()?;
    let root = root_dir(&graph).to_path_buf();

    let mut sources: Vec<ShellSource> = Vec::new();
    let mut script_ids: HashMap<String, String> = HashMap::new();
    for path in tracked_scripts(&root) {
        let Ok(bytes) = std::fs::read(root.join(&path)) else {
            continue;
        };
        let source = shell::file_source(&path, &String::from_utf8_lossy(&bytes));
        script_ids.insert(path, source.id.clone());
        sources.push(source);
    }

    let snippets = shell::task_snippets(&graph);
    let tmp = TempDir::new()?;
    let mut snippet_ids: HashMap<String, String> = HashMap::new();
    for (n, snippet) in snippets.iter().enumerate() {
        let file = tmp.0.join(format!("{n}.bash"));
        std::fs::write(&file, &snippet.lint_text)
            .wrap_err_with(|| format!("writing {}", file.display()))?;
        snippet_ids.insert(file.display().to_string(), snippet.source.id.clone());
    }

    let excludes = SNIPPET_EXCLUDES.map(|c| format!("SC{c}")).join(",");
    let snippet_args = ["--shell=bash".to_string(), format!("--exclude={excludes}")];
    let (script_findings, snippet_findings) = std::thread::scope(|s| {
        let scripts = s.spawn(|| lint(&opts.shellcheck, &root, &[], &script_ids));
        let found = lint(&opts.shellcheck, &tmp.0, &snippet_args, &snippet_ids);
        let scripts = scripts
            .join()
            .unwrap_or_else(|_| Err(eyre!("script lint thread panicked")));
        (scripts, found)
    });
    let mut findings = script_findings?;
    let by_id: HashMap<&str, &shell::Snippet> =
        snippets.iter().map(|s| (s.source.id.as_str(), s)).collect();
    findings.extend(snippet_findings?.into_iter().filter(|f| {
        !by_id
            .get(f.source_id.as_str())
            .is_some_and(|s| s.is_template_artifact(f))
    }));
    sources.extend(snippets.into_iter().map(|s| s.source));

    let mut scan = ShellScan {
        scan_id: Uuid::now_v7(),
        scanned_at: Utc::now(),
        sha: git_head(&root),
        tool,
        ci: detect_ci(|name| std::env::var(name).ok()),
        sources,
        findings,
    };
    shell::sort_scan(&mut scan);

    let json = serde_json::to_string_pretty(&scan)?;
    match &opts.out {
        Some(out) => {
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(out, json + "\n")
                .wrap_err_with(|| format!("writing {}", out.display()))?;
            eprintln!("{} → {}", summary(&scan), out.display());
        }
        None => {
            println!("{json}");
            eprintln!("{}", summary(&scan));
        }
    }
    Ok(())
}

fn shellcheck_version(bin: &str) -> Result<String> {
    let out = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => eyre!(
                "ShellCheck not found ({bin}); install it (brew install shellcheck, apt-get install shellcheck) or pass --shellcheck PATH"
            ),
            _ => eyre!("running {bin} --version: {e}"),
        })?;
    if !out.status.success() {
        bail!("{bin} --version exited with {}", out.status);
    }
    shell::tool_version(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| eyre!("{bin} --version printed no `version:` line; is it ShellCheck?"))
}

/// Tracked shell scripts, relative to `root`. Outside a git checkout there
/// are none (warned): the Taskfile shell is still scanned.
fn tracked_scripts(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => {
            warn!(root = %root.display(), "not a git checkout (or no git); scanning Taskfile shell only");
            return Vec::new();
        }
    };
    String::from_utf8_lossy(&out)
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let full = root.join(p);
            // Symlinks are skipped: their target is tracked in its own right
            // (or lives outside the checkout).
            if !std::fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_file()) {
                return false;
            }
            let first_line = Path::new(p)
                .extension()
                .is_none()
                .then(|| first_line(&full))
                .flatten();
            shell::is_shell_script(p, first_line.as_deref())
        })
        .map(str::to_string)
        .collect()
}

fn first_line(path: &Path) -> Option<String> {
    let mut head = [0u8; 256];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]);
    text.lines().next().map(str::to_string)
}

/// ShellCheck `files` (keys of `ids`, relative to `cwd` or absolute) in
/// chunks; exit 1 only means "found something".
fn lint(
    bin: &str,
    cwd: &Path,
    extra: &[String],
    ids: &HashMap<String, String>,
) -> Result<Vec<ShellFinding>> {
    let mut files: Vec<&String> = ids.keys().collect();
    files.sort();
    let mut findings = Vec::new();
    for chunk in files.chunks(CHUNK) {
        let out = Command::new(bin)
            .current_dir(cwd)
            .args(["--format=json1"])
            .args(extra)
            .arg("--")
            .args(chunk)
            .stdin(Stdio::null())
            .output()
            .wrap_err_with(|| format!("running {bin}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        match (out.status.code(), shell::parse_json1(&stdout, ids)) {
            (Some(0 | 1), Ok(found)) => findings.extend(found),
            // 2: some files could not be processed; the rest are reported.
            (Some(2), Ok(found)) => {
                warn!(stderr = %String::from_utf8_lossy(&out.stderr).trim(), "ShellCheck skipped some files");
                findings.extend(found);
            }
            (_, parsed) => bail!(
                "{bin} failed ({}): {}{}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim(),
                parsed
                    .err()
                    .map(|e| format!(" (output: {e})"))
                    .unwrap_or_default()
            ),
        }
    }
    Ok(findings)
}

fn summary(scan: &ShellScan) -> String {
    let files = scan
        .sources
        .iter()
        .filter(|s| s.kind == ShellSourceKind::File)
        .count();
    let level = |l: FindingLevel| scan.findings.iter().filter(|f| f.level == l).count();
    format!(
        "{} {}: {} sources ({} scripts, {} Taskfile snippets), {} findings ({} error, {} warning, {} info, {} style)",
        scan.tool,
        scan.sha
            .as_deref()
            .map_or("(no git)", |s| &s[..s.len().min(12)]),
        scan.sources.len(),
        files,
        scan.sources.len() - files,
        scan.findings.len(),
        level(FindingLevel::Error),
        level(FindingLevel::Warning),
        level(FindingLevel::Info),
        level(FindingLevel::Style),
    )
}

/// A scratch directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("taskgraph-shell-scan-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
