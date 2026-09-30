//! Project-graph inference, in nx's own two-phase plugin shape.
//!
//! A [`Layer`] is what nx calls a plugin: `nodes` is `createNodesV2` (files in,
//! per-root contributions out) and `dependencies` is `createDependencies`
//! (the merged project set in, edges out). The graph is built the way nx 23
//! builds it (`mergeCreateNodesResults`): the repo's plugins (nx.json
//! `plugins`) merge first, then a synthetic layer made from nx.json
//! `targetDefaults` ([`target_defaults`]), then nx's own default plugins —
//! `package.json` and `project.json` ([`nx_core`]) — so a target default beats
//! what a plugin inferred while an explicit manifest beats the default. Every
//! merge follows nx's rules ([`TargetConfig::merged_over`]); the result is then
//! normalized like nx's `normalizeTarget`, and edges come last.
//!
//! Keeping nx's shape is what makes two things cheap: `butler graph verify`
//! can diff the result against `nx graph --file` field for field, and an
//! external inferrer (see [`external`]) speaks the exact JSON an nx plugin
//! already returns, so a repo's existing TypeScript plugin can feed butler
//! with a ten-line adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eyre::{Result, WrapErr, bail, eyre};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::config::{Json, JsonMap, TargetConfig, TargetDefault};
use crate::graph::{Edge, Project, ProjectGraph};
use crate::settings::{AppOverrides, Root};

pub mod external;
pub mod matching;
pub mod native;
pub mod nx_core;
pub mod nx_js;
pub mod target_defaults;

/// Everything a layer may read. `files` is the workspace file universe
/// (`git ls-files -co --exclude-standard`, workspace-relative, sorted) — the
/// same set nx hands its plugins, so gitignored trees never become projects.
pub struct Ctx<'a> {
    pub workspace_root: &'a Path,
    pub files: &'a [String],
    /// `butler.toml`, when the repo has one. Layers that need it (app
    /// inference) contribute nothing without it.
    pub settings: Option<&'a Root>,
    pub overrides: Option<&'a AppOverrides>,
}

/// One project root's contribution from one layer — an entry of nx's
/// `createNodesV2` result (`projects[root]`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Contribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implicit_dependencies: Vec<String>,
    pub targets: BTreeMap<String, TargetConfig>,
}

/// Contributions a layer derived from one matched file, keyed by project root.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Node {
    pub file: String,
    pub projects: BTreeMap<String, Contribution>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DepKind {
    Static,
    Dynamic,
    Implicit,
}

impl DepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
            Self::Implicit => "implicit",
        }
    }
}

/// A project-to-project edge — nx's `RawProjectGraphDependency`, plus the one
/// fact nx does not keep: whether the edge is a dev-only dependency. Image
/// build contexts (`tilt`) follow only non-dev edges.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub kind: DepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev: bool,
}

/// What `dependencies` sees: every project after all `nodes` merged —
/// nx's `context.projects` (name -> root).
pub type ProjectRoots = BTreeMap<String, String>;

pub trait Layer {
    /// Human name, for error context.
    fn name(&self) -> &str;
    fn nodes(&self, ctx: &Ctx) -> Result<Vec<Node>>;
    fn dependencies(&self, _ctx: &Ctx, _projects: &ProjectRoots) -> Result<Vec<Dependency>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Default, Clone)]
struct Draft {
    name: Option<String>,
    project_type: Option<String>,
    tags: Vec<String>,
    implicit_dependencies: Vec<String>,
    targets: BTreeMap<String, TargetConfig>,
}

/// Accumulates layer output in order, with nx's merge rules.
#[derive(Debug, Default, Clone)]
pub struct Builder {
    drafts: BTreeMap<String, Draft>,
    /// Every name a root has gone by, the latest owner winning — nx's name
    /// history. A plugin may reference a project by a name a later layer
    /// replaces (a crate named by its manifest, renamed by its
    /// `package.json`); nx rewrites such references to the final name.
    name_history: BTreeMap<String, String>,
}

