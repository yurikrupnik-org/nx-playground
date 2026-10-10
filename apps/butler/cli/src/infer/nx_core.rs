//! The two project layers nx runs itself, after every nx.json plugin (nx
//! 23's "default plugins"): `package.json` (`nx/core/package-json`) and
//! `project.json` (`nx/core/project-json`), in that order.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, WrapErr, bail};
use serde::Deserialize;

use super::{Contribution, Ctx, Layer, Node, glob, normalize_root, to_project_name};
use crate::config::{Json, ProjectJson, TargetConfig};

/// The fields of a `package.json` nx reads.
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub private: Option<Json>,
    pub keywords: Vec<String>,
    pub workspaces: Option<Json>,
    pub scripts: BTreeMap<String, Json>,
    pub dependencies: BTreeMap<String, Json>,
    pub dev_dependencies: BTreeMap<String, Json>,
    pub peer_dependencies: BTreeMap<String, Json>,
    pub optional_dependencies: BTreeMap<String, Json>,
    pub exports: Option<Json>,
    pub main: Option<Json>,
    pub nx: Option<PackageNx>,
}

/// The `nx` property of a `package.json`: project configuration spelled
/// inline.
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct PackageNx {
    pub name: Option<String>,
    pub root: Option<String>,
    pub project_type: Option<String>,
    pub tags: Vec<String>,
    pub implicit_dependencies: Vec<String>,
    pub included_scripts: Option<Vec<String>>,
    pub targets: BTreeMap<String, TargetConfig>,
}

pub fn read_json<T: serde::de::DeserializeOwned>(ctx: &Ctx, file: &str) -> Result<T> {
    let raw = std::fs::read_to_string(ctx.workspace_root.join(file))
        .wrap_err_with(|| format!("reading {file}"))?;
    serde_json::from_str(&raw).wrap_err_with(|| format!("parsing {file}"))
}

fn dir_of(file: &str) -> String {
    normalize_root(file.rsplit_once('/').map_or(".", |(d, _)| d))
}

fn is_named(file: &str, name: &str) -> bool {
    file == name || file.ends_with(&format!("/{name}"))
}

/// A `package.json` nx turns into (part of) a project.
pub struct PackageFile {
    pub file: String,
    pub root: String,
    /// Matched by the package manager's workspace globs — which is what links
    /// it into `node_modules`, where JavaScript imports resolve it.
    pub in_workspaces: bool,
    pub json: PackageJson,
}

/// Package-manager workspace globs, as nx reads them
/// (`getGlobPatternsFromPackageManagerWorkspaces`): `package.json`
/// `workspaces` (list or `{packages}`), `pnpm-workspace.yaml` `packages`,
/// `lerna.json` `packages`; each made to name a `package.json`, and the root
/// manifest included only when it carries an `nx` property.
fn workspace_patterns(ctx: &Ctx) -> Result<Vec<String>> {
    let normalize = |p: &str| {
        let p = if p.ends_with("/package.json") {
            p.to_string()
        } else {
            format!("{}/package.json", p.trim_end_matches('/'))
        };
        p.strip_prefix("./").map(str::to_string).unwrap_or(p)
    };
    let mut patterns = Vec::new();
    if !ctx.workspace_root.join("package.json").is_file() {
        return Ok(patterns);
    }
    let root: PackageJson = read_json(ctx, "package.json")?;
    let listed = match &root.workspaces {
        Some(Json::Array(list)) => list.clone(),
        Some(Json::Object(o)) => o
            .get("packages")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    patterns.extend(listed.iter().filter_map(Json::as_str).map(normalize));
    let pnpm = ctx.workspace_root.join("pnpm-workspace.yaml");
    if pnpm.is_file() {
        let raw = std::fs::read_to_string(&pnpm)?;
        let doc: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&raw).wrap_err("parsing pnpm-workspace.yaml")?;
        if let Some(list) = doc.get("packages").and_then(|p| p.as_sequence()) {
            patterns.extend(list.iter().filter_map(|p| p.as_str()).map(normalize));
        }
    }
    if ctx.workspace_root.join("lerna.json").is_file() {
        let lerna: Json = read_json(ctx, "lerna.json")?;
        match lerna
            .get("packages")
            .and_then(Json::as_array)
            .filter(|l| !l.is_empty())
        {
            Some(list) => patterns.extend(list.iter().filter_map(Json::as_str).map(normalize)),
            None => patterns.push(normalize("packages/*")),
        }
    }
    if root.nx.is_some() {
        patterns.push("package.json".into());
    }
    Ok(patterns)
}

