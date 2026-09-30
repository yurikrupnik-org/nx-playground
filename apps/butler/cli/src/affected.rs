//! `affected`: which projects a change touches, after nx 23
//! (`command-line-utils.js` `parseFiles`, `project-graph/affected/*`).
//!
//! The touched files come from git exactly as nx computes them (merge-base
//! of `--base`/`NX_BASE`/`defaultBase` with the head, plus uncommitted and
//! untracked files when no head is given). Touched projects come from nx's
//! locators: file ownership, `nx.json`, the `{workspaceRoot}` file inputs a
//! project's targets declare, deleted project manifests, and npm dependency
//! changes. Affected = touched plus everything that depends on them.
//!
//! One deliberate difference: nx maps a root `package.json` dependency change
//! to the npm node it names and walks back to the projects importing that
//! package; butler's graph has no npm nodes, so such a change affects every
//! project (a superset — affected may over-select, never under-select).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use eyre::{Result, WrapErr, bail};

use crate::config::{Json, NxJson};
use crate::graph::ProjectGraph;
use crate::infer::glob;

/// The change range, nx's affected options.
#[derive(Debug, Default, Clone)]
pub struct Range {
    pub base: Option<String>,
    pub head: Option<String>,
    pub files: Vec<String>,
    pub uncommitted: bool,
    pub untracked: bool,
}

/// nx `getBaseRef`, overridden by `NX_BASE`/`NX_HEAD` when the flags are
/// absent.
fn base_and_head(root: &Path, range: &Range) -> Result<(String, Option<String>)> {
    let head = range
        .head
        .clone()
        .or_else(|| std::env::var("NX_HEAD").ok().filter(|h| !h.is_empty()));
    let base = match range
        .base
        .clone()
        .or_else(|| std::env::var("NX_BASE").ok().filter(|b| !b.is_empty()))
    {
        Some(b) => b,
        None => default_base(root)?,
    };
    Ok((base, head))
}

fn default_base(root: &Path) -> Result<String> {
    let path = root.join("nx.json");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let v: Json = serde_json::from_str(&raw).wrap_err("parsing nx.json")?;
        for pointer in ["/defaultBase", "/affected/defaultBase"] {
            if let Some(b) = v.pointer(pointer).and_then(Json::as_str) {
                return Ok(b.to_string());
            }
        }
    }
    Ok("main".into())
}

/// nx `parseFiles`.
pub fn touched_files(root: &Path, range: &Range) -> Result<Vec<String>> {
    if !range.files.is_empty() {
        return Ok(range.files.clone());
    }
    if range.uncommitted {
        return uncommitted(root);
    }
    if range.untracked {
        return untracked(root);
    }
    let (base, head) = base_and_head(root, range)?;
    let base = merge_base(root, &base, head.as_deref().unwrap_or("HEAD"));
    let mut files = match &head {
        Some(h) => diff(root, &base, h)?,
        None => {
            let mut f = diff(root, &base, "HEAD")?;
            f.extend(uncommitted(root)?);
            f.extend(untracked(root)?);
            f
        }
    };
    let mut seen = BTreeSet::new();
    files.retain(|f| seen.insert(f.clone()));
    Ok(files)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .wrap_err_with(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// nx `getMergeBase`: merge-base, then `--fork-point`, then the ref itself.
fn merge_base(root: &Path, base: &str, head: &str) -> String {
    git(root, &["merge-base", base, head])
        .or_else(|_| git(root, &["merge-base", "--fork-point", base, head]))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| base.to_string())
}

fn diff(root: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    Ok(lines(&git(
        root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "--relative",
            base,
            head,
        ],
    )?))
}

fn uncommitted(root: &Path) -> Result<Vec<String>> {
    Ok(lines(&git(
        root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "--relative",
            "HEAD",
            ".",
        ],
    )?))
}

fn untracked(root: &Path) -> Result<Vec<String>> {
    Ok(lines(&git(
        root,
        &["ls-files", "--others", "--exclude-standard"],
    )?))
}

/// Files whose deletion means a project may have disappeared: nx's own
/// manifests plus every configured plugin's `createNodes` glob.
const PROJECT_GLOBS: &[&str] = &[
    "**/package.json",
    "**/project.json",
    "project.json",
    "package.json",
    // @monodon/rust
    "*/**/Cargo.toml",
    // @nx-tools/nx-container
    "**/Dockerfile",
    // tools/nx/plugin.ts MARKERS
    "apps/**/vite.config.ts",
    "apps/**/astro.config.mjs",
    "apps/**/butler.toml",
];

/// nx's lock files for the dependency-update locator (`AUTO_AFFECTED_LOCK_FILES`).
const LOCK_FILES: &[&str] = &[
    "yarn.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "pnpm-lock.yml",
];