impl Builder {
    /// Merge one layer's nodes (nx's `mergeProjectConfigurationIntoRootMap`):
    /// scalar fields are overwritten, tags are unioned, implicit dependencies
    /// concatenated, and each target merged over the one already there. A
    /// target key that is a glob merges onto every existing target it matches.
    pub fn apply(&mut self, layer: &str, nodes: Vec<Node>) -> Result<()> {
        for node in nodes {
            for (root, c) in node.projects {
                let root = normalize_root(&root);
                let draft = self.drafts.entry(root.clone()).or_default();
                if let Some(name) = c.name {
                    self.name_history.insert(name.clone(), root.clone());
                    draft.name = Some(name);
                }
                if c.project_type.is_some() {
                    draft.project_type = c.project_type;
                }
                for tag in c.tags {
                    if !draft.tags.contains(&tag) {
                        draft.tags.push(tag);
                    }
                }
                draft.implicit_dependencies.extend(c.implicit_dependencies);
                for (target_name, target) in c.targets {
                    let ctx = format!("{layer}: {root}:{target_name} (from {})", node.file);
                    let target = target.desugar(&ctx)?;
                    let mut names: Vec<String> = Vec::new();
                    if is_glob_pattern(&target_name) {
                        let m = glob(&target_name).wrap_err_with(|| ctx.clone())?;
                        names.extend(draft.targets.keys().filter(|n| m.is_match(n)).cloned());
                    }
                    if names.is_empty() {
                        names.push(target_name);
                    }
                    for name in names {
                        let merged = target.merged_over(draft.targets.get(&name));
                        draft.targets.insert(name, merged);
                    }
                }
            }
        }
        Ok(())
    }

    /// nx's `validateAndNormalizeProjectRootMap` + `normalizeTarget`, run once
    /// after every layer merged: a project no layer named takes its
    /// `project.json`'s directory name, else its `package.json` name;
    /// renamed-project references are rewritten; and every target gets
    /// what nx adds when it builds the graph — `{workspaceRoot}` /
    /// `{projectRoot}` / `{projectName}` resolved in options and
    /// configurations, `parallelism: true` unless set, the deprecated
    /// target-name `cache` fallback, `nx:noop` for an executor-less target that
    /// only has `dependsOn`, and removal of one that has neither.
    pub fn normalize(
        &mut self,
        workspace_root: &Path,
        target_defaults: &BTreeMap<String, TargetDefault>,
    ) -> Result<()> {
        for (root, d) in &mut self.drafts {
            if d.name.is_none() {
                d.name = fallback_name(workspace_root, root)?;
            }
            if d.name.is_none() {
                bail!("project at {root} has no name (no layer named it)");
            }
        }
        let final_names: BTreeMap<String, String> = self
            .drafts
            .iter()
            .map(|(root, d)| (root.clone(), d.name.clone().expect("named above")))
            .collect();
        let rename = |name: &str| -> Option<String> {
            let root = self.name_history.get(name)?;
            final_names.get(root).filter(|n| *n != name).cloned()
        };
        let mut renamed: BTreeMap<(String, String), TargetConfig> = BTreeMap::new();
        for (root, d) in &self.drafts {
            for (tname, t) in &d.targets {
                let mut t2 = t.clone();
                let changed = rename_refs(&mut t2, &d.targets, &rename);
                if changed {
                    renamed.insert((root.clone(), tname.clone()), t2);
                }
            }
        }
        for ((root, tname), t) in renamed {
            self.drafts
                .get_mut(&root)
                .expect("known root")
                .targets
                .insert(tname, t);
        }

        for (root, d) in &mut self.drafts {
            let name = d.name.clone().expect("named above");
            let mut invalid = Vec::new();
            d.targets.retain(|tname, t| {
                let at = format!("{root}:{tname}");
                match normalize_target(t, tname, root, &name, target_defaults, &at) {
                    Ok(keep) => keep,
                    Err(e) => {
                        invalid.push(format!("{e:#}"));
                        true
                    }
                }
            });
            if !invalid.is_empty() {
                bail!("{}", invalid.join("\n"));
            }
            for (tname, t) in &d.targets {
                if t.cache == Some(true) && t.continuous == Some(true) {
                    bail!(
                        "project {name}: \"{tname}\" has both \"cache\" and \"continuous\" set \
                         to true; continuous targets cannot be cached"
                    );
                }
            }
        }
        Ok(())
    }