/// nx's `buildPackageJsonWorkspacesMatcher`: in a positive pattern and in no
/// negative one; only negatives means "every package.json but those".
fn workspace_matcher(patterns: &[String]) -> Result<impl Fn(&str) -> bool + use<>> {
    let (negative, mut positive): (Vec<&String>, Vec<&String>) =
        patterns.iter().partition(|p| p.starts_with('!'));
    let any_package = "**/package.json".to_string();
    if !negative.is_empty()
        && (positive.is_empty() || (positive.len() == 1 && positive[0] == "package.json"))
    {
        positive.push(&any_package);
    }
    let positive_exact: BTreeSet<String> = positive.iter().map(|p| (*p).clone()).collect();
    let negative_exact: BTreeSet<String> = negative.iter().map(|p| p[1..].to_string()).collect();
    let positive = positive
        .iter()
        .map(|p| glob(p))
        .collect::<Result<Vec<_>>>()?;
    let negative = negative
        .iter()
        .map(|p| glob(&p[1..]))
        .collect::<Result<Vec<_>>>()?;
    Ok(move |p: &str| {
        (positive_exact.contains(p) || positive.iter().any(|m| m.is_match(p)))
            && !negative_exact.contains(p)
            && !negative.iter().any(|m| m.is_match(p))
    })
}

/// Every `package.json` nx makes a project of: in the package manager's
/// workspaces, or beside a `project.json`.
pub fn package_files(ctx: &Ctx) -> Result<Vec<PackageFile>> {
    let in_workspaces = workspace_matcher(&workspace_patterns(ctx)?)?;
    let project_json_roots: BTreeSet<String> = ctx
        .files
        .iter()
        .filter(|f| is_named(f, "project.json"))
        .map(|f| dir_of(f))
        .collect();
    let mut out = Vec::new();
    for file in ctx.files.iter().filter(|f| is_named(f, "package.json")) {
        if !ctx.workspace_root.join(file).is_file() {
            continue;
        }
        let root = dir_of(file);
        let in_ws = in_workspaces(file);
        if !in_ws && !project_json_roots.contains(&root) {
            continue;
        }
        out.push(PackageFile {
            file: file.clone(),
            json: read_json(ctx, file)?,
            root,
            in_workspaces: in_ws,
        });
    }
    Ok(out)
}

/// `package.json` projects (`buildProjectConfigurationFromPackageJson`).
pub struct PackageJsonLayer;

impl Layer for PackageJsonLayer {
    fn name(&self) -> &str {
        "nx/core/package-json"
    }

    fn nodes(&self, ctx: &Ctx) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        for PackageFile {
            file, root, json, ..
        } in package_files(ctx)?
        {
            let mut scripts = json.scripts.clone();
            // A script a sibling project.json target runs differently (its own
            // command, another executor, or another script) is not inferred.
            let sibling = if root == "." {
                "project.json".to_string()
            } else {
                format!("{root}/project.json")
            };
            if ctx.workspace_root.join(&sibling).is_file() {
                let pj: Json = read_json(ctx, &sibling).unwrap_or(Json::Null);
                for (name, t) in pj
                    .get("targets")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    let executor = t
                        .get("executor")
                        .and_then(Json::as_str)
                        .filter(|e| !e.is_empty());
                    let script = t.pointer("/options/script");
                    let command = t.get("command").is_some_and(crate::config::truthy);
                    if command
                        || executor.is_some_and(|e| {
                            e != "nx:run-script" || script != Some(&Json::String(name.clone()))
                        })
                    {
                        scripts.remove(name);
                    }
                }
            }
            if root == "."
                && json.name.is_none()
                && json.nx.as_ref().is_none_or(|n| n.name.is_none())
            {
                bail!(
                    "{file}: nx requires the root package.json to specify a name if it is a project"
                );
            }
            let nx = json.nx.clone().unwrap_or_default();

