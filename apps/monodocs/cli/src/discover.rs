//! Polyglot project discovery.
//!
//! A *project* is any directory holding a recognised manifest: `Cargo.toml` with a `[package]`
//! table (Rust), `package.json` (TypeScript/JavaScript), `kcl.mod` (KCL), or `nupm.nuon` / a bare
//! `*.nu` entry point (Nushell). Metadata comes from the manifest, task names from the Nx
//! `project.json` next to it, and prose from the project's `README.md`.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use eyre::{Context, Result};

/// Directory names never descended into.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".github",
    ".idea",
    ".vscode",
    ".nx",
    ".cargo",
    ".claude",
    "node_modules",
    "target",
    "dist",
    "src",
    "tests",
    "fixtures",
    "__pycache__",
    ".venv",
];

const MAX_DEPTH: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Rust,
    TypeScript,
    JavaScript,
    Kcl,
    Nushell,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::TypeScript => "TypeScript",
            Lang::JavaScript => "JavaScript",
            Lang::Kcl => "KCL",
            Lang::Nushell => "Nushell",
        }
    }

    /// CSS modifier class, also used as the fence language for inline snippets.
    pub fn slug(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::JavaScript => "javascript",
            Lang::Kcl => "kcl",
            Lang::Nushell => "nushell",
        }
    }
}

/// A markdown file pulled into the rendered document.
#[derive(Debug)]
pub struct DocFile {
    pub path: PathBuf,
    /// Path relative to the workspace root, for provenance in the output.
    pub rel: String,
    /// `None` for the project README, `Some(file stem)` for supplementary docs.
    pub extra_title: Option<String>,
}

#[derive(Debug)]
pub struct Project {
    pub name: String,
    pub slug: String,
    pub dir: PathBuf,
    pub rel_dir: String,
    pub lang: Lang,
    pub kind: String,
    pub group: String,
    pub group_rank: u8,
    pub version: Option<String>,
    pub description: Option<String>,
    pub docs: Vec<DocFile>,
    pub nx_targets: Vec<String>,
    /// Rustdoc crate directory name (`pg_gen`), when the project is a Rust crate.
    pub rustdoc_crate: Option<String>,
}

impl Project {
    /// True when the project has no prose to render.
    pub fn is_undocumented(&self) -> bool {
        self.docs.is_empty()
    }

    /// The synthetic entry for the repository root, which has no language of its own.
    pub fn is_workspace(&self) -> bool {
        self.rel_dir == "."
    }
}