    /// name -> root, for the `dependencies` phase. Fails on an unnamed or a
    /// duplicate-named project, as nx does.
    pub fn project_roots(&self) -> Result<ProjectRoots> {
        let mut out = ProjectRoots::new();
        for (root, d) in &self.drafts {
            let Some(name) = &d.name else {
                bail!("project at {root} has no name (no layer named it)");
            };
            if let Some(other) = out.insert(name.clone(), root.clone()) {
                bail!("duplicate project name `{name}` at {other} and {root}");
            }
        }
        Ok(out)
    }

    /// Assemble the graph from the merged projects and every layer's edges,
    /// the way nx's `ProjectGraphBuilder` does: `implicitDependencies` are
    /// expanded (names, globs, `tag:`, directories) into implicit edges, a
    /// `!name` entry removes every edge to that project, self-edges and edges
    /// to non-projects are dropped, and an edge is unique per kind.
    pub fn finish(self, dependencies: Vec<Dependency>) -> Result<ProjectGraph> {
        let roots = self.project_roots()?;
        let candidates: Vec<matching::Candidate> = self
            .drafts
            .iter()
            .map(|(root, d)| matching::Candidate {
                name: d.name.as_deref().expect("checked by project_roots"),
                root,
                tags: &d.tags,
            })
            .collect();
        let mut implicit: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for c in &candidates {
            let draft = &self.drafts[c.root];
            implicit.insert(
                c.name.to_string(),
                matching::normalize_implicit_dependencies(
                    c.name,
                    &draft.implicit_dependencies,
                    &candidates,
                )?,
            );
        }

        let mut graph = ProjectGraph::default();
        for (root, d) in self.drafts {
            let name = d.name.expect("checked by project_roots");
            graph.projects.insert(
                name.clone(),
                Project {
                    implicit_dependencies: implicit.remove(&name).unwrap_or_default(),
                    name,
                    root,
                    project_type: d.project_type,
                    tags: d.tags,
                    targets: d.targets,
                    deps: BTreeSet::new(),
                    build_deps: BTreeSet::new(),
                },
            );
        }

        let mut removed: BTreeSet<(String, String)> = BTreeSet::new();
        let mut edges: Vec<(Edge, bool)> = Vec::new();
        for p in graph.projects.values() {
            for dep in &p.implicit_dependencies {
                if let Some(target) = dep.strip_prefix('!') {
                    removed.insert((p.name.clone(), target.to_string()));
                } else {
                    let edge = Edge {
                        source: p.name.clone(),
                        target: dep.clone(),
                        kind: DepKind::Implicit,
                    };
                    edges.push((edge, false));
                }
            }
        }
        for dep in dependencies {
            if !roots.contains_key(&dep.source) {
                return Err(eyre!("edge from unknown project `{}`", dep.source));
            }
            let edge = Edge {
                source: dep.source,
                target: dep.target,
                kind: dep.kind,
            };
            edges.push((edge, dep.dev));
        }
        // One edge per (source, target, kind); it feeds a shipped artifact if
        // any layer reported it as a non-dev dependency.
        let mut dev_only: BTreeMap<Edge, bool> = BTreeMap::new();
        for (edge, dev) in edges {
            if edge.source == edge.target
                || !roots.contains_key(&edge.target)
                || removed.contains(&(edge.source.clone(), edge.target.clone()))
            {
                continue;
            }
            let slot = dev_only.entry(edge).or_insert(true);
            *slot &= dev;
        }
        for (edge, dev) in dev_only {
            let p = graph.projects.get_mut(&edge.source).expect("known source");
            p.deps.insert(edge.target.clone());
            if !dev {
                p.build_deps.insert(edge.target.clone());
            }
            graph.edges.insert(edge);
        }
        Ok(graph)
    }
}

