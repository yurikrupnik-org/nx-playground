//! Project model and project graph. (The task graph is in [`crate::task`].)

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use eyre::{Result, eyre};

use crate::config::TargetConfig;

#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    /// Workspace-root-relative root directory.
    pub root: String,
    pub project_type: Option<String>,
    /// In contribution order, de-duplicated — nx unions tags across layers.
    pub tags: Vec<String>,
    /// `implicitDependencies` as nx normalizes them: patterns expanded to
    /// project names, `!name` exclusions kept.
    pub implicit_dependencies: Vec<String>,
    /// Targets as nx's graph holds them: every inference layer and
    /// `targetDefaults` merged, then normalized (tokens resolved in options).
    pub targets: BTreeMap<String, TargetConfig>,
    /// Names of workspace projects this project depends on, over edges of
    /// every kind (static, dynamic, implicit).
    pub deps: BTreeSet<String>,
    /// `deps` minus dev-dependencies: what actually feeds a shipped artifact.
    pub build_deps: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct ProjectGraph {
    pub projects: BTreeMap<String, Project>,
    /// Every project-to-project edge with its nx kind — nx's `dependencies`;
    /// [`Project::deps`] is the kind-less view of the same edges.
    pub edges: BTreeSet<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: crate::infer::DepKind,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, root: &str, deps: &[&str], targets: &[(&str, &[&str])]) -> Project {
        let mut map = BTreeMap::new();
        for (target, depends_on) in targets {
            map.insert(
                (*target).to_string(),
                serde_json::from_value(serde_json::json!({
                    "dependsOn": depends_on,
                    "options": {"command": format!("echo {name}:{target}")},
                }))
                .unwrap(),
            );
        }
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets: map,
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
        assert!(g.project_for_file("Taskfile.yml").is_none());
    }
}
