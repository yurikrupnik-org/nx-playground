//! The task graph: a port of nx 23's `createTaskGraph`
//! (`tasks-runner/create-task-graph.js` + `utils.js`) and of what nx decides
//! per task before running it — the configuration, the overrides, the
//! outputs, the environment — ending in the executor plan.
//!
//! Faithfulness matters more here than anywhere: `dependsOn` decides what
//! runs before what, and a runner that orders or selects tasks differently
//! from nx is not an alternative to it. In particular a `^target` dependency
//! walks *through* projects that lack the target (nx's "dummy" tasks), and a
//! dependency task inherits CLI overrides only when its `dependsOn` entry
//! says `params: "forward"`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;

use eyre::{Result, bail, eyre};

use crate::config::{Json, JsonMap, TargetConfig};
use crate::executor::{self, Plan, PlanCtx};
use crate::graph::{Project, ProjectGraph};
use crate::infer::matching::{Candidate, find_matching_projects};
use crate::infer::{glob, is_glob_pattern};

/// nx `findMatchingProjects` over the whole graph: names, globs, `tag:`,
/// `directory:`, `!` exclusions.
pub fn find_matching(graph: &ProjectGraph, patterns: &[String]) -> Result<Vec<String>> {
    let candidates: Vec<Candidate> = graph
        .projects
        .values()
        .map(|p| Candidate {
            name: &p.name,
            root: &p.root,
            tags: &p.tags,
        })
        .collect();
    find_matching_projects(patterns, &candidates)
}

/// nx's task id: `project:target[:configuration]`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId {
    pub project: String,
    pub target: String,
    pub configuration: Option<String>,
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.project, self.target)?;
        if let Some(c) = &self.configuration {
            write!(f, ":{c}")?;
        }
        Ok(())
    }
}

/// CLI overrides as nx carries them per task: the parsed map plus the raw
/// args (`__overrides_unparsed__`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    pub map: JsonMap,
    pub unparsed: Vec<String>,
}

impl Overrides {
    /// nx `createOverrides`.
    pub fn from_args(args: &[String]) -> Self {
        let mut map = executor::args::parse(args, executor::args::OVERRIDES);
        if map
            .get("_")
            .and_then(Json::as_array)
            .is_some_and(Vec::is_empty)
        {
            map.remove("_");
        }
        Self {
            map,
            unparsed: args.to_vec(),
        }
    }
}

/// A target after nx's per-task decisions — everything the runner and the
/// hasher need.
#[derive(Clone)]
pub struct ResolvedTarget {
    pub executor: String,
    pub plan: Plan,
    pub cache: bool,
    /// Raw nx input entries; `None` = nx's default inputs.
    pub inputs: Option<Vec<Json>>,
    /// Workspace-relative output paths/globs (nx `getOutputs`).
    pub outputs: Vec<String>,
}

#[derive(Clone)]
pub struct Task {
    pub id: TaskId,
    pub resolved: ResolvedTarget,
    /// Tasks that must finish first.
    pub deps: BTreeSet<TaskId>,
    /// The environment the task's processes get (dotenv files + nx's
    /// `NX_TASK_*` variables over butler's environment).
    pub env: Arc<BTreeMap<String, String>>,
    /// Overrides as the executor saw them (hashed, like nx's `hashCommand`).
    pub overrides: Overrides,
}

pub struct Request<'a> {
    pub projects: &'a [String],
    pub targets: &'a [String],
    pub configuration: Option<&'a str>,
    pub overrides: &'a Overrides,
    /// `--exclude-task-dependencies`: run only the requested tasks.
    pub exclude_task_dependencies: bool,
}

const DUMMY_TASK_TARGET: &str = "__nx_dummy_task__";

