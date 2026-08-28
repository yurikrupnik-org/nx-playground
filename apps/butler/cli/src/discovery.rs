//! Workspace discovery: cargo workspace members, package.json workspaces, and
//! standalone project.json projects, merged into one project set.
//!
//! Precedence per project root when several sources describe the same root:
//! package.json scripts < inferred cargo targets < project.json targets
//! (per-target overlay). The project name follows the same order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{bail, eyre, Result, WrapErr};
use globset::{Glob, GlobSetBuilder};
use serde::Deserialize;

use crate::config::{ProjectJson, RunOptions, TargetConfig};
use crate::graph::{Project, ProjectGraph};

/// Directories never traversed while scanning for project files.
const PRUNED: &[&str] = &["node_modules", ".git", "dist", "target", ".nx", ".venv"];

pub fn find_workspace_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("nx.json").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("no nx.json found in the current directory or any parent");
        }
    }
}

pub fn discover(root: &Path) -> Result<ProjectGraph> {
    // Keyed by workspace-relative project root.
    let mut by_root: BTreeMap<String, Project> = BTreeMap::new();
    // npm package name -> project root (for workspace dependency edges).
    let mut npm_names: BTreeMap<String, String> = BTreeMap::new();
    // node project root -> declared dependency package names.
    let mut npm_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    discover_cargo(root, &mut by_root)?;
    discover_node(root, &mut by_root, &mut npm_names, &mut npm_deps)?;
    overlay_project_json(root, &mut by_root)?;

    // Resolve node dependency edges now that names are final.
    for (proj_root, dep_names) in &npm_deps {
        let dep_roots: Vec<String> = dep_names
            .iter()
            .filter_map(|n| npm_names.get(n))
            .filter(|dep_root| *dep_root != proj_root)
            .cloned()
            .collect();
        let dep_project_names: Vec<String> = dep_roots
            .iter()
            .filter_map(|r| by_root.get(r))
            .map(|p| p.name.clone())
            .collect();
        if let Some(p) = by_root.get_mut(proj_root) {
            p.deps.extend(dep_project_names);
        }
    }

    let mut graph = ProjectGraph::default();
    for (_, p) in by_root {
        if let Some(existing) = graph.projects.get(&p.name) {
            bail!(
                "duplicate project name `{}` at {} and {}",
                p.name,
                existing.root,
                p.root
            );
        }
        graph.projects.insert(p.name.clone(), p);
    }
    Ok(graph)
}

// ---------------------------------------------------------------------------
// Cargo

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    manifest_path: PathBuf,
    targets: Vec<CargoTarget>,
    dependencies: Vec<CargoDependency>,
}

#[derive(Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct CargoDependency {
    name: String,
    path: Option<PathBuf>,
}

fn discover_cargo(root: &Path, by_root: &mut BTreeMap<String, Project>) -> Result<()> {
    if !root.join("Cargo.toml").exists() {
        return Ok(());
    }
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .wrap_err("running cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let meta: CargoMetadata =
        serde_json::from_slice(&out.stdout).wrap_err("parsing cargo metadata")?;

    // Manifest dir -> package name, for resolving path dependencies.
    let mut root_to_name: BTreeMap<String, String> = BTreeMap::new();
    for pkg in &meta.packages {
        root_to_name.insert(relative_dir(root, &pkg.manifest_path)?, pkg.name.clone());
    }

    for pkg in &meta.packages {
        let proj_root = relative_dir(root, &pkg.manifest_path)?;
        let is_app = pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "bin"));

        let mut deps = BTreeSet::new();
        for dep in &pkg.dependencies {
            let Some(path) = &dep.path else { continue };
            let rel = pathdiff_rel(root, path)?;
            if let Some(name) = root_to_name.get(&rel) {
                deps.insert(name.clone());
            } else {
                // Path dep outside the workspace member set; keep by name if it
                // happens to be a member under a renamed key.
                let _ = &dep.name;
            }
        }

        let mut targets = BTreeMap::new();
        let build_cmd = if is_app { "build" } else { "check" };
        targets.insert(
            "build".into(),
            cargo_target(&format!("cargo {build_cmd} --package {}", pkg.name)),
        );
        targets.insert(
            "test".into(),
            cargo_target(&format!("cargo test --package {}", pkg.name)),
        );
        targets.insert(
            "lint".into(),
            cargo_target(&format!("cargo clippy --package {}", pkg.name)),
        );

        by_root.insert(
            proj_root.clone(),
            Project {
                name: pkg.name.clone(),
                root: proj_root,
                project_type: Some(if is_app { "application" } else { "library" }.into()),
                tags: vec![],
                targets,
                deps,
            },
        );
    }
    Ok(())
}

