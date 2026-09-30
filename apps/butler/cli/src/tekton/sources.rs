//! Where the runnable targets come from. Each source is read from what is on
//! disk, never declared: the project graph for nx, the root Taskfile and its
//! includes, the root justfile, every `.nu` script with a `main`, and every app
//! with a `[workload]`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};
use serde_yaml_ng::Value as Y;

use super::{Deploy, Runner, Target};
use crate::graph::ProjectGraph;
use crate::settings::{AppOverrides, Root};
use crate::{container, k8s};

/// Every `project:target` in the graph.
pub(super) fn nx(graph: &ProjectGraph) -> Vec<Target> {
    graph
        .projects
        .values()
        .flat_map(|p| {
            p.targets.keys().map(|t| Target {
                runner: Runner::Nx,
                name: format!("{}:{t}", p.name),
                params: vec![("project", p.name.clone()), ("target", t.clone())],
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Taskfile

/// go-task's own lookup order for a directory's Taskfile.
const TASKFILE_NAMES: [&str; 8] = [
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

/// Include chains deeper than this are a cycle.
const MAX_INCLUDE_DEPTH: usize = 32;

/// Every task `task <name>` accepts from the workspace root: the root
/// Taskfile's own tasks plus its includes', namespaced (`ns:task`) unless the
/// include is `flatten`ed, minus `internal` ones and an include's `excludes`.
///
/// Remote includes (`https://…`, `git@…`) are skipped: they are not facts of
/// this repository, and generation must not depend on the network.
pub(super) fn taskfile(workspace_root: &Path) -> Result<Vec<Target>> {
    let Some(path) = find_taskfile(workspace_root) else {
        return Ok(Vec::new());
    };
    let mut names = BTreeSet::new();
    collect_tasks(&path, "", false, &[], &mut names, 0)?;
    Ok(names
        .into_iter()
        .map(|name| Target {
            runner: Runner::Task,
            params: vec![("task", name.clone())],
            name,
        })
        .collect())
}

fn find_taskfile(dir: &Path) -> Option<PathBuf> {
    TASKFILE_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn collect_tasks(
    path: &Path,
    prefix: &str,
    internal: bool,
    excludes: &[String],
    out: &mut BTreeSet<String>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_INCLUDE_DEPTH {
        bail!(
            "{}: Taskfile includes nest deeper than {MAX_INCLUDE_DEPTH}; is there a cycle?",
            path.display()
        );
    }
    let raw =
        std::fs::read_to_string(path).map_err(|e| eyre!("reading {}: {e}", path.display()))?;
    let doc: Y =
        serde_yaml_ng::from_str(&raw).map_err(|e| eyre!("parsing {}: {e}", path.display()))?;

    if let Some(tasks) = doc.get("tasks").and_then(Y::as_mapping) {
        for (key, def) in tasks {
            let name = key
                .as_str()
                .ok_or_else(|| eyre!("{}: a task name is not a string", path.display()))?;
            if excludes.iter().any(|e| e == name) || internal || flag(def, "internal") {
                continue;
            }
            let full = format!("{prefix}{name}");
            if !out.insert(full.clone()) {
                bail!("{}: task `{full}` is defined twice", path.display());
            }
        }
    }

    let Some(includes) = doc.get("includes").and_then(Y::as_mapping) else {
        return Ok(());
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    for (key, def) in includes {
        let namespace = key
            .as_str()
            .ok_or_else(|| eyre!("{}: an include name is not a string", path.display()))?;
        let file = match def {
            Y::String(file) => file.as_str(),
            _ => def.get("taskfile").and_then(Y::as_str).ok_or_else(|| {
                eyre!("{}: include `{namespace}` has no taskfile", path.display())
            })?,
        };
        if file.contains("://") || file.starts_with("git@") {
            continue;
        }
        if file.contains("{{") {
            bail!(
                "{}: include `{namespace}` templates its path (`{file}`), which only \
                 go-task can resolve",
                path.display()
            );
        }
        let target = dir.join(file);
        let resolved = if target.is_dir() {
            find_taskfile(&target)
        } else {
            Some(target).filter(|p| p.is_file())
        };
        let Some(resolved) = resolved else {
            if flag(def, "optional") {
                continue;
            }
            bail!(
                "{}: include `{namespace}` points at missing {file}",
                path.display()
            );
        };
        let child_excludes: Vec<String> = def
            .get("excludes")
            .and_then(Y::as_sequence)
            .map(|seq| {
                seq.iter()
                    .filter_map(Y::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let child_prefix = if flag(def, "flatten") {
            prefix.to_string()
        } else {
            format!("{prefix}{namespace}:")
        };
        collect_tasks(
            &resolved,
            &child_prefix,
            internal || flag(def, "internal"),
            &child_excludes,
            out,
            depth + 1,
        )?;
    }
    Ok(())
}

fn flag(def: &Y, key: &str) -> bool {
    def.get(key).and_then(Y::as_bool).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// justfile

/// Every public recipe of the root justfile, modules as `mod::recipe`. The
/// recipe list comes from `just --dump` itself: just's grammar (imports,
/// modules, attributes, conditional recipes) is not something to re-parse.
pub(super) fn justfile(workspace_root: &Path) -> Result<Vec<Target>> {
    let Some(path) = find_justfile(workspace_root)? else {
        return Ok(Vec::new());
    };
    let output = std::process::Command::new("just")
        .arg("--justfile")
        .arg(&path)
        .arg("--working-directory")
        .arg(workspace_root)
        .args(["--dump", "--dump-format", "json"])
        .output()
        .map_err(|e| eyre!("{} exists, but running `just` failed: {e}", path.display()))?;
    if !output.status.success() {
        bail!(
            "just --dump {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let dump: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| eyre!("parsing `just --dump` of {}: {e}", path.display()))?;
    let mut names = BTreeSet::new();
    collect_recipes(&dump, "", &mut names);
    Ok(names
        .into_iter()
        .map(|name| Target {
            runner: Runner::Just,
            params: vec![("recipe", name.clone())],
            name,
        })
        .collect())
}

/// just matches `justfile` case-insensitively, dotted or not.
fn find_justfile(dir: &Path) -> Result<Option<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|e| eyre!("reading {}: {e}", dir.display()))?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            (name == "justfile" || name == ".justfile") && e.path().is_file()
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => bail!(
            "{} has more than one justfile ({}); just refuses that too",
            dir.display(),
            found
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn collect_recipes(module: &serde_json::Value, prefix: &str, out: &mut BTreeSet<String>) {
    if let Some(recipes) = module.get("recipes").and_then(serde_json::Value::as_object) {
        for (name, recipe) in recipes {
            let private = recipe
                .get("private")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !private && !name.starts_with('_') {
                out.insert(format!("{prefix}{name}"));
            }
        }
    }
    if let Some(modules) = module.get("modules").and_then(serde_json::Value::as_object) {
        for (name, sub) in modules {
            collect_recipes(sub, &format!("{prefix}{name}::"), out);
        }
    }
}

// ---------------------------------------------------------------------------
// nu

/// Every tracked `.nu` file with a `main` — a script, runnable as
/// `nu <file>`. A `.nu` file without one is a module that scripts `use`.
pub(super) fn nu(workspace_root: &Path, files: &[String]) -> Result<Vec<Target>> {
    let mut out = Vec::new();
    for file in files.iter().filter(|f| f.ends_with(".nu")) {
        let raw = match std::fs::read_to_string(workspace_root.join(file)) {
            Ok(raw) => raw,
            // Tracked, but deleted in the working tree.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => bail!("reading {file}: {e}"),
        };
        if raw.lines().any(defines_main) {
            out.push(Target {
                runner: Runner::Nu,
                name: file.clone(),
                params: vec![("script", file.clone())],
            });
        }
    }
    Ok(out)
}

/// `def main`, `def "main sub"`, `def --env main`, … at the start of a line.
fn defines_main(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("def ") else {
        return false;
    };
    rest.split_whitespace()
        .find(|word| !word.starts_with("--"))
        .is_some_and(|name| {
            let name = name.trim_matches(['"', '\'']);
            name == "main" || name.starts_with("main[")
        })
}

// ---------------------------------------------------------------------------
// Deploys

/// One build-and-deploy per app with a `[workload]`: the Tilt dev loop, run in
/// the cluster. Image inputs are the same resolution the Tiltfile and the nx
/// `container` target use; the manifest is the one `butler k8s gen` commits.
pub(super) fn deploys(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
) -> Result<Vec<Deploy>> {
    k8s::workload_apps(workspace_root, graph, root, overrides)?
        .into_iter()
        .map(|app| {
            let project = graph
                .projects
                .values()
                .find(|p| p.root == app.dir)
                .ok_or_else(|| eyre!("{} is not a discovered project", app.dir))?;
            let declared = overrides.get(&app.dir).and_then(|a| a.image.as_ref());
            let facts =
                container::resolve(root, app.kind, &app.dir, Some(&project.name), declared)?;
            for target in &facts.container_depends_on {
                if !project.targets.contains_key(target) {
                    bail!(
                        "{}: its image needs `{}:{target}` first, but the project has no \
                         `{target}` target",
                        app.dir,
                        project.name
                    );
                }
            }
            Ok(Deploy {
                image: k8s::image_ref(root, &app, &root.env),
                manifest: k8s::rendered_path(root, &app),
                project: project.name.clone(),
                prebuild: facts.container_depends_on,
                dockerfile: facts.file,
                context: facts.context,
                stage: facts.stage,
                build_args: facts.build_args,
                name: app.name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "butler-tekton-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    fn task_names(dir: &Path) -> Vec<String> {
        taskfile(dir)
            .expect("taskfile parses")
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    #[test]
    fn taskfile_names_follow_go_task_namespacing() {
        let dir = scratch();
        write(
            &dir,
            "Taskfile.yml",
            "version: '3'\n\
             includes:\n  \
               flat: { taskfile: ./tasks/flat.yml, flatten: true }\n  \
               ns: ./tasks/ns.yml\n  \
               hidden: { taskfile: ./tasks/hidden.yml, internal: true }\n  \
               picky: { taskfile: ./tasks/picky.yml, excludes: [skip] }\n  \
               gone: { taskfile: ./tasks/none.yml, optional: true }\n  \
               remote: { taskfile: https://example.com/Taskfile.yml }\n\
             tasks:\n  \
               root: echo\n  \
               private: { internal: true, cmds: [echo] }\n",
        );
        write(
            &dir,
            "tasks/flat.yml",
            "version: '3'\ntasks:\n  lint: echo\n",
        );
        write(
            &dir,
            "tasks/ns.yml",
            "version: '3'\n\
             includes:\n  deep: ./deep\n\
             tasks:\n  build: echo\n",
        );
        write(
            &dir,
            "tasks/deep/Taskfile.yml",
            "version: '3'\ntasks:\n  x: echo\n",
        );
        write(
            &dir,
            "tasks/hidden.yml",
            "version: '3'\ntasks:\n  h: echo\n",
        );
        write(
            &dir,
            "tasks/picky.yml",
            "version: '3'\ntasks:\n  keep: echo\n  skip: echo\n",
        );

        assert_eq!(
            task_names(&dir),
            ["lint", "ns:build", "ns:deep:x", "picky:keep", "root"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_flattened_name_clash_is_an_error() {
        let dir = scratch();
        write(
            &dir,
            "Taskfile.yml",
            "version: '3'\n\
             includes:\n  a: { taskfile: ./a.yml, flatten: true }\n\
             tasks:\n  lint: echo\n",
        );
        write(&dir, "a.yml", "version: '3'\ntasks:\n  lint: echo\n");
        let err = taskfile(&dir).expect_err("go-task rejects the clash too");
        assert!(err.to_string().contains("`lint` is defined twice"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_required_include_is_an_error() {
        let dir = scratch();
        write(
            &dir,
            "Taskfile.yml",
            "version: '3'\nincludes:\n  a: ./nope.yml\n",
        );
        assert!(taskfile(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nu_scripts_are_files_with_a_main() {
        for line in [
            "def main [] {",
            "def --env main [] {",
            "def \"main build\" [] {",
            "def 'main' [] {",
            "def main[] {",
        ] {
            assert!(defines_main(line), "{line}");
        }
        for line in [
            "def mainly [] {",
            "  def main [] {",
            "export def helper [] {",
        ] {
            assert!(!defines_main(line), "{line}");
        }
    }

    #[test]
    fn just_modules_and_private_recipes() {
        let dump = serde_json::json!({
            "recipes": {
                "build": {"private": false},
                "_helper": {"private": false},
                "secret": {"private": true},
            },
            "modules": {
                "db": {"recipes": {"migrate": {"private": false}}, "modules": {}},
            },
        });
        let mut out = BTreeSet::new();
        collect_recipes(&dump, "", &mut out);
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            ["build", "db::migrate"]
        );
    }
}