/// Build the task graph for `req` and resolve every task, returned in a
/// dependency-respecting order.
pub fn build(
    root: &Path,
    graph: &ProjectGraph,
    base_env: &BTreeMap<String, String>,
    req: &Request<'_>,
) -> Result<Vec<Task>> {
    let mut b = Builder::new(graph);
    b.process_tasks(req)?;

    let mut tasks = Vec::with_capacity(b.tasks.len());
    for (key, spec) in &b.tasks {
        let project = graph.get(&spec.id.project)?;
        let target = &project.targets[&spec.id.target];
        if target.continuous == Some(true) {
            bail!(
                "{}: continuous (long-running) tasks are not supported by butler; run it through nx",
                spec.id
            );
        }
        let env = Arc::new(task_env(root, base_env, project, &spec.id)?);
        let resolved = resolve(
            root,
            graph,
            project,
            &spec.id.target,
            target,
            spec.id.configuration.as_deref(),
            &spec.overrides,
            &env,
        )?;
        let deps = b.dependencies[key]
            .iter()
            .map(|d| b.tasks[d].id.clone())
            .collect();
        tasks.push(Task {
            id: spec.id.clone(),
            resolved,
            deps,
            env,
            overrides: spec.overrides.clone(),
        });
    }
    topo_sort(tasks)
}

/// The per-task environment: dotenv files (nx `getTaskSpecificEnv`), then
/// nx's own variables over everything (`getNxEnvVariablesForTask`, minus the
/// hash, which the runner adds once it is known).
fn task_env(
    root: &Path,
    base: &BTreeMap<String, String>,
    project: &Project,
    id: &TaskId,
) -> Result<BTreeMap<String, String>> {
    let mut env = executor::env::task_env(
        root,
        base,
        &project.root,
        &id.target,
        id.configuration.as_deref(),
    )?;
    let force_color = base
        .get("FORCE_COLOR")
        .cloned()
        .unwrap_or_else(|| "true".into());
    env.insert("FORCE_COLOR".into(), force_color);
    env.insert(
        "NX_WORKSPACE_ROOT".into(),
        root.to_string_lossy().into_owned(),
    );
    env.insert("NX_TASK_TARGET_PROJECT".into(), id.project.clone());
    env.insert("NX_TASK_TARGET_TARGET".into(), id.target.clone());
    match &id.configuration {
        Some(c) => env.insert("NX_TASK_TARGET_CONFIGURATION".into(), c.clone()),
        None => env.remove("NX_TASK_TARGET_CONFIGURATION"),
    };
    env.insert("LERNA_PACKAGE_NAME".into(), id.project.clone());
    env.insert("NX_TUI".into(), "false".into());
    if env.get("NX_LOAD_DOT_ENV_FILES").map(String::as_str) != Some("false") {
        env.insert("NX_LOAD_DOT_ENV_FILES".into(), "true".into());
    }
    env.remove("NX_SET_CLI");
    Ok(env)
}

/// Resolve one task: nx's configuration merge, output interpolation, and the
/// executor plan.
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    root: &Path,
    graph: &ProjectGraph,
    project: &Project,
    target_name: &str,
    target: &TargetConfig,
    configuration: Option<&str>,
    overrides: &Overrides,
    env: &BTreeMap<String, String>,
) -> Result<ResolvedTarget> {
    let task = format!("{}:{target_name}", project.name);
    let executor = target
        .executor
        .clone()
        .ok_or_else(|| eyre!("Target \"{task}\" does not have an executor configured"))?;

    let mut options = target.options.clone().unwrap_or_default();
    if let Some(c) = configuration
        && let Some(cfg) = target.configurations.as_ref().and_then(|m| m.get(c))
    {
        options.extend(cfg.clone());
    }
    let overrides = Overrides {
        map: interpolate_overrides(&overrides.map, project)?,
        unparsed: overrides.unparsed.clone(),
    };

    let plan = if executor == "nx:noop" {
        // nx's aggregator executor: runs nothing, exists for its dependsOn.
        Plan {
            steps: Vec::new(),
            parallel: false,
        }
    } else {
        executor::plan(
            &executor,
            &PlanCtx {
                workspace_root: root,
                graph,
                project,
                target: target_name,
                configuration,
                options: &options,
                overrides: &overrides.map,
                unparsed: &overrides.unparsed,
                env,
            },
        )?
    };

    let mut output_options = options.clone();
    output_options.extend(overrides.map.clone());
    let outputs =
        outputs(project, target_name, target, &output_options).map_err(|e| eyre!("{task}: {e}"))?;
    if target.cache == Some(true)
        && let Some(negated) = outputs.iter().find(|o| o.starts_with('!'))
    {
        bail!(
            "{task}: negated output `{negated}` is not supported by butler's cache; run it through nx"
        );
    }

    Ok(ResolvedTarget {
        executor,
        plan,
        cache: target.cache.unwrap_or(false),
        inputs: target.inputs.clone(),
        outputs,
    })
}

