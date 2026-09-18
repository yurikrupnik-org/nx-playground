//! Project model, project graph, and task graph.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use eyre::{Result, bail, eyre};

use crate::config::{ResolvedTarget, TargetConfig};

#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    /// Workspace-root-relative root directory.
    pub root: String,
    pub project_type: Option<String>,
    pub tags: Vec<String>,
    /// Raw (unresolved) targets, already merged: scripts < inferred < project.json.
    pub targets: BTreeMap<String, TargetConfig>,
    /// Names of workspace projects this project depends on.
    pub deps: BTreeSet<String>,
    /// `deps` minus dev-dependencies: what actually feeds a shipped artifact.
    pub build_deps: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct ProjectGraph {
    pub projects: BTreeMap<String, Project>,
}

impl ProjectGraph {
    pub fn get(&self, name: &str) -> Result<&Project> {
        self.projects
            .get(name)
            .ok_or_else(|| eyre!("unknown project `{name}`"))
    }

    /// Reverse adjacency: project -> projects that depend on it.
    pub fn dependents_map(&self) -> BTreeMap<&str, BTreeSet<&str>> {
        let mut map: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for p in self.projects.values() {
            for dep in &p.deps {
                map.entry(dep.as_str()).or_default().insert(p.name.as_str());
            }
        }
        map
    }

    /// `seeds` plus everything that transitively depends on them.
    pub fn with_dependents(&self, seeds: &BTreeSet<String>) -> BTreeSet<String> {
        let dependents = self.dependents_map();
        let mut out: BTreeSet<String> = seeds.clone();
        let mut queue: VecDeque<&str> = seeds.iter().map(String::as_str).collect();
        while let Some(name) = queue.pop_front() {
            if let Some(users) = dependents.get(name) {
                for user in users {
                    if out.insert((*user).to_string()) {
                        queue.push_back(user);
                    }
                }
            }
        }
        out
    }

