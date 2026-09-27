//! Read-only queries over a parsed [`Graph`]: name/alias lookup, edges in both
//! directions, transitive closures and the drill-down tree.

use std::collections::{HashMap, HashSet, VecDeque};

use contract_taskgraph::{Graph, TaskNode, Via};
use serde::Serialize;

/// A resolved edge to (or from) `task`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Edge<'g> {
    pub task: &'g str,
    pub via: Via,
}

/// One node of the drill-down tree rooted at a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeNode {
    pub task: String,
    pub via: Via,
    /// `false` for a templated or unknown reference (always a leaf).
    pub resolved: bool,
    /// Already expanded earlier in this tree (shared dep or cycle): children elided.
    pub repeated: bool,
    /// Depth limit reached: children elided.
    pub truncated: bool,
    pub children: Vec<TreeNode>,
}

pub struct GraphIndex<'g> {
    graph: &'g Graph,
    by_name: HashMap<&'g str, &'g TaskNode>,
    /// Namespaced task alias (`rust:l`) → canonical name (`rust:lint`).
    by_alias: HashMap<String, &'g str>,
    dependents: HashMap<&'g str, Vec<Edge<'g>>>,
}

impl<'g> GraphIndex<'g> {
    pub fn new(graph: &'g Graph) -> Self {
        let by_name: HashMap<&str, &TaskNode> =
            graph.tasks.iter().map(|t| (t.name.as_str(), t)).collect();
        let mut by_alias = HashMap::new();
        let mut dependents: HashMap<&str, Vec<Edge>> = HashMap::new();
        for task in &graph.tasks {
            let namespace = task.name.rsplit_once(':').map(|(ns, _)| ns);
            for alias in &task.aliases {
                let full = match namespace {
                    Some(ns) => format!("{ns}:{alias}"),
                    None => alias.clone(),
                };
                by_alias.entry(full).or_insert(task.name.as_str());
            }
            for (via, target) in outgoing(task) {
                dependents.entry(target).or_default().push(Edge {
                    task: task.name.as_str(),
                    via,
                });
            }
        }
        Self {
            graph,
            by_name,
            by_alias,
            dependents,
        }
    }

    pub fn graph(&self) -> &'g Graph {
        self.graph
    }

    /// Canonical name for a task name or alias.
    pub fn resolve(&self, name: &str) -> Option<&'g str> {
        self.by_name
            .get_key_value(name)
            .map(|(k, _)| *k)
            .or_else(|| self.by_alias.get(name).copied())
    }

    pub fn task(&self, name: &str) -> Option<&'g TaskNode> {
        self.resolve(name)
            .and_then(|n| self.by_name.get(n).copied())
    }

    /// Resolved outgoing edges: `deps` first, then `cmds` calls, declaration order.
    pub fn edges(&self, name: &str) -> Vec<Edge<'g>> {
        self.task(name)
            .map(|t| outgoing(t).map(|(via, task)| Edge { task, via }).collect())
            .unwrap_or_default()
    }

    /// Direct dependents: tasks whose `deps` or `cmds` reference `name`.
    pub fn dependents(&self, name: &str) -> &[Edge<'g>] {
        self.resolve(name)
            .and_then(|n| self.dependents.get(n))
            .map_or(&[], Vec::as_slice)
    }

    /// Everything `name` can cause to run, excluding itself, in dependency
    /// order (a task appears after everything it needs).
    pub fn closure(&self, name: &str) -> Vec<&'g str> {
        let Some(root) = self.resolve(name) else {
            return Vec::new();
        };
        let mut seen = HashSet::from([root]);
        let mut order = Vec::new();
        // Iterative post-order DFS: (task, its edges, next edge index).
        let mut stack = vec![(root, self.edges(root), 0usize)];
        while let Some(top) = stack.last_mut() {
            if let Some(edge) = top.1.get(top.2).copied() {
                top.2 += 1;
                if seen.insert(edge.task) {
                    stack.push((edge.task, self.edges(edge.task), 0));
                }
            } else if let Some((done, ..)) = stack.pop()
                && done != root
            {
                order.push(done);
            }
        }
        order
    }

    /// Everything that (transitively) runs `name`, nearest first.
    pub fn transitive_dependents(&self, name: &str) -> Vec<&'g str> {
        let Some(root) = self.resolve(name) else {
            return Vec::new();
        };
        let mut seen = HashSet::from([root]);
        let mut queue = VecDeque::from([root]);
        let mut out = Vec::new();
        while let Some(task) = queue.pop_front() {
            for edge in self.dependents(task) {
                if seen.insert(edge.task) {
                    out.push(edge.task);
                    queue.push_back(edge.task);
                }
            }
        }
        out
    }

    /// Entry points: public tasks nothing else references.
    pub fn roots(&self) -> Vec<&'g str> {
        self.graph
            .tasks
            .iter()
            .filter(|t| !t.internal && !self.dependents.contains_key(t.name.as_str()))
            .map(|t| t.name.as_str())
            .collect()
    }

    /// Drill-down tree from `name`, each task expanded once (later occurrences
    /// are marked `repeated`), cut at `max_depth` levels below the root.
    pub fn tree(&self, name: &str, max_depth: usize) -> Option<TreeNode> {
        let root = self.resolve(name)?;
        let mut expanded = HashSet::new();
        Some(self.subtree(root, Via::Root, true, 0, max_depth, &mut expanded))
    }

    fn subtree(
        &self,
        name: &'g str,
        via: Via,
        resolved: bool,
        depth: usize,
        max_depth: usize,
        expanded: &mut HashSet<&'g str>,
    ) -> TreeNode {
        let mut node = TreeNode {
            task: name.to_string(),
            via,
            resolved,
            repeated: false,
            truncated: false,
            children: Vec::new(),
        };
        let Some(task) = resolved.then(|| self.by_name.get(name)).flatten() else {
            return node;
        };
        let has_children = task.deps.len() + task.calls.len() > 0;
        if !expanded.insert(name) {
            node.repeated = has_children;
            return node;
        }
        if depth >= max_depth {
            node.truncated = has_children;
            return node;
        }
        let refs = task
            .deps
            .iter()
            .map(|r| (Via::Dep, r))
            .chain(task.calls.iter().map(|r| (Via::Call, r)));
        for (via, reference) in refs {
            node.children.push(self.subtree(
                reference.name.as_str(),
                via,
                reference.resolved,
                depth + 1,
                max_depth,
                expanded,
            ));
        }
        node
    }
}