/// nx `resolveConfiguration`: the requested configuration if the target has
/// it, else the target's `defaultConfiguration`.
pub fn resolve_configuration(target: &TargetConfig, requested: Option<&str>) -> Option<String> {
    let default = target.default_configuration.clone();
    let wanted = requested.map(str::to_string).or_else(|| default.clone());
    match wanted {
        Some(c)
            if target
                .configurations
                .as_ref()
                .is_some_and(|m| m.contains_key(&c)) =>
        {
            Some(c)
        }
        _ => default,
    }
}

/// One normalized `dependsOn` entry (nx `normalizeDependencyConfigDefinition`).
#[derive(Debug, Clone, PartialEq)]
struct DepConfig {
    target: String,
    /// Resolved project names; `None` means "the dependencies of the
    /// project".
    projects: Option<Vec<String>>,
    params_forward: bool,
    options_forward: bool,
}

#[derive(Clone)]
struct Spec {
    id: TaskId,
    overrides: Overrides,
}

/// A task being processed; dummy tasks share the source task's project and
/// target under their own key.
#[derive(Clone)]
struct Node {
    key: String,
    project: String,
    target: String,
    configuration: Option<String>,
}

struct Builder<'a> {
    graph: &'a ProjectGraph,
    all_target_names: Vec<String>,
    seen: HashSet<String>,
    tasks: BTreeMap<String, Spec>,
    dependencies: BTreeMap<String, Vec<String>>,
}

impl<'a> Builder<'a> {
    fn new(graph: &'a ProjectGraph) -> Self {
        let all: BTreeSet<String> = graph
            .projects
            .values()
            .flat_map(|p| p.targets.keys().cloned())
            .collect();
        Self {
            graph,
            all_target_names: all.into_iter().collect(),
            seen: HashSet::new(),
            tasks: BTreeMap::new(),
            dependencies: BTreeMap::new(),
        }
    }

    /// `processTasks`.
    fn process_tasks(&mut self, req: &Request<'_>) -> Result<()> {
        for project_name in req.projects {
            let project = self.graph.get(project_name)?;
            for target in req.targets {
                if req.targets.len() == 1 || project.targets.contains_key(target) {
                    let config = project.targets.get(target).ok_or_else(|| {
                        eyre!("Cannot find configuration for task {project_name}:{target}")
                    })?;
                    let resolved = resolve_configuration(config, req.configuration);
                    let node =
                        self.create_task(project, target, resolved, req.overrides.clone())?;
                    self.dependencies.entry(node.key).or_default();
                }
            }
        }
        let initial: BTreeSet<String> = self.tasks.keys().cloned().collect();
        let roots: Vec<Node> = self
            .tasks
            .iter()
            .map(|(k, s)| Node {
                key: k.clone(),
                project: s.id.project.clone(),
                target: s.id.target.clone(),
                configuration: s.id.configuration.clone(),
            })
            .collect();
        for node in roots {
            let project = node.project.clone();
            self.process_task(&node, &project, req)?;
        }
        if req.exclude_task_dependencies {
            self.tasks.retain(|k, _| initial.contains(k));
            self.dependencies.retain(|k, _| initial.contains(k));
            for deps in self.dependencies.values_mut() {
                deps.retain(|d| initial.contains(d));
            }
        }
        self.filter_dummy_tasks();
        for (key, deps) in self.dependencies.iter_mut() {
            let mut seen = BTreeSet::new();
            deps.retain(|d| d != key && seen.insert(d.clone()));
        }
        Ok(())
    }

    fn create_task(
        &mut self,
        project: &Project,
        target: &str,
        configuration: Option<String>,
        overrides: Overrides,
    ) -> Result<Node> {
        let config = project.targets.get(target).ok_or_else(|| {
            eyre!(
                "Cannot find configuration for task {}:{target}",
                project.name
            )
        })?;
        if config.executor.is_none() {
            bail!(
                "Target \"{}:{target}\" does not have an executor configured",
                project.name
            );
        }
        let id = TaskId {
            project: project.name.clone(),
            target: target.to_string(),
            configuration,
        };
        let key = id.to_string();
        let node = Node {
            key: key.clone(),
            project: id.project.clone(),
            target: id.target.clone(),
            configuration: id.configuration.clone(),
        };
        self.tasks.insert(key, Spec { id, overrides });
        Ok(node)
    }