    /// Transitive non-dev dependency closure of `name`, excluding `name` itself.
    /// This is what makes a generated `docker_build(only=...)` tight: only the
    /// crates that actually feed the binary enter the build context.
    pub fn transitive_build_deps(&self, name: &str) -> Result<BTreeSet<String>> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = self.get(name)?.build_deps.iter().cloned().collect();
        while let Some(dep) = queue.pop_front() {
            if !out.insert(dep.clone()) {
                continue;
            }
            // A dep may be an external/undiscovered name; skip rather than fail.
            if let Some(p) = self.projects.get(&dep) {
                queue.extend(p.build_deps.iter().cloned());
            }
        }
        out.remove(name);
        Ok(out)
    }

    /// Map a changed file to the owning project (longest matching root).
    pub fn project_for_file<'a>(&'a self, file: &str) -> Option<&'a Project> {
        self.projects
            .values()
            .filter(|p| p.root == "." || file.starts_with(&format!("{}/", p.root)))
            .max_by_key(|p| if p.root == "." { 0 } else { p.root.len() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId {
    pub project: String,
    pub target: String,
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.project, self.target)
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub resolved: ResolvedTarget,
    /// Task-level dependencies (edges into the task graph).
    pub deps: BTreeSet<TaskId>,
}

/// Expand seed (project, target) pairs into the full task graph by following
/// `dependsOn` ("t" = same project, "^t" = each project dependency that has t),
/// then return tasks in a valid topological order.
pub fn build_task_graph(
    graph: &ProjectGraph,
    seeds: &[(String, String)],
    configuration: Option<&str>,
) -> Result<Vec<Task>> {
    let mut tasks: BTreeMap<TaskId, Task> = BTreeMap::new();
    let mut queue: VecDeque<TaskId> = VecDeque::new();

    for (project, target) in seeds {
        let p = graph.get(project)?;
        if !p.targets.contains_key(target) {
            bail!("project `{project}` has no target `{target}`");
        }
        queue.push_back(TaskId {
            project: project.clone(),
            target: target.clone(),
        });
    }

    while let Some(id) = queue.pop_front() {
        if tasks.contains_key(&id) {
            continue;
        }
        let project = graph.get(&id.project)?;
        let config = project
            .targets
            .get(&id.target)
            .ok_or_else(|| eyre!("project `{}` has no target `{}`", id.project, id.target))?;
        let resolved = config.resolve(&project.name, &project.root, &id.target, configuration)?;

        let mut deps = BTreeSet::new();
        for dep_spec in &resolved.depends_on {
            if let Some(dep_target) = dep_spec.strip_prefix('^') {
                for dep_project in &project.deps {
                    let dp = graph.get(dep_project)?;
                    if dp.targets.contains_key(dep_target) {
                        deps.insert(TaskId {
                            project: dep_project.clone(),
                            target: dep_target.to_string(),
                        });
                    }
                }
            } else {
                if !project.targets.contains_key(dep_spec) {
                    bail!(
                        "{}: dependsOn `{dep_spec}` but the project has no such target",
                        id
                    );
                }
                deps.insert(TaskId {
                    project: id.project.clone(),
                    target: dep_spec.clone(),
                });
            }
        }

        for dep in &deps {
            queue.push_back(dep.clone());
        }
        tasks.insert(id.clone(), Task { id, resolved, deps });
    }

    topo_sort(tasks)
}

fn topo_sort(mut tasks: BTreeMap<TaskId, Task>) -> Result<Vec<Task>> {
    let mut in_degree: BTreeMap<TaskId, usize> = tasks
        .iter()
        .map(|(id, t)| (id.clone(), t.deps.len()))
        .collect();
    let mut dependents: BTreeMap<TaskId, Vec<TaskId>> = BTreeMap::new();
    for t in tasks.values() {
        for dep in &t.deps {
            dependents
                .entry(dep.clone())
                .or_default()
                .push(t.id.clone());
        }
    }

    let mut ready: VecDeque<TaskId> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut out = Vec::with_capacity(tasks.len());

    while let Some(id) = ready.pop_front() {
        for user in dependents.get(&id).into_iter().flatten() {
            let d = in_degree
                .get_mut(user)
                .expect("dependent must be in the task set");
            *d -= 1;
            if *d == 0 {
                ready.push_back(user.clone());
            }
        }
        let task = tasks.remove(&id).expect("ready task must exist");
        out.push(task);
    }

    if !tasks.is_empty() {
        let cycle: Vec<String> = tasks.keys().map(ToString::to_string).collect();
        bail!("task graph has a cycle among: {}", cycle.join(", "));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunOptions;

    fn project(name: &str, root: &str, deps: &[&str], targets: &[(&str, &[&str])]) -> Project {
        let mut map = BTreeMap::new();
        for (target, depends_on) in targets {
            map.insert(
                (*target).to_string(),
                TargetConfig {
                    depends_on: Some(depends_on.iter().map(|d| serde_json::json!(*d)).collect()),
                    options: RunOptions {
                        command: Some(format!("echo {name}:{target}")),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            targets: map,
            deps: deps.iter().map(|d| (*d).to_string()).collect(),
            build_deps: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    fn graph(projects: Vec<Project>) -> ProjectGraph {
        ProjectGraph {
            projects: projects.into_iter().map(|p| (p.name.clone(), p)).collect(),
        }
    }

    #[test]
    fn dependents_closure_includes_transitive_users() {
        let g = graph(vec![
            project("lib", "libs/lib", &[], &[("build", &[])]),
            project("mid", "libs/mid", &["lib"], &[("build", &[])]),
            project("app", "apps/app", &["mid"], &[("build", &[])]),
            project("other", "apps/other", &[], &[("build", &[])]),
        ]);
        let affected = g.with_dependents(&BTreeSet::from(["lib".to_string()]));
        assert_eq!(
            affected,
            BTreeSet::from(["lib".into(), "mid".into(), "app".into()])
        );
    }

    #[test]
    fn file_maps_to_longest_project_root() {
        let g = graph(vec![
            project("outer", "libs/core/proc_macros", &[], &[("build", &[])]),
            project(
                "inner",
                "libs/core/proc_macros/api_resource",
                &[],
                &[("build", &[])],
            ),
        ]);
        let p = g
            .project_for_file("libs/core/proc_macros/api_resource/src/lib.rs")
            .unwrap();
        assert_eq!(p.name, "inner");
        let p = g
            .project_for_file("libs/core/proc_macros/src/lib.rs")
            .unwrap();
        assert_eq!(p.name, "outer");
        assert!(g.project_for_file("justfile").is_none());
    }

    #[test]
    fn caret_depends_on_expands_to_project_deps_with_target() {
        let g = graph(vec![
            project("lib", "libs/lib", &[], &[("build", &[])]),
            project("nolib", "libs/nolib", &[], &[("test", &[])]), // no build target
            project(
                "app",
                "apps/app",
                &["lib", "nolib"],
                &[("build", &["^build"])],
            ),
        ]);
        let tasks = build_task_graph(&g, &[("app".into(), "build".into())], None).unwrap();
        let ids: Vec<String> = tasks.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(ids, vec!["lib:build", "app:build"]); // topo order, nolib skipped
    }

    #[test]
    fn same_project_depends_on_orders_tasks() {
        let g = graph(vec![project(
            "kcl",
            "scripts/kcl/ci",
            &[],
            &[("pkg", &["test", "lint"]), ("test", &[]), ("lint", &[])],
        )]);
        let tasks = build_task_graph(&g, &[("kcl".into(), "pkg".into())], None).unwrap();
        let ids: Vec<String> = tasks.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids.last().unwrap(), "kcl:pkg");
    }

    #[test]
    fn cycle_is_reported() {
        let g = graph(vec![project(
            "a",
            "libs/a",
            &[],
            &[("x", &["y"]), ("y", &["x"])],
        )]);
        let err = build_task_graph(&g, &[("a".into(), "x".into())], None).unwrap_err();
        assert!(format!("{err:#}").contains("cycle"));
    }
}