/// Projects affected by `touched` files: touched projects plus their
/// dependents.
pub fn affected_projects(
    root: &Path,
    graph: &ProjectGraph,
    nx: &NxJson,
    touched: &[String],
    range: &Range,
) -> Result<BTreeSet<String>> {
    let all = || graph.projects.keys().cloned().collect::<BTreeSet<_>>();
    let mut seeds: BTreeSet<String> = BTreeSet::new();

    // getTouchedProjects: file ownership (nx `findProjectForPath`).
    for f in touched {
        if let Some(p) = graph.project_for_file(f) {
            seeds.insert(p.name.clone());
        }
    }

    // getImplicitlyTouchedProjects: nx.json, and workspace file inputs.
    if touched.iter().any(|f| f == "nx.json") {
        return Ok(all());
    }
    for p in graph.projects.values() {
        let mut patterns = Vec::new();
        for t in p.targets.values() {
            if let Some(inputs) = &t.inputs {
                workspace_files(inputs, &nx.named_inputs, &mut patterns, 0);
            }
        }
        for pattern in patterns {
            let m = glob(&pattern)?;
            if touched.iter().any(|f| m.is_match(f)) {
                seeds.insert(p.name.clone());
                break;
            }
        }
    }

    // getTouchedProjectsFromProjectGlobChanges: a deleted manifest.
    let mut manifest_globs = Vec::with_capacity(PROJECT_GLOBS.len());
    for g in PROJECT_GLOBS {
        manifest_globs.push(glob(g)?);
    }
    if touched
        .iter()
        .any(|f| manifest_globs.iter().any(|m| m.is_match(f)) && !root.join(f).exists())
    {
        return Ok(all());
    }

    // The JS plugin's locators: lock files, root package.json deps, root
    // tsconfig.
    if touched.iter().any(|f| LOCK_FILES.contains(&f.as_str())) {
        return Ok(all());
    }
    if touched.iter().any(|f| f == "package.json") && package_json_deps_changed(root, range)? {
        return Ok(all());
    }
    if touched
        .iter()
        .any(|f| f == "tsconfig.base.json" || f == "tsconfig.json")
    {
        return Ok(all());
    }

    Ok(graph.with_dependents(&seeds))
}

/// nx `extractFilesFromInputs`: `{workspaceRoot}/…` strings and filesets,
/// named inputs expanded.
fn workspace_files(
    inputs: &[Json],
    named: &std::collections::BTreeMap<String, Vec<Json>>,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > 32 {
        return; // a self-referencing named input; nx would recurse forever
    }
    for input in inputs {
        match input {
            Json::String(s) if named.contains_key(s) => {
                workspace_files(&named[s], named, out, depth + 1);
            }
            Json::String(s) => {
                if let Some(rest) = s.strip_prefix("{workspaceRoot}/") {
                    out.push(rest.to_string());
                }
            }
            Json::Object(o) => {
                if let Some(rest) = o
                    .get("fileset")
                    .and_then(Json::as_str)
                    .and_then(|f| f.strip_prefix("{workspaceRoot}/"))
                {
                    out.push(rest.to_string());
                }
            }
            _ => {}
        }
    }
}

/// Whether the root `package.json`'s `dependencies`, `devDependencies`,
/// `overrides` or `resolutions` differ between the base and the head (or the
/// working tree).
fn package_json_deps_changed(root: &Path, range: &Range) -> Result<bool> {
    if !range.files.is_empty() || range.uncommitted || range.untracked {
        return Ok(true); // no base revision to compare with: assume it did
    }
    let (base, head) = base_and_head(root, range)?;
    let base = merge_base(root, &base, head.as_deref().unwrap_or("HEAD"));
    let before = git(root, &["show", &format!("{base}:package.json")]).unwrap_or_default();
    let after = match &head {
        Some(h) => git(root, &["show", &format!("{h}:package.json")]).unwrap_or_default(),
        None => std::fs::read_to_string(root.join("package.json")).unwrap_or_default(),
    };
    let parse = |s: &str| serde_json::from_str::<Json>(s).ok();
    let (Some(a), Some(b)) = (parse(&before), parse(&after)) else {
        return Ok(true); // a whole-file change
    };
    Ok([
        "dependencies",
        "devDependencies",
        "overrides",
        "resolutions",
        "pnpm",
    ]
    .iter()
    .any(|k| a.get(k) != b.get(k)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Project;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn project(name: &str, root: &str, deps: &[&str], targets: Json) -> Project {
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets: serde_json::from_value(targets).unwrap(),
            deps: deps.iter().map(|d| (*d).to_string()).collect(),
            build_deps: BTreeSet::new(),
        }
    }

    fn affected(g: &ProjectGraph, nx: &NxJson, files: &[&str]) -> BTreeSet<String> {
        let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
        let range = Range {
            files: files.clone(),
            ..Default::default()
        };
        affected_projects(Path::new("/nonexistent-butler-ws"), g, nx, &files, &range).unwrap()
    }

    #[test]
    fn locators_follow_nx() {
        let g = ProjectGraph {
            projects: [
                project("lib", "libs/lib", &[], json!({})),
                project(
                    "app",
                    "apps/app",
                    &["lib"],
                    json!({"lint": {"inputs": ["rust", "default"]}}),
                ),
                project("other", "apps/other", &[], json!({})),
            ]
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect(),
            ..Default::default()
        };
        let nx = NxJson {
            named_inputs: BTreeMap::from([(
                "rust".to_string(),
                vec![json!("{workspaceRoot}/Cargo.lock")],
            )]),
            ..Default::default()
        };
        let set = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<BTreeSet<_>>();
        // Ownership plus dependents.
        assert_eq!(
            affected(&g, &nx, &["libs/lib/src/a.rs"]),
            set(&["app", "lib"])
        );
        // A workspace file input touches only the project declaring it.
        assert_eq!(affected(&g, &nx, &["Cargo.lock"]), set(&["app"]));
        // nx.json touches everything; an unowned file nothing.
        assert_eq!(affected(&g, &nx, &["nx.json"]).len(), 3);
        assert!(affected(&g, &nx, &["README.md"]).is_empty());
        // A deleted manifest may have removed a project: everything.
        assert_eq!(affected(&g, &nx, &["apps/gone/package.json"]).len(), 3);
    }
}