    /// `processTask`.
    fn process_task(&mut self, task: &Node, derive_from: &str, req: &Request<'_>) -> Result<()> {
        if !self.seen.insert(format!("{}-{derive_from}", task.key)) {
            return Ok(());
        }
        for dep in self.dependency_configs(&task.project, &task.target)? {
            let overrides = self.task_overrides(&dep, req.overrides, task);
            match &dep.projects {
                Some(projects) => {
                    if projects.is_empty() {
                        eprintln!(
                            "warning: `dependsOn` is misconfigured for {}:{}: its project patterns match no project",
                            task.project, task.target
                        );
                    }
                    for p in projects.clone() {
                        self.process_single_project(task, &p, &dep, &overrides, req)?;
                    }
                }
                None => self.process_dependencies(derive_from, &dep, task, &overrides, req)?,
            }
        }
        Ok(())
    }

    /// `processTasksForSingleProject`.
    fn process_single_project(
        &mut self,
        task: &Node,
        project_name: &str,
        dep: &DepConfig,
        overrides: &Overrides,
        req: &Request<'_>,
    ) -> Result<()> {
        let project = self.graph.get(project_name)?;
        let Some(config) = project.targets.get(&dep.target) else {
            return Ok(());
        };
        let configuration = resolve_configuration(config, req.configuration);
        let id = TaskId {
            project: project.name.clone(),
            target: dep.target.clone(),
            configuration,
        };
        let key = id.to_string();
        if !self.tasks.contains_key(&key) {
            let node = self.create_task(
                project,
                &dep.target,
                id.configuration.clone(),
                overrides.clone(),
            )?;
            self.dependencies.entry(node.key.clone()).or_default();
            let derive = node.project.clone();
            self.process_task(&node, &derive, req)?;
        }
        if task.key != key {
            self.dependencies
                .entry(task.key.clone())
                .or_default()
                .push(key);
        }
        Ok(())
    }

    /// `processTasksForDependencies`: each project dependency with the target
    /// gets a task; one without it is walked through (a dummy task), so
    /// `^build` reaches past a library that has nothing to build.
    fn process_dependencies(
        &mut self,
        derive_from: &str,
        dep: &DepConfig,
        task: &Node,
        overrides: &Overrides,
        req: &Request<'_>,
    ) -> Result<()> {
        let Some(project) = self.graph.projects.get(derive_from) else {
            return Ok(());
        };
        for dep_name in project.deps.clone() {
            let Some(dep_project) = self.graph.projects.get(&dep_name) else {
                continue;
            };
            if let Some(config) = dep_project.targets.get(&dep.target) {
                let configuration = resolve_configuration(config, req.configuration);
                let id = TaskId {
                    project: dep_project.name.clone(),
                    target: dep.target.clone(),
                    configuration,
                };
                let key = id.to_string();
                if task.key != key {
                    self.dependencies
                        .entry(task.key.clone())
                        .or_default()
                        .push(key.clone());
                }
                if !self.tasks.contains_key(&key) {
                    let node = self.create_task(
                        dep_project,
                        &dep.target,
                        id.configuration.clone(),
                        overrides.clone(),
                    )?;
                    self.dependencies.entry(node.key.clone()).or_default();
                    let derive = node.project.clone();
                    self.process_task(&node, &derive, req)?;
                }
            } else {
                let dummy_key = format!(
                    "{}:{}{}__{}{DUMMY_TASK_TARGET}",
                    dep_project.name, task.project, task.target, dep.target
                );
                self.dependencies
                    .entry(task.key.clone())
                    .or_default()
                    .push(dummy_key.clone());
                self.dependencies.entry(dummy_key.clone()).or_default();
                let dummy = Node {
                    key: dummy_key,
                    ..task.clone()
                };
                self.process_task(&dummy, &dep_project.name, req)?;
            }
        }
        Ok(())
    }

    /// `createTaskOverrides`.
    fn task_overrides(&self, dep: &DepConfig, cli: &Overrides, source: &Node) -> Overrides {
        let mut forwarded = JsonMap::new();
        if dep.options_forward
            && let Some(config) = self
                .graph
                .projects
                .get(&source.project)
                .and_then(|p| p.targets.get(&source.target))
        {
            if let Some(o) = &config.options {
                forwarded.extend(o.clone());
            }
            if let Some(c) = &source.configuration
                && let Some(cfg) = config.configurations.as_ref().and_then(|m| m.get(c))
            {
                forwarded.extend(cfg.clone());
            }
        }
        if dep.params_forward {
            forwarded.extend(cli.map.clone());
            Overrides {
                map: forwarded,
                unparsed: cli.unparsed.clone(),
            }
        } else {
            Overrides {
                map: forwarded,
                unparsed: Vec::new(),
            }
        }
    }