fn cargo_target(command: &str) -> TargetConfig {
    TargetConfig {
        executor: Some("butler:cargo".into()),
        options: RunOptions {
            command: Some(command.into()),
            cwd: Some("{workspaceRoot}".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Node (package.json workspaces)

#[derive(Deserialize, Default)]
struct PackageJson {
    name: Option<String>,
    #[serde(default)]
    workspaces: Vec<String>,
    #[serde(default)]
    scripts: BTreeMap<String, String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: BTreeMap<String, String>,
}

fn discover_node(
    root: &Path,
    by_root: &mut BTreeMap<String, Project>,
    npm_names: &mut BTreeMap<String, String>,
    npm_deps: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let root_pkg_path = root.join("package.json");
    if !root_pkg_path.exists() {
        return Ok(());
    }
    let root_pkg: PackageJson = serde_json::from_str(&std::fs::read_to_string(&root_pkg_path)?)
        .wrap_err("parsing root package.json")?;
    if root_pkg.workspaces.is_empty() {
        return Ok(());
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in &root_pkg.workspaces {
        builder.add(Glob::new(pattern).wrap_err_with(|| format!("workspaces glob `{pattern}`"))?);
    }
    let workspaces = builder.build()?;

    for dir in walk_dirs(root) {
        let rel = pathdiff_rel(root, &dir)?;
        if rel == "." || !workspaces.is_match(&rel) {
            continue;
        }
        let pkg_path = dir.join("package.json");
        if !pkg_path.exists() {
            continue;
        }
        let pkg: PackageJson = serde_json::from_str(&std::fs::read_to_string(&pkg_path)?)
            .wrap_err_with(|| format!("parsing {}", pkg_path.display()))?;

        if let Some(name) = &pkg.name {
            npm_names.insert(name.clone(), rel.clone());
        }
        let deps: BTreeSet<String> = pkg
            .dependencies
            .keys()
            .chain(pkg.dev_dependencies.keys())
            .cloned()
            .collect();
        npm_deps.insert(rel.clone(), deps);

        let mut targets = BTreeMap::new();
        for script in pkg.scripts.keys() {
            targets.insert(
                script.clone(),
                TargetConfig {
                    executor: Some("butler:script".into()),
                    options: RunOptions {
                        command: Some(format!("bun run {script}")),
                        cwd: Some("{projectRoot}".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }

        match by_root.get_mut(&rel) {
            Some(existing) => {
                // Cargo project at the same root: keep its name, add script
                // targets that don't collide with inferred cargo ones.
                for (name, t) in targets {
                    existing.targets.entry(name).or_insert(t);
                }
            }
            None => {
                let name = pkg.name.clone().unwrap_or_else(|| rel.replace('/', "-"));
                by_root.insert(
                    rel.clone(),
                    Project {
                        name,
                        root: rel.clone(),
                        project_type: None,
                        tags: vec![],
                        targets,
                        deps: BTreeSet::new(),
                    },
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// project.json overlay

fn overlay_project_json(root: &Path, by_root: &mut BTreeMap<String, Project>) -> Result<()> {
    for dir in walk_dirs(root) {
        let pj_path = dir.join("project.json");
        if !pj_path.exists() {
            continue;
        }
        let rel = pathdiff_rel(root, &dir)?;
        if rel == "." {
            continue;
        }
        let pj = ProjectJson::load(&pj_path)?;

        let project = by_root.entry(rel.clone()).or_insert_with(|| Project {
            name: rel.replace('/', "-"),
            root: rel.clone(),
            project_type: None,
            tags: vec![],
            targets: BTreeMap::new(),
            deps: BTreeSet::new(),
        });
        if let Some(name) = pj.name {
            project.name = name;
        }
        if pj.project_type.is_some() {
            project.project_type = pj.project_type;
        }
        project.tags = pj.tags;
        for (name, target) in pj.targets {
            project.targets.insert(name, target);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers

/// All directories under `root`, pruned of vendored/build trees.
fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        out.push(dir.clone());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || PRUNED.contains(&name.as_ref()) {
                continue;
            }
            stack.push(path);
        }
    }
    out
}

fn relative_dir(root: &Path, manifest: &Path) -> Result<String> {
    let dir = manifest
        .parent()
        .ok_or_else(|| eyre!("manifest without parent: {}", manifest.display()))?;
    pathdiff_rel(root, dir)
}

fn pathdiff_rel(root: &Path, path: &Path) -> Result<String> {
    let canonical = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let rel = canonical
        .strip_prefix(root)
        .map_err(|_| eyre!("{} is outside the workspace", path.display()))?;
    let s = rel.to_string_lossy().replace('\\', "/");
    Ok(if s.is_empty() { ".".into() } else { s })
}