/// Discover every project under `root`, plus a synthetic `workspace` entry for the root
/// `README.md` and `docs/*.md`. Sorted apps first, then libraries, then everything else.
pub fn discover(root: &Path) -> Result<Vec<Project>> {
    let mut projects = Vec::new();
    if let Some(workspace) = workspace_project(root)? {
        projects.push(workspace);
    }
    walk(root, root, 0, &mut projects)?;
    reassign_root_docs(&mut projects);
    projects.sort_by(|a, b| {
        (a.group_rank, &a.rel_dir)
            .cmp(&(b.group_rank, &b.rel_dir))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(projects)
}

/// Move root-level `docs/<project>.md` onto the project it documents (e.g. the generated
/// `docs/pg-cli.md` CLI reference belongs under `pg-cli`, not under the workspace).
fn reassign_root_docs(projects: &mut [Project]) {
    let Some(workspace) = projects.iter().position(|p| p.rel_dir == ".") else {
        return;
    };
    let owners: Vec<(usize, String)> = projects
        .iter()
        .enumerate()
        .filter(|(index, project)| *index != workspace && project.rel_dir != ".")
        .map(|(index, project)| (index, project.name.clone()))
        .collect();

    let mut moved: Vec<(usize, DocFile)> = Vec::new();
    projects[workspace].docs.retain(|doc| {
        let Some(stem) = doc.extra_title.as_deref() else {
            return true;
        };
        match owners.iter().find(|(_, name)| name == stem) {
            Some((index, _)) => {
                moved.push((
                    *index,
                    DocFile {
                        path: doc.path.clone(),
                        rel: doc.rel.clone(),
                        extra_title: doc.extra_title.clone(),
                    },
                ));
                false
            }
            None => true,
        }
    });
    for (index, doc) in moved {
        projects[index].docs.push(doc);
    }
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Project>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    if depth > 0
        && let Some(project) = project_at(root, dir)?
    {
        out.push(project);
    }

    let entries = fs::read_dir(dir).wrap_err_with(|| format!("reading {}", dir.display()))?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        walk(root, &child, depth + 1, out)?;
    }
    Ok(())
}

/// The root `README.md` plus `docs/*.md`, rendered as the document's opening section.
fn workspace_project(root: &Path) -> Result<Option<Project>> {
    let mut docs = Vec::new();
    if let Some(readme) = readme_in(root) {
        docs.push(doc_file(root, readme, None));
    }
    collect_extra_docs(root, root, &mut docs);
    if docs.is_empty() {
        return Ok(None);
    }
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".to_string());
    Ok(Some(Project {
        slug: slugify(&name),
        name,
        dir: root.to_path_buf(),
        rel_dir: ".".to_string(),
        lang: Lang::Rust,
        kind: "workspace".to_string(),
        group: "Workspace".to_string(),
        group_rank: 0,
        version: None,
        description: None,
        docs,
        nx_targets: Vec::new(),
        rustdoc_crate: None,
    }))
}

fn project_at(root: &Path, dir: &Path) -> Result<Option<Project>> {
    let cargo = dir.join("Cargo.toml");
    let package_json = dir.join("package.json");
    let kcl_mod = dir.join("kcl.mod");
    let nupm = dir.join("nupm.nuon");

    let mut project = if cargo.is_file() {
        rust_project(dir, &cargo)?
    } else if package_json.is_file() {
        node_project(dir, &package_json)?
    } else if kcl_mod.is_file() {
        kcl_project(dir, &kcl_mod)?
    } else if nupm.is_file() || has_nu_sources(dir) {
        nu_project(dir, nupm.is_file().then_some(nupm.as_path()))?
    } else {
        None
    };

    let Some(project) = project.take() else {
        return Ok(None);
    };
    Ok(Some(finish(root, dir, project)))
}

/// Partially built project; the shared fields are filled in by [`finish`].
struct Draft {
    name: String,
    lang: Lang,
    kind: String,
    version: Option<String>,
    description: Option<String>,
    rustdoc_crate: Option<String>,
}

fn finish(root: &Path, dir: &Path, draft: Draft) -> Project {
    let rel_dir = rel(root, dir);
    let (group, group_rank) = group_for(&rel_dir);
    let mut docs = Vec::new();
    if let Some(readme) = readme_in(dir) {
        docs.push(doc_file(root, readme, None));
    }
    collect_extra_docs(root, dir, &mut docs);
    Project {
        slug: slugify(&draft.name),
        name: draft.name,
        dir: dir.to_path_buf(),
        rel_dir,
        lang: draft.lang,
        kind: draft.kind,
        group,
        group_rank,
        version: draft.version,
        description: draft.description,
        docs,
        nx_targets: nx_targets(dir),
        rustdoc_crate: draft.rustdoc_crate,
    }
}

fn rust_project(dir: &Path, manifest: &Path) -> Result<Option<Draft>> {
    let text =
        fs::read_to_string(manifest).wrap_err_with(|| format!("reading {}", manifest.display()))?;
    let value: toml::Table = text
        .parse()
        .wrap_err_with(|| format!("parsing {}", manifest.display()))?;
    let Some(package) = value.get("package").and_then(toml::Value::as_table) else {
        // Virtual workspace manifest: not a project of its own.
        return Ok(None);
    };
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or("unnamed")
        .to_string();

    let has_bin = value.get("bin").is_some() || dir.join("src/main.rs").is_file();
    let has_lib = value.get("lib").is_some() || dir.join("src/lib.rs").is_file();
    let kind = match (has_bin, has_lib) {
        (true, true) => "binary + library",
        (true, false) => "binary",
        _ => "library",
    };

    Ok(Some(Draft {
        // `cargo doc` emits a page for binary targets too, so both kinds get a link.
        rustdoc_crate: Some(name.replace('-', "_")),
        name,
        lang: Lang::Rust,
        kind: kind.to_string(),
        version: str_field(package.get("version")),
        description: str_field(package.get("description")),
    }))
}

fn node_project(dir: &Path, manifest: &Path) -> Result<Option<Draft>> {
    let text =
        fs::read_to_string(manifest).wrap_err_with(|| format!("reading {}", manifest.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).wrap_err_with(|| format!("parsing {}", manifest.display()))?;
    // A workspace-root package.json that only pins tooling is not a documented project.
    if value.get("private").and_then(serde_json::Value::as_bool) == Some(true)
        && value.get("workspaces").is_some()
    {
        return Ok(None);
    }
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unnamed")
        .to_string();
    let typescript = dir.join("tsconfig.json").is_file() || has_extension(dir, "ts");
    let kind = if value.get("bin").is_some() {
        "binary"
    } else {
        "package"
    };
    Ok(Some(Draft {
        name,
        lang: if typescript {
            Lang::TypeScript
        } else {
            Lang::JavaScript
        },
        kind: kind.to_string(),
        version: value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        description: value
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        rustdoc_crate: None,
    }))
}

fn kcl_project(dir: &Path, manifest: &Path) -> Result<Option<Draft>> {
    let text =
        fs::read_to_string(manifest).wrap_err_with(|| format!("reading {}", manifest.display()))?;
    let value: toml::Table = text
        .parse()
        .wrap_err_with(|| format!("parsing {}", manifest.display()))?;
    let package = value.get("package").and_then(toml::Value::as_table);
    let name = package
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| dir_name(dir));
    Ok(Some(Draft {
        name,
        lang: Lang::Kcl,
        kind: "module".to_string(),
        version: package.and_then(|p| str_field(p.get("version"))),
        description: package.and_then(|p| str_field(p.get("description"))),
        rustdoc_crate: None,
    }))
}

fn nu_project(dir: &Path, manifest: Option<&Path>) -> Result<Option<Draft>> {
    // `nupm.nuon` is NUON, a JSON superset; only the flat string fields are needed here, so it is
    // scanned rather than parsed.
    let text = match manifest {
        Some(path) => {
            fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?
        }
        None => String::new(),
    };
    Ok(Some(Draft {
        name: nuon_field(&text, "name").unwrap_or_else(|| dir_name(dir)),
        lang: Lang::Nushell,
        kind: "module".to_string(),
        version: nuon_field(&text, "version"),
        description: nuon_field(&text, "description"),
        rustdoc_crate: None,
    }))
}

/// Pull `key: "value"` / `key = "value"` out of a NUON record.
fn nuon_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with(key))
        .and_then(|line| {
            let rest = line[key.len()..].trim_start();
            let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('='))?;
            let rest = rest.trim().trim_end_matches(',');
            Some(rest.trim_matches(['"', '\''].as_slice()).to_string())
        })
        .filter(|value| !value.is_empty())
}