    /// `getDependencyConfigs` for one target.
    fn dependency_configs(&self, project_name: &str, target: &str) -> Result<Vec<DepConfig>> {
        let project = self.graph.get(project_name)?;
        let Some(entries) = project
            .targets
            .get(target)
            .and_then(|t| t.depends_on.as_ref())
        else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries {
            let dep = self
                .normalize(entry, project_name)
                .map_err(|e| eyre!("{project_name}:{target}: dependsOn {entry}: {e}"))?;
            if is_glob_pattern(&dep.target) {
                let m = glob(&dep.target)?;
                for t in self
                    .all_target_names
                    .iter()
                    .filter(|t| m.is_match(t.as_str()))
                {
                    out.push(DepConfig {
                        target: t.clone(),
                        ..dep.clone()
                    });
                }
            } else {
                out.push(dep);
            }
        }
        Ok(out)
    }

    /// `expandDependencyConfigSyntaxSugar` + `normalizeDependencyConfigProjects`.
    fn normalize(&self, entry: &Json, current: &str) -> Result<DepConfig> {
        let (target, mut projects, mut dependencies, params, options) = match entry {
            Json::String(s) => match s.strip_prefix('^') {
                Some(t) => (t.to_string(), None, true, None, None),
                None => match self.split_project_target(s) {
                    Some((p, t)) => (
                        t,
                        Some(Json::Array(vec![Json::String(p)])),
                        false,
                        None,
                        None,
                    ),
                    None => (s.clone(), None, false, None, None),
                },
            },
            Json::Object(o) => {
                let target = o
                    .get("target")
                    .and_then(Json::as_str)
                    .ok_or_else(|| eyre!("an object entry needs a string `target`"))?
                    .to_string();
                (
                    target,
                    o.get("projects").cloned(),
                    o.get("dependencies")
                        .and_then(Json::as_bool)
                        .unwrap_or(false),
                    o.get("params").and_then(Json::as_str).map(str::to_string),
                    o.get("options").and_then(Json::as_str).map(str::to_string),
                )
            }
            other => bail!("unsupported entry {other}"),
        };
        // The legacy `projects: "self" | "dependencies"` strings.
        match projects.as_ref() {
            Some(Json::String(s)) if s == "self" => projects = None,
            Some(Json::String(s)) if s == "dependencies" => {
                dependencies = true;
                projects = None;
            }
            Some(Json::String(s)) => projects = Some(Json::Array(vec![Json::String(s.clone())])),
            _ => {}
        }
        let projects = match projects {
            Some(Json::Array(list)) => {
                let patterns: Vec<String> = list
                    .iter()
                    .map(|p| {
                        p.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| eyre!("`projects` entries must be strings"))
                    })
                    .collect::<Result<_>>()?;
                Some(find_matching(self.graph, &patterns)?)
            }
            Some(other) => bail!("`projects` must be a string or an array, got {other}"),
            None if !dependencies => Some(vec![current.to_string()]),
            None => None,
        };
        Ok(DepConfig {
            target,
            projects,
            params_forward: params.as_deref() == Some("forward"),
            options_forward: options.as_deref() == Some("forward"),
        })
    }

    /// `readProjectAndTargetFromTargetString`: `project:target` when the part
    /// before a colon is a project, else the whole string is a target name
    /// (targets may contain colons, e.g. `build:debug`).
    fn split_project_target(&self, s: &str) -> Option<(String, String)> {
        self.graph
            .projects
            .keys()
            .filter(|p| {
                s.len() > p.len() + 1 && s.starts_with(p.as_str()) && s.as_bytes()[p.len()] == b':'
            })
            .max_by_key(|p| p.len())
            .map(|p| (p.clone(), s[p.len() + 1..].to_string()))
    }

    /// `filterDummyTasks`: replace each dummy dependency with the real tasks
    /// behind it.
    fn filter_dummy_tasks(&mut self) {
        let cycles = find_cycles(&self.dependencies);
        let keys: Vec<String> = self.dependencies.keys().cloned().collect();
        for key in keys.iter().filter(|k| !k.ends_with(DUMMY_TASK_TARGET)) {
            let mut normalized = Vec::new();
            for dep in self.dependencies[key].clone() {
                let mut seen = HashSet::from([key.clone()]);
                non_dummy_deps(
                    &dep,
                    &self.dependencies,
                    &cycles,
                    &mut seen,
                    &mut normalized,
                );
            }
            self.dependencies.insert(key.clone(), normalized);
        }
        self.dependencies
            .retain(|k, _| !k.ends_with(DUMMY_TASK_TARGET));
    }
}