/// Rewrite `dependsOn`/`inputs` references to a project a later layer
/// renamed (nx's `ProjectNameInNodePropsManager`). Returns whether anything
/// changed.
fn rename_refs(
    t: &mut TargetConfig,
    own_targets: &BTreeMap<String, TargetConfig>,
    rename: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let rename_projects = |entry: &mut JsonMap, skip: &[&str]| -> bool {
        let mut changed = false;
        match entry.get_mut("projects") {
            Some(Json::String(p)) if !skip.contains(&p.as_str()) => {
                if let Some(n) = rename(p) {
                    *p = n;
                    changed = true;
                }
            }
            Some(Json::Array(list)) => {
                for p in list.iter_mut() {
                    if let Json::String(s) = p
                        && !is_glob_pattern(s)
                        && let Some(n) = rename(s)
                    {
                        *s = n;
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        changed
    };
    let mut changed = false;
    for input in t.inputs.iter_mut().flatten() {
        if let Json::Object(o) = input {
            changed |= rename_projects(o, &["self", "dependencies"]);
        }
    }
    for dep in t.depends_on.iter_mut().flatten() {
        match dep {
            Json::Object(o) => changed |= rename_projects(o, &["*", "self", "dependencies"]),
            Json::String(s) if !s.starts_with('^') && !own_targets.contains_key(s.as_str()) => {
                if let Some((project, target)) = s.split_once(':')
                    && let Some(n) = rename(project)
                {
                    *s = format!("{n}:{target}");
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

/// One target through nx's `normalizeTarget` and the per-target checks of
/// `normalizeTargets`. Returns `false` when nx would drop the target.
fn normalize_target(
    t: &mut TargetConfig,
    target_name: &str,
    root: &str,
    project_name: &str,
    target_defaults: &BTreeMap<String, TargetDefault>,
    at: &str,
) -> Result<bool> {
    *t = std::mem::take(t).desugar(at)?;
    let options = t.options.take().unwrap_or_default();
    t.options = Some(resolve_tokens(options, root, project_name, at)?);
    let mut configurations = t.configurations.take().unwrap_or_default();
    for cfg in configurations.values_mut() {
        *cfg = resolve_tokens(std::mem::take(cfg), root, project_name, at)?;
    }
    t.configurations = Some(configurations);
    t.parallelism.get_or_insert(true);

    // Deprecated since nx 23 (removed in 24): a target whose cache is decided
    // by nothing, but whose executor key shadowed the target-name key that
    // declares `cache: true`, is cached anyway.
    let long_running = t.continuous == Some(true)
        || target_name.ends_with(":watch")
        || target_name.ends_with("-watch")
        || matches!(target_name, "serve" | "dev" | "start");
    if t.cache.is_none()
        && !long_running
        && t.executor
            .as_ref()
            .is_some_and(|e| target_defaults.contains_key(e))
        && target_defaults.get(target_name).is_some_and(|td| {
            let mut declared = None;
            for entry in td.entries() {
                if let Some(cache) = entry.target.cache {
                    if entry.filter.is_some() {
                        return false;
                    }
                    declared = Some(cache);
                }
            }
            declared == Some(true)
        })
    {
        t.cache = Some(true);
    }

    let runs_something = t.executor.as_deref().is_some_and(|e| !e.is_empty())
        || t.command.as_deref().is_some_and(|c| !c.is_empty());
    if !runs_something {
        if t.depends_on.as_ref().is_some_and(|d| !d.is_empty()) {
            t.executor = Some("nx:noop".into());
        } else {
            return Ok(false);
        }
    }
    Ok(true)
}

/// nx's `resolveNxTokensInOptions`: in every string, a leading
/// `{workspaceRoot}` (with its slash) is removed — butler, like nx, runs
/// commands from the workspace root — `{projectRoot}` and `{projectName}` are
/// substituted, and `{workspaceRoot}` anywhere else is an error.
fn resolve_tokens(map: JsonMap, root: &str, name: &str, at: &str) -> Result<JsonMap> {
    fn value(v: Json, root: &str, name: &str, at: &str) -> Result<Json> {
        Ok(match v {
            Json::String(s) => {
                let s = s
                    .strip_prefix("{workspaceRoot}/")
                    .or_else(|| s.strip_prefix("{workspaceRoot}"))
                    .unwrap_or(&s);
                if s.contains("{workspaceRoot}") {
                    bail!(
                        "{at}: the {{workspaceRoot}} token is only valid at the beginning of an option"
                    );
                }
                Json::String(
                    s.replace("{projectRoot}", root)
                        .replace("{projectName}", name),
                )
            }
            Json::Array(list) => Json::Array(
                list.into_iter()
                    .map(|x| value(x, root, name, at))
                    .collect::<Result<_>>()?,
            ),
            Json::Object(m) => Json::Object(resolve_tokens(m, root, name, at)?),
            other => other,
        })
    }
    map.into_iter()
        .map(|(k, v)| Ok((k, value(v, root, name, at)?)))
        .collect()
}

/// The name nx gives a project no layer named: a `project.json` at its root
/// makes it the directory's name (that file named nothing, or it would have
/// named the project), otherwise its `package.json` name
/// (`validateProject`).
fn fallback_name(workspace_root: &Path, root: &str) -> Result<Option<String>> {
    if workspace_root.join(root).join("project.json").is_file() {
        return Ok(Some(to_project_name(&format!("{root}/project.json"))));
    }
    let path = workspace_root.join(root).join("package.json");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let json: Json =
        serde_json::from_str(&raw).wrap_err_with(|| format!("parsing {}", path.display()))?;
    Ok(json.get("name").and_then(Json::as_str).map(str::to_string))
}

/// `./apps/x/` and `apps/x` are one root; the workspace root is `.`.
pub fn normalize_root(root: &str) -> String {
    let r = root.trim_start_matches("./").trim_end_matches('/');
    if r.is_empty() { ".".into() } else { r.into() }
}

/// nx's `toProjectName`: the lower-cased name of the directory holding a
/// config file — the name nx gives a project whose manifest names none.
pub fn to_project_name(file: &str) -> String {
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    dir.rsplit('/').next().unwrap_or("").to_lowercase()
}

/// nx's `isGlobPattern`.
pub fn is_glob_pattern(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c, '*' | '|' | '{' | '}' | '(' | ')' | '['))
}

/// A minimatch-style glob: `*` stays within one path segment, `**` crosses
/// them. (minimatch extglobs such as `+(a|b)` are not supported.)
pub fn glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .wrap_err_with(|| format!("invalid glob `{pattern}`"))?
        .compile_matcher())
}

/// Run `plugins` (nx.json `plugins`, in order) and nx's own layers over `ctx`
/// and build the graph, in nx 23's order: plugins, then `targetDefaults`,
/// then `package.json` and `project.json`; edges from every layer, nx's
/// JavaScript dependency analysis ([`nx_js`]) included.
pub fn build(
    ctx: &Ctx,
    plugins: &[&dyn Layer],
    target_defaults: &BTreeMap<String, TargetDefault>,
) -> Result<ProjectGraph> {
    let defaults: [&dyn Layer; 3] = [
        &nx_js::JsLayer,
        &nx_core::PackageJsonLayer,
        &nx_core::ProjectJsonLayer,
    ];
    let run = |layer: &&dyn Layer| -> Result<(String, Vec<Node>)> {
        let nodes = layer
            .nodes(ctx)
            .map_err(|e| e.wrap_err(format!("inference layer `{}`", layer.name())))?;
        Ok((layer.name().to_string(), nodes))
    };
    let plugin_nodes = plugins.iter().map(run).collect::<Result<Vec<_>>>()?;
    let default_nodes = defaults.iter().map(run).collect::<Result<Vec<_>>>()?;

    let mut builder = Builder::default();
    for (name, nodes) in plugin_nodes {
        builder.apply(&name, nodes)?;
    }
    if !target_defaults.is_empty() {
        // The synthetic layer needs the shape nx's own layers will give each
        // target (its eventual executor/command), so stage them alone first.
        let mut staged = Builder::default();
        for (name, nodes) in &default_nodes {
            staged.apply(name, nodes.clone())?;
        }
        let synthetic = target_defaults::synthesize(&builder, &staged, target_defaults)?;
        builder.apply("nx.json targetDefaults", synthetic)?;
    }
    for (name, nodes) in default_nodes {
        builder.apply(&name, nodes)?;
    }
    builder.normalize(ctx.workspace_root, target_defaults)?;

    let roots = builder.project_roots()?;
    let mut deps = Vec::new();
    for layer in plugins.iter().chain(defaults.iter()) {
        deps.extend(layer.dependencies(ctx, &roots).map_err(|e| {
            e.wrap_err(format!("inference layer `{}` (dependencies)", layer.name()))
        })?);
    }
    builder.finish(deps)
}