fn outgoing(task: &TaskNode) -> impl Iterator<Item = (Via, &str)> {
    task.deps
        .iter()
        .map(|r| (Via::Dep, r))
        .chain(task.calls.iter().map(|r| (Via::Call, r)))
        .filter(|(_, r)| r.resolved)
        .map(|(via, r)| (via, r.name.as_str()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use contract_taskgraph::TaskRef;

    pub(crate) fn node(name: &str, deps: &[&str], calls: &[&str]) -> TaskNode {
        let refs = |names: &[&str]| {
            names
                .iter()
                .map(|n| TaskRef {
                    name: (*n).to_string(),
                    resolved: !n.starts_with('?'),
                })
                .collect()
        };
        TaskNode {
            name: name.to_string(),
            desc: None,
            summary: None,
            aliases: Vec::new(),
            taskfile: "Taskfile.yml".into(),
            internal: false,
            deps: refs(deps),
            calls: refs(calls),
            cmds: Vec::new(),
            sources: Vec::new(),
            generates: Vec::new(),
            status: Vec::new(),
            preconditions: Vec::new(),
            requires: Vec::new(),
            run: None,
            dir: None,
        }
    }

    pub(crate) fn graph(tasks: Vec<TaskNode>) -> Graph {
        Graph {
            id: "g".into(),
            taskfile: "/r/Taskfile.yml".into(),
            host: "h".into(),
            digest: "d".into(),
            tasks,
            includes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// check → {lint, test} → build; test also calls report; cycle x ↔ y.
    fn sample() -> Graph {
        graph(vec![
            node("check", &["lint", "test"], &[]),
            node("lint", &["build"], &[]),
            node("test", &["build"], &["report", "?{{.X}}"]),
            node("build", &[], &[]),
            node("report", &[], &[]),
            node("x", &["y"], &[]),
            node("y", &["x"], &[]),
        ])
    }

    #[test]
    fn closure_is_dependency_ordered_and_dependents_are_transitive() {
        let g = sample();
        let index = GraphIndex::new(&g);
        let closure = index.closure("check");
        assert_eq!(closure.len(), 4);
        let pos = |t: &str| closure.iter().position(|c| *c == t).expect(t);
        assert!(pos("build") < pos("lint") && pos("build") < pos("test"));
        assert!(pos("report") < pos("test"));

        assert_eq!(
            index.transitive_dependents("build"),
            ["lint", "test", "check"]
        );
        assert_eq!(index.closure("x"), ["y"], "cycles terminate");
        assert_eq!(index.roots(), ["check"]);
    }

    #[test]
    fn tree_expands_each_task_once_and_honours_depth() {
        let g = sample();
        let index = GraphIndex::new(&g);
        let tree = index.tree("check", 10).expect("tree");
        let lint = &tree.children[0];
        let test = &tree.children[1];
        assert_eq!(lint.children[0].task, "build");
        assert!(!lint.children[0].repeated, "build has no children to elide");
        assert_eq!(test.children.len(), 3);
        assert_eq!(test.children[1].via, Via::Call);
        assert!(!test.children[2].resolved);

        let shallow = index.tree("check", 1).expect("tree");
        assert!(shallow.children[0].truncated);
        let cyclic = index.tree("x", 10).expect("tree");
        assert!(cyclic.children[0].children[0].repeated);
    }
}