fn non_dummy_deps(
    current: &str,
    deps: &BTreeMap<String, Vec<String>>,
    cycles: &HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    if !seen.insert(current.to_string()) {
        return;
    }
    if current.ends_with(DUMMY_TASK_TARGET) {
        if cycles.contains(current) {
            return;
        }
        for d in deps.get(current).into_iter().flatten() {
            non_dummy_deps(d, deps, cycles, seen, out);
        }
    } else {
        out.push(current.to_string());
    }
}

/// Every task that can reach itself.
fn find_cycles(deps: &BTreeMap<String, Vec<String>>) -> HashSet<String> {
    let mut out = HashSet::new();
    for start in deps.keys() {
        let mut stack: Vec<&str> = deps[start].iter().map(String::as_str).collect();
        let mut visited: HashSet<&str> = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == start {
                out.insert(start.clone());
                break;
            }
            if visited.insert(n) {
                stack.extend(deps.get(n).into_iter().flatten().map(String::as_str));
            }
        }
    }
    out
}

fn topo_sort(tasks: Vec<Task>) -> Result<Vec<Task>> {
    let mut pending: BTreeMap<TaskId, Task> =
        tasks.into_iter().map(|t| (t.id.clone(), t)).collect();
    let mut done: BTreeSet<TaskId> = BTreeSet::new();
    let mut out = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let ready: Vec<TaskId> = pending
            .values()
            .filter(|t| t.deps.iter().all(|d| done.contains(d)))
            .map(|t| t.id.clone())
            .collect();
        if ready.is_empty() {
            let cycle: Vec<String> = pending.keys().map(ToString::to_string).collect();
            bail!(
                "the task graph has a circular dependency among: {}",
                cycle.join(", ")
            );
        }
        for id in ready {
            done.insert(id.clone());
            out.push(pending.remove(&id).expect("ready task is pending"));
        }
    }
    Ok(out)
}

/// nx `interpolateOverrides`: string overrides may use `{projectRoot}` etc.
fn interpolate_overrides(overrides: &JsonMap, project: &Project) -> Result<JsonMap> {
    let data = interpolation_data(project, None, true);
    overrides
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Json::String(s) => Json::String(interpolate(s, &data)?),
                other => other.clone(),
            };
            Ok((k.clone(), v))
        })
        .collect()
}

fn interpolation_data(project: &Project, options: Option<&JsonMap>, workspace_root: bool) -> Json {
    let mut data = serde_json::json!({
        "projectRoot": project.root,
        "projectName": project.name,
        "project": {
            "name": project.name,
            "root": project.root,
            "tags": project.tags,
            "projectType": project.project_type,
            "implicitDependencies": project.implicit_dependencies,
        },
    });
    if let Some(o) = options {
        data["options"] = Json::Object(o.clone());
    }
    if workspace_root {
        data["workspaceRoot"] = Json::String(String::new());
    }
    data
}

/// nx `getOutputsForTargetAndConfiguration`.
fn outputs(
    project: &Project,
    target_name: &str,
    target: &TargetConfig,
    options: &JsonMap,
) -> Result<Vec<String>> {
    if let Some(list) = &target.outputs {
        let data = interpolation_data(project, Some(options), false);
        let mut out: Vec<String> = Vec::new();
        for o in list {
            let resolved = interpolate(o, &data)?;
            if !resolved.is_empty() && !has_unresolved_token(&resolved) && !out.contains(&resolved)
            {
                out.push(resolved);
            }
        }
        return Ok(out);
    }
    if let Some(p) = options.get("outputPath") {
        return Ok(match p {
            Json::Array(a) => a.iter().map(executor::args::js_string).collect(),
            other => vec![executor::args::js_string(other)],
        });
    }
    if target_name == "build" || target_name == "prepare" {
        let r = &project.root;
        return Ok(vec![
            format!("dist/{r}"),
            format!("{r}/dist"),
            format!("{r}/build"),
            format!("{r}/public"),
        ]);
    }
    Ok(Vec::new())
}