            let mut targets: BTreeMap<String, TargetConfig> = BTreeMap::new();
            // `includedScripts` lists scripts verbatim, even ones a sibling
            // project.json overrides.
            let included = nx
                .included_scripts
                .clone()
                .unwrap_or_else(|| scripts.keys().cloned().collect());
            for script in included {
                let mut options = serde_json::Map::new();
                options.insert("script".into(), Json::String(script.clone()));
                targets.insert(
                    script,
                    TargetConfig {
                        executor: Some("nx:run-script".into()),
                        options: Some(options),
                        ..Default::default()
                    },
                );
            }
            for (name, t) in nx.targets {
                let merged = match targets.get(&name) {
                    // How to run it is spelled out: it replaces the script.
                    Some(_) if t.executor.is_some() || t.command.is_some() => t,
                    base => t
                        .desugar(&format!("{file} nx.targets.{name}"))?
                        .merged_over(base),
                };
                targets.insert(name, merged);
            }

            let mut tags = vec![
                if json.private.as_ref().is_some_and(crate::config::truthy) {
                    "npm:private".to_string()
                } else {
                    "npm:public".to_string()
                },
            ];
            tags.extend(json.keywords.iter().map(|k| format!("npm:{k}")));
            tags.extend(nx.tags.iter().cloned());

            let name = nx
                .name
                .clone()
                .or_else(|| json.name.clone())
                .unwrap_or_else(|| to_project_name(&file));
            let project_root = nx.root.as_deref().map_or(root, normalize_root);
            out.push(Node {
                file,
                projects: BTreeMap::from([(
                    project_root,
                    Contribution {
                        name: Some(name),
                        project_type: nx.project_type,
                        tags,
                        implicit_dependencies: nx.implicit_dependencies,
                        targets,
                    },
                )]),
            });
        }
        Ok(out)
    }
}

/// `project.json` files, which nx merges last so declared targets override
/// every inferred one.
pub struct ProjectJsonLayer;

impl Layer for ProjectJsonLayer {
    fn name(&self) -> &str {
        "nx/core/project-json"
    }

    fn nodes(&self, ctx: &Ctx) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        for file in ctx
            .files
            .iter()
            .filter(|f| is_named(f, "project.json") && ctx.workspace_root.join(f).is_file())
        {
            let pj: ProjectJson = read_json(ctx, file)?;
            let root = pj
                .root
                .as_deref()
                .map_or_else(|| dir_of(file), normalize_root);
            out.push(Node {
                file: file.clone(),
                projects: BTreeMap::from([(
                    root,
                    Contribution {
                        // Unnamed, it keeps the name an earlier layer gave
                        // (a sibling package.json's); only a project nothing
                        // named falls back to the directory name.
                        name: pj.name,
                        project_type: pj.project_type,
                        tags: pj.tags,
                        implicit_dependencies: pj.implicit_dependencies,
                        targets: pj.targets,
                    },
                )]),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(patterns: &[&str]) -> impl Fn(&str) -> bool {
        let owned: Vec<String> = patterns.iter().map(|p| (*p).to_string()).collect();
        workspace_matcher(&owned).unwrap()
    }

    #[test]
    fn workspace_globs_stay_within_their_segments() {
        let m = matcher(&["libs/ui/*/package.json", "apps/**/*/package.json"]);
        assert!(m("libs/ui/web-auth/package.json"));
        assert!(!m("libs/ui/web-auth/nested/package.json"));
        assert!(m("apps/todo/e2e/package.json"));
        assert!(!m("package.json"));
    }

    #[test]
    fn negative_only_workspaces_mean_everything_else() {
        let m = matcher(&["!legacy/*/package.json"]);
        assert!(m("apps/x/package.json"));
        assert!(!m("legacy/old/package.json"));
    }

    #[test]
    fn project_name_falls_back_to_the_directory_name() {
        assert_eq!(to_project_name("apps/Todo/Web/package.json"), "web");
    }
}