fn has_nu_sources(dir: &Path) -> bool {
    has_extension(dir, "nu")
}

fn has_extension(dir: &Path, ext: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.path().extension() == Some(OsStr::new(ext))
            || entry.path().join("index").with_extension(ext).is_file()
    })
}

fn nx_targets(dir: &Path) -> Vec<String> {
    let path = dir.join("project.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut targets: Vec<String> = value
        .get("targets")
        .and_then(serde_json::Value::as_object)
        .map(|targets| targets.keys().cloned().collect())
        .unwrap_or_default();
    targets.sort();
    targets
}

fn readme_in(dir: &Path) -> Option<PathBuf> {
    ["README.md", "Readme.md", "readme.md"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// Markdown under `<dir>/docs`, one level deep, excluding anything already collected.
fn collect_extra_docs(root: &Path, dir: &Path, out: &mut Vec<DocFile>) {
    let docs_dir = dir.join("docs");
    let Ok(entries) = fs::read_dir(&docs_dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension() == Some(OsStr::new("md")))
        .collect();
    paths.sort();
    for path in paths {
        let title = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        out.push(doc_file(root, path, title));
    }
}

fn doc_file(root: &Path, path: PathBuf, extra_title: Option<String>) -> DocFile {
    DocFile {
        rel: rel(root, &path),
        path,
        extra_title,
    }
}

/// Group name and sort rank from the layout: `apps/…`, `libs/…`, any other top directory that
/// actually contains projects, and a flat `<root>/<project>` layout that has no grouping at all.
fn group_for(rel_dir: &str) -> (String, u8) {
    let mut parts = rel_dir.split('/');
    let first = parts.next().unwrap_or("");
    let nested = parts.next().is_some();
    match first {
        "." | "" => ("Workspace".to_string(), 0),
        "apps" => ("Apps".to_string(), 1),
        "libs" => ("Libraries".to_string(), 2),
        other if nested => (title_case(other), 3),
        _ => ("Projects".to_string(), 3),
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string())
}

fn str_field(value: Option<&toml::Value>) -> Option<String> {
    value.and_then(toml::Value::as_str).map(str::to_string)
}

/// Path relative to `root`, always with `/` separators.
pub fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Lowercase, hyphen-separated, HTML-id-safe form of `value`.
pub fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut dash = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_id_safe() {
        assert_eq!(slugify("pg-cli"), "pg-cli");
        assert_eq!(slugify("Resource Stats / Types"), "resource-stats-types");
        assert_eq!(slugify("@scope/pkg"), "scope-pkg");
    }

    #[test]
    fn nuon_fields_tolerate_both_separators() {
        let text = "{\n  name: \"my-nu-mod\",\n  version = '0.2.0'\n}";
        assert_eq!(nuon_field(text, "name").as_deref(), Some("my-nu-mod"));
        assert_eq!(nuon_field(text, "version").as_deref(), Some("0.2.0"));
        assert_eq!(nuon_field(text, "description"), None);
    }

    #[test]
    fn groups_rank_workspace_apps_then_libs() {
        assert_eq!(group_for(".").1, 0);
        assert_eq!(group_for("apps/clis/pg-cli").0, "Apps");
        assert_eq!(group_for("libs/pg-gen").1, 2);
        assert_eq!(group_for("packages/thing"), ("Packages".to_string(), 3));
        // A project directly under the root has no grouping directory to name.
        assert_eq!(group_for("kcl-config"), ("Projects".to_string(), 3));
    }
}