/// `/{(projectRoot|workspaceRoot|(options.*))}/`
fn has_unresolved_token(s: &str) -> bool {
    ["{projectRoot}", "{workspaceRoot}"]
        .iter()
        .any(|t| s.contains(t))
        || s.match_indices("{options")
            .any(|(i, _)| s[i..].contains('}'))
}

/// nx `interpolate` (tasks-runner/utils.js): per path segment, `{a.b}` walks
/// `data`; unresolvable tokens stay; segments are joined `path.posix.join`
/// style and a leading `{workspaceRoot}/` is dropped.
pub fn interpolate(template: &str, data: &Json) -> Result<String> {
    if template.starts_with('/') || !has_token(template) {
        return Ok(template.to_string());
    }
    if template.len() > 1 && template[1..].contains("{workspaceRoot}") {
        bail!(
            "Output '{template}' is invalid. {{workspaceRoot}} can only be used at the beginning of the expression."
        );
    }
    let project_root_is_dot = data.get("projectRoot").and_then(Json::as_str) == Some(".");
    let parts: Vec<String> = template
        .split('/')
        .map(|seg| {
            let seg = if project_root_is_dot {
                seg.replacen("{projectRoot}", "", 1)
            } else {
                seg.to_string()
            };
            interpolate_segment(&seg, data)
        })
        .collect();
    Ok(posix_join(&parts).replacen("{workspaceRoot}/", "", 1))
}

fn has_token(s: &str) -> bool {
    s.find('{')
        .is_some_and(|i| s[i + 1..].find('}').is_some_and(|n| n > 0))
}

fn interpolate_segment(seg: &str, data: &Json) -> String {
    let mut out = String::new();
    let mut rest = seg;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}').filter(|c| *c > 0) else {
            break;
        };
        out.push_str(&rest[..open]);
        let token = &after[..close];
        let mut value = Some(data);
        for key in token.trim().split('.') {
            value = value.and_then(|v| v.get(key)).filter(|v| truthy(v));
        }
        match value {
            Some(v) => out.push_str(&executor::args::js_string(v)),
            None => {
                out.push('{');
                out.push_str(token);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn truthy(v: &Json) -> bool {
    match v {
        Json::Null | Json::Bool(false) => false,
        Json::String(s) => !s.is_empty(),
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => true,
    }
}

/// `path.posix.join(...parts)`.
fn posix_join(parts: &[String]) -> String {
    let joined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        return ".".into();
    }
    let absolute = joined.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !absolute {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let body = out.join("/");
    match (absolute, body.is_empty()) {
        (true, _) => format!("/{body}"),
        (false, true) => ".".into(),
        (false, false) => {
            if joined.ends_with('/') {
                format!("{body}/")
            } else {
                body
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(name: &str, deps: &[&str], targets: Json) -> Project {
        Project {
            name: name.into(),
            root: format!("libs/{name}"),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets: serde_json::from_value(targets).unwrap(),
            deps: deps.iter().map(|d| (*d).to_string()).collect(),
            build_deps: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    fn graph(projects: Vec<Project>) -> ProjectGraph {
        ProjectGraph {
            projects: projects.into_iter().map(|p| (p.name.clone(), p)).collect(),
            ..Default::default()
        }
    }

    fn run(
        g: &ProjectGraph,
        projects: &[&str],
        targets: &[&str],
        cfg: Option<&str>,
        cli: &[&str],
    ) -> Result<Vec<Task>> {
        let projects: Vec<String> = projects.iter().map(|s| (*s).to_string()).collect();
        let targets: Vec<String> = targets.iter().map(|s| (*s).to_string()).collect();
        let cli: Vec<String> = cli.iter().map(|s| (*s).to_string()).collect();
        let overrides = Overrides::from_args(&cli);
        build(
            Path::new("/nonexistent-butler-ws"),
            g,
            &BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]),
            &Request {
                projects: &projects,
                targets: &targets,
                configuration: cfg,
                overrides: &overrides,
                exclude_task_dependencies: false,
            },
        )
    }

    fn cmd(c: &str) -> Json {
        json!({"executor": "nx:run-commands", "options": {"command": c}})
    }

    fn ids(tasks: &[Task]) -> Vec<String> {
        tasks.iter().map(|t| t.id.to_string()).collect()
    }

    #[test]
    fn caret_walks_through_projects_without_the_target() {
        let mut app_build = cmd("b");
        app_build["dependsOn"] = json!(["^build"]);
        let g = graph(vec![
            project("base", &[], json!({"build": cmd("b")})),
            project("mid", &["base"], json!({"test": cmd("t")})),
            project("app", &["mid"], json!({"build": app_build})),
        ]);
        let tasks = run(&g, &["app"], &["build"], None, &[]).unwrap();
        assert_eq!(ids(&tasks), vec!["base:build", "app:build"]);
        assert!(tasks[1].deps.iter().any(|d| d.project == "base"));
    }

    #[test]
    fn object_forms_and_project_target_strings() {
        let mut t = cmd("x");
        t["dependsOn"] = json!([
            {"projects": ["lib"], "target": "gen"},
            "other:lint",
            {"target": "fmt"},
            "build:debug"
        ]);
        let g = graph(vec![
            project("lib", &[], json!({"gen": cmd("g")})),
            project("other", &[], json!({"lint": cmd("l")})),
            project(
                "app",
                &[],
                json!({"x": t, "fmt": cmd("f"), "build:debug": cmd("d")}),
            ),
        ]);
        let tasks = run(&g, &["app"], &["x"], None, &[]).unwrap();
        let deps: Vec<String> = tasks
            .last()
            .unwrap()
            .deps
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            deps,
            vec!["app:build:debug", "app:fmt", "lib:gen", "other:lint"]
        );
    }

    #[test]
    fn overrides_reach_dependencies_only_when_forwarded() {
        let mut t = cmd("x");
        t["dependsOn"] = json!(["plain", {"target": "fwd", "params": "forward"}]);
        let g = graph(vec![project(
            "app",
            &[],
            json!({"x": t, "plain": cmd("p"), "fwd": cmd("f")}),
        )]);
        let tasks = run(&g, &["app"], &["x"], None, &["--check"]).unwrap();
        let by_target = |n: &str| tasks.iter().find(|t| t.id.target == n).unwrap();
        assert!(by_target("plain").overrides.unparsed.is_empty());
        assert_eq!(by_target("fwd").overrides.unparsed, vec!["--check"]);
        assert_eq!(by_target("x").overrides.map["check"], json!(true));
    }

    #[test]
    fn configuration_applies_only_where_it_exists() {
        let mut t = cmd("x");
        t["configurations"] = json!({"ci": {"command": "x --ci"}});
        t["dependsOn"] = json!(["dep"]);
        let g = graph(vec![project("app", &[], json!({"x": t, "dep": cmd("d")}))]);
        let tasks = run(&g, &["app"], &["x"], Some("ci"), &[]).unwrap();
        assert_eq!(ids(&tasks), vec!["app:dep", "app:x:ci"]);
    }

    #[test]
    fn cycles_are_reported() {
        let mut a = cmd("a");
        a["dependsOn"] = json!(["b"]);
        let mut b = cmd("b");
        b["dependsOn"] = json!(["a"]);
        let g = graph(vec![project("p", &[], json!({"a": a, "b": b}))]);
        let err = run(&g, &["p"], &["a"], None, &[]).err().unwrap();
        assert!(err.to_string().contains("circular"));
    }

    #[test]
    fn outputs_interpolate_like_nx() {
        let p = project("x", &[], json!({}));
        let t: TargetConfig = serde_json::from_value(json!({
            "outputs": ["{workspaceRoot}/coverage/{projectRoot}", "{projectRoot}/dist", "{options.outDir}", "{options.missing}/x"]
        }))
        .unwrap();
        let opts: JsonMap = serde_json::from_value(json!({"outDir": "out/x"})).unwrap();
        assert_eq!(
            outputs(&p, "test", &t, &opts).unwrap(),
            vec!["coverage/libs/x", "libs/x/dist", "out/x"]
        );
        let none: TargetConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            outputs(&p, "build", &none, &JsonMap::new()).unwrap().len(),
            4
        );
    }
}
