//! go-task v3 Taskfile parser: the root file plus every local include,
//! flattened into go-task's `namespace:task` names.
//!
//! This reads the *declared* graph — it does not evaluate templates, run
//! `status:` checks or expand `for:` loops. Anything it cannot resolve
//! statically (remote includes, `{{.VAR}}` paths or task names) becomes an
//! unresolved node plus a warning rather than an error, so one exotic include
//! never hides the rest of the graph. Execution semantics stay go-task's: the
//! CLI runs the real `task` binary and observes it (`observe` module).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use contract_taskgraph::{Graph, IncludeNode, TaskNode, TaskRef};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::error::{TaskgraphError, TaskgraphResult};

/// File names go-task looks for, in its lookup order.
pub const DEFAULT_TASKFILES: [&str; 8] = [
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

/// The Taskfile go-task would use when invoked from `start`: the first default
/// name in `start`, else in the nearest ancestor that has one.
pub fn find_taskfile(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(taskfile_in)
}

fn taskfile_in(dir: &Path) -> Option<PathBuf> {
    DEFAULT_TASKFILES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Parse `root` and its local includes into a [`Graph`] owned by `host`.
pub fn parse(root: &Path, host: &str) -> TaskgraphResult<Graph> {
    let root = std::fs::canonicalize(root).map_err(|source| TaskgraphError::Io {
        path: root.display().to_string(),
        source,
    })?;
    let root_dir = root.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut loader = Loader {
        root_dir,
        drafts: BTreeMap::new(),
        aliases: HashMap::new(),
        includes: Vec::new(),
        warnings: Vec::new(),
        stack: Vec::new(),
    };
    let scope = Scope {
        namespace: String::new(),
        alias_prefixes: Vec::new(),
        internal: false,
        excludes: Vec::new(),
    };
    // The root file is the one place a read/parse failure is fatal: without it
    // there is no graph at all.
    let doc = read_yaml(&root)?;
    loader.load(&root, &doc, &scope);
    Ok(loader.finish(&root, host))
}

/// Stable identity of a Taskfile on a host: the same file keeps the same id
/// across edits, so a re-publish replaces rather than duplicates it.
pub fn graph_id(host: &str, root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(host.as_bytes());
    hasher.update([0]);
    hasher.update(root.as_os_str().as_encoded_bytes());
    hex_prefix(&hasher.finalize(), 12)
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(chars);
    for byte in bytes.iter().take(chars.div_ceil(2)) {
        let _ = write!(out, "{byte:02x}");
    }
    out.truncate(chars);
    out
}

fn read_yaml(path: &Path) -> TaskgraphResult<Value> {
    let text = std::fs::read_to_string(path).map_err(|source| TaskgraphError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_yaml_ng::from_str(&text).map_err(|source| TaskgraphError::Yaml {
        path: path.display().to_string(),
        source,
    })
}

/// Where a file's tasks land in the flattened namespace.
struct Scope {
    namespace: String,
    /// Extra prefixes the file's tasks answer to (include `aliases:`).
    alias_prefixes: Vec<String>,
    internal: bool,
    excludes: Vec<String>,
}

/// A task before its references are resolved against the full name table.
struct Draft {
    node: TaskNode,
    /// Namespace of the declaring file: references resolve relative to it.
    namespace: String,
    deps: Vec<String>,
    calls: Vec<String>,
}

struct Loader {
    root_dir: PathBuf,
    drafts: BTreeMap<String, Draft>,
    /// Every alternative name (task aliases, include-alias prefixes) → canonical.
    aliases: HashMap<String, String>,
    includes: Vec<IncludeNode>,
    warnings: Vec<String>,
    /// Files currently being loaded, to break include cycles.
    stack: Vec<PathBuf>,
}

fn join(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

fn key_str(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn get<'v>(map: &'v Mapping, key: &str) -> Option<&'v Value> {
    map.get(Value::String(key.to_string()))
}

fn get_str(map: &Mapping, key: &str) -> Option<String> {
    get(map, key).and_then(key_str)
}

fn get_bool(map: &Mapping, key: &str) -> bool {
    matches!(get(map, key), Some(Value::Bool(true)))
}

/// A scalar or a list of scalars; `exclude:` entries (in `sources:`) render as `!glob`.
fn str_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Mapping(m) => get_str(m, "exclude").map(|g| format!("!{g}")),
                other => key_str(other),
            })
            .collect(),
        Some(other) => key_str(other).into_iter().collect(),
    }
}

impl Loader {
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn load(&mut self, path: &Path, doc: &Value, scope: &Scope) {
        let Value::Mapping(top) = doc else {
            self.warnings
                .push(format!("{}: not a YAML mapping", self.rel(path)));
            return;
        };
        self.stack.push(path.to_path_buf());

        match get_str(top, "version").as_deref() {
            Some(v) if v == "3" || v.starts_with("3.") => {}
            Some(v) => self.warnings.push(format!(
                "{}: version {v:?} is not go-task v3",
                self.rel(path)
            )),
            None => self
                .warnings
                .push(format!("{}: no `version:` key", self.rel(path))),
        }

        if let Some(Value::Mapping(tasks)) = get(top, "tasks") {
            for (key, def) in tasks {
                let Some(name) = key_str(key) else { continue };
                if scope.excludes.contains(&name) {
                    continue;
                }
                self.add_task(path, &name, def, scope);
            }
        }

        if let Some(Value::Mapping(includes)) = get(top, "includes") {
            for (key, spec) in includes {
                let Some(name) = key_str(key) else { continue };
                self.include(path, &name, spec, scope);
            }
        }

        self.stack.pop();
    }

    fn add_task(&mut self, path: &Path, name: &str, def: &Value, scope: &Scope) {
        let full = join(&scope.namespace, name);
        let mut draft = Draft {
            node: TaskNode {
                name: full.clone(),
                desc: None,
                summary: None,
                aliases: Vec::new(),
                taskfile: self.rel(path),
                internal: scope.internal,
                deps: Vec::new(),
                calls: Vec::new(),
                cmds: Vec::new(),
                sources: Vec::new(),
                generates: Vec::new(),
                status: Vec::new(),
                preconditions: Vec::new(),
                requires: Vec::new(),
                run: None,
                dir: None,
            },
            namespace: scope.namespace.clone(),
            deps: Vec::new(),
            calls: Vec::new(),
        };

        match def {
            Value::String(cmd) => draft.node.cmds.push(cmd.clone()),
            Value::Sequence(cmds) => parse_cmds(cmds, &mut draft),
            Value::Mapping(map) => parse_task_map(map, &mut draft),
            Value::Null => {}
            _ => self
                .warnings
                .push(format!("{full}: unsupported task definition")),
        }

        let mut alternatives: Vec<String> = draft
            .node
            .aliases
            .iter()
            .map(|a| join(&scope.namespace, a))
            .collect();
        for prefix in &scope.alias_prefixes {
            alternatives.push(join(prefix, name));
            alternatives.extend(draft.node.aliases.iter().map(|a| join(prefix, a)));
        }
        for alt in alternatives {
            self.aliases.entry(alt).or_insert_with(|| full.clone());
        }

        if self.drafts.contains_key(&full) {
            self.warnings.push(format!(
                "{full}: defined more than once (go-task refuses this); keeping the first"
            ));
            return;
        }
        self.drafts.insert(full, draft);
    }

    fn include(&mut self, path: &Path, name: &str, spec: &Value, parent: &Scope) {
        let (taskfile, map) = match spec {
            Value::String(s) => (Some(s.clone()), None),
            Value::Mapping(m) => (get_str(m, "taskfile"), Some(m)),
            _ => (None, None),
        };
        let flag = |key: &str| map.is_some_and(|m| get_bool(m, key));
        let optional = flag("optional");
        let flatten = flag("flatten");
        let internal = parent.internal || flag("internal");
        let include_aliases = map.map(|m| str_list(get(m, "aliases"))).unwrap_or_default();
        let excludes = map
            .map(|m| str_list(get(m, "excludes")))
            .unwrap_or_default();

        let namespace = if flatten {
            parent.namespace.clone()
        } else {
            join(&parent.namespace, name)
        };
        let mut record = IncludeNode {
            namespace: namespace.clone(),
            taskfile: taskfile.clone().unwrap_or_default(),
            resolved: false,
            remote: false,
            optional,
            flatten,
            internal,
        };

        let Some(raw) = taskfile else {
            self.warnings
                .push(format!("include {namespace:?}: no `taskfile:`"));
            self.includes.push(record);
            return;
        };
        if is_remote(&raw) {
            record.remote = true;
            self.warnings.push(format!(
                "include {namespace:?}: remote Taskfile {raw} is not fetched; its tasks are not in the graph"
            ));
            self.includes.push(record);
            return;
        }

        let base = path.parent().unwrap_or(Path::new("/"));
        let substituted = raw
            .replace("{{.ROOT_DIR}}", &self.root_dir.display().to_string())
            .replace("{{.TASKFILE_DIR}}", &base.display().to_string());
        if substituted.contains("{{") {
            self.warnings.push(format!(
                "include {namespace:?}: templated path {raw} cannot be resolved statically"
            ));
            self.includes.push(record);
            return;
        }
        let candidate = base.join(&substituted);
        let file = if candidate.is_dir() {
            taskfile_in(&candidate)
        } else if candidate.is_file() {
            Some(candidate.clone())
        } else {
            None
        };
        let Some(file) = file.and_then(|f| std::fs::canonicalize(f).ok()) else {
            if !optional {
                self.warnings.push(format!(
                    "include {namespace:?}: {} does not exist",
                    self.rel(&candidate)
                ));
            }
            self.includes.push(record);
            return;
        };
        record.taskfile = self.rel(&file);
        if self.stack.contains(&file) {
            self.warnings.push(format!(
                "include {namespace:?}: {} includes itself",
                record.taskfile
            ));
            self.includes.push(record);
            return;
        }
        let doc = match read_yaml(&file) {
            Ok(doc) => doc,
            Err(e) => {
                self.warnings.push(format!("include {namespace:?}: {e}"));
                self.includes.push(record);
                return;
            }
        };
        record.resolved = true;
        self.includes.push(record);

        let mut alias_prefixes = Vec::new();
        if !flatten {
            alias_prefixes.extend(parent.alias_prefixes.iter().map(|p| join(p, name)));
            for alias in &include_aliases {
                alias_prefixes.push(join(&parent.namespace, alias));
                alias_prefixes.extend(parent.alias_prefixes.iter().map(|p| join(p, alias)));
            }
        } else {
            alias_prefixes.extend(parent.alias_prefixes.iter().cloned());
        }
        let scope = Scope {
            namespace,
            alias_prefixes,
            internal,
            excludes,
        };
        self.load(&file, &doc, &scope);
    }

    fn resolve(&self, namespace: &str, reference: &str) -> TaskRef {
        if reference.contains("{{") {
            return TaskRef {
                name: reference.to_string(),
                resolved: false,
            };
        }
        let wanted = match reference.strip_prefix(':') {
            Some(root_relative) => root_relative.to_string(),
            None => join(namespace, reference),
        };
        if self.drafts.contains_key(&wanted) {
            return TaskRef {
                name: wanted,
                resolved: true,
            };
        }
        match self.aliases.get(&wanted) {
            Some(canonical) => TaskRef {
                name: canonical.clone(),
                resolved: true,
            },
            None => TaskRef {
                name: wanted,
                resolved: false,
            },
        }
    }

    fn finish(mut self, root: &Path, host: &str) -> Graph {
        let mut tasks = Vec::with_capacity(self.drafts.len());
        for draft in self.drafts.values() {
            let mut node = draft.node.clone();
            node.deps = draft
                .deps
                .iter()
                .map(|r| self.resolve(&draft.namespace, r))
                .collect();
            node.calls = draft
                .calls
                .iter()
                .map(|r| self.resolve(&draft.namespace, r))
                .collect();
            for r in node.deps.iter().chain(&node.calls) {
                if !r.resolved && !r.name.contains("{{") {
                    self.warnings.push(format!(
                        "{}: references unknown task {:?}",
                        node.name, r.name
                    ));
                }
            }
            tasks.push(node);
        }

        let digest = {
            let model = serde_json::to_vec(&(&tasks, &self.includes)).unwrap_or_default();
            hex_prefix(&Sha256::digest(&model), 16)
        };
        Graph {
            id: graph_id(host, root),
            taskfile: root.display().to_string(),
            host: host.to_string(),
            digest,
            tasks,
            includes: self.includes,
            warnings: self.warnings,
        }
    }
}

fn is_remote(path: &str) -> bool {
    ["http://", "https://", "git@", "git://", "ssh://"]
        .iter()
        .any(|scheme| path.starts_with(scheme))
}

fn parse_task_map(map: &Mapping, draft: &mut Draft) {
    let node = &mut draft.node;
    node.desc = get_str(map, "desc");
    node.summary = get_str(map, "summary");
    node.aliases = str_list(get(map, "aliases"));
    node.internal |= get_bool(map, "internal");
    node.sources = str_list(get(map, "sources"));
    node.generates = str_list(get(map, "generates"));
    node.status = str_list(get(map, "status"));
    node.run = get_str(map, "run");
    node.dir = get_str(map, "dir");
    node.preconditions = match get(map, "preconditions") {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Mapping(m) => get_str(m, "sh"),
                other => key_str(other),
            })
            .collect(),
        other => str_list(other),
    };
    if let Some(Value::Mapping(requires)) = get(map, "requires")
        && let Some(Value::Sequence(vars)) = get(requires, "vars")
    {
        node.requires = vars
            .iter()
            .filter_map(|v| match v {
                Value::Mapping(m) => get_str(m, "name"),
                other => key_str(other),
            })
            .collect();
    }

    match get(map, "deps") {
        Some(Value::Sequence(deps)) => {
            for dep in deps {
                match dep {
                    Value::Mapping(m) => draft.deps.extend(get_str(m, "task")),
                    other => draft.deps.extend(key_str(other)),
                }
            }
        }
        Some(other) => draft.deps.extend(key_str(other)),
        None => {}
    }

    match get(map, "cmds") {
        Some(Value::Sequence(cmds)) => parse_cmds(cmds, draft),
        Some(Value::String(cmd)) => draft.node.cmds.push(cmd.clone()),
        _ => {}
    }
    // Pre-3.x shorthand still accepted by go-task: a single `cmd:`.
    if let Some(cmd) = get_str(map, "cmd") {
        draft.node.cmds.push(cmd);
    }
}

fn parse_cmds(cmds: &[Value], draft: &mut Draft) {
    for item in cmds {
        match item {
            Value::Mapping(m) => {
                if let Some(task) = get_str(m, "task") {
                    draft.calls.push(task);
                } else if let Some(cmd) = get_str(m, "cmd") {
                    draft.node.cmds.push(cmd);
                } else if let Some(deferred) = get(m, "defer") {
                    match deferred {
                        Value::Mapping(d) => draft.calls.extend(get_str(d, "task")),
                        other => draft
                            .node
                            .cmds
                            .extend(key_str(other).map(|c| format!("defer: {c}"))),
                    }
                }
            }
            other => draft.node.cmds.extend(key_str(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory tree; removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("taskgraph-{}", uuid::Uuid::now_v7()));
            for (rel, body) in files {
                let path = dir.join(rel);
                std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
                std::fs::write(path, body).expect("write");
            }
            Self(dir)
        }
        fn root(&self) -> PathBuf {
            self.0.join("Taskfile.yml")
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn task<'g>(graph: &'g Graph, name: &str) -> &'g TaskNode {
        graph
            .tasks
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no task {name}"))
    }

    fn refs(list: &[TaskRef]) -> Vec<(&str, bool)> {
        list.iter().map(|r| (r.name.as_str(), r.resolved)).collect()
    }

    /// References inside an included file are relative to its namespace, `:x`
    /// escapes to the root, a flattened include adds no prefix, and task/include
    /// aliases resolve to the canonical name — the rules go-task applies.
    #[test]
    fn includes_flatten_into_namespaces_and_references_resolve_like_go_task() {
        let tree = Tree::new(&[
            (
                "Taskfile.yml",
                r#"
version: '3'
includes:
  rust: { taskfile: ./scripts/rust.yml, aliases: [r] }
  web: ./web
  flat: { taskfile: ./flat.yml, flatten: true }
tasks:
  check:
    deps: [r:lint, web:build]
    cmds:
      - task: fmt
      - echo done
  fmt: cargo fmt
"#,
            ),
            (
                "scripts/rust.yml",
                r#"
version: '3'
tasks:
  lint:
    desc: clippy
    aliases: [l]
    deps: [build, ':fmt']
    cmds: [cargo clippy]
  build:
    sources: ['src/**/*.rs', { exclude: target/** }]
    cmds: [cargo build]
"#,
            ),
            (
                "web/Taskfile.yml",
                "version: '3'\ntasks:\n  build:\n    deps: [':rust:l']\n    cmds: [bun run build]\n",
            ),
            (
                "flat.yml",
                "version: '3'\ntasks:\n  flat-task: [echo flat]\n",
            ),
        ]);
        let graph = parse(&tree.root(), "h").expect("parse");

        assert_eq!(
            graph
                .tasks
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            [
                "check",
                "flat-task",
                "fmt",
                "rust:build",
                "rust:lint",
                "web:build"
            ]
        );
        let check = task(&graph, "check");
        assert_eq!(
            refs(&check.deps),
            [("rust:lint", true), ("web:build", true)]
        );
        assert_eq!(refs(&check.calls), [("fmt", true)]);
        assert_eq!(check.cmds, ["echo done"]);

        let lint = task(&graph, "rust:lint");
        assert_eq!(refs(&lint.deps), [("rust:build", true), ("fmt", true)]);
        assert_eq!(lint.taskfile, "scripts/rust.yml");
        assert_eq!(refs(&task(&graph, "web:build").deps), [("rust:lint", true)]);
        assert_eq!(
            task(&graph, "rust:build").sources,
            ["src/**/*.rs", "!target/**"]
        );
        assert!(graph.warnings.is_empty(), "{:?}", graph.warnings);
        assert!(graph.includes.iter().all(|i| i.resolved));
    }

    /// Unresolvable inputs degrade to warnings and unresolved refs; the rest of
    /// the graph survives.
    #[test]
    fn remote_templated_missing_and_unknown_degrade_to_warnings() {
        let tree = Tree::new(&[(
            "Taskfile.yml",
            r#"
version: '3'
includes:
  remote: https://example.com/Taskfile.yml
  tmpl: ./{{.DIR}}/Taskfile.yml
  gone: ./gone.yml
  maybe: { taskfile: ./maybe.yml, optional: true }
tasks:
  a:
    deps: [nope, '{{.DYNAMIC}}']
"#,
        )]);
        let graph = parse(&tree.root(), "h").expect("parse");

        assert_eq!(
            refs(&task(&graph, "a").deps),
            [("nope", false), ("{{.DYNAMIC}}", false)]
        );
        let remote = graph
            .includes
            .iter()
            .find(|i| i.namespace == "remote")
            .expect("remote");
        assert!(remote.remote && !remote.resolved);
        assert!(graph.includes.iter().all(|i| !i.resolved));
        // remote, templated, missing non-optional, unknown task; NOT the optional one.
        assert_eq!(graph.warnings.len(), 4, "{:#?}", graph.warnings);
    }

    #[test]
    fn graph_id_is_stable_per_host_and_path_and_digest_tracks_content() {
        let tree = Tree::new(&[("Taskfile.yml", "version: '3'\ntasks:\n  a: [echo a]\n")]);
        let first = parse(&tree.root(), "h").expect("parse");
        std::fs::write(tree.root(), "version: '3'\ntasks:\n  a: [echo b]\n").expect("write");
        let edited = parse(&tree.root(), "h").expect("parse");
        let other_host = parse(&tree.root(), "h2").expect("parse");

        assert_eq!(first.id, edited.id);
        assert_ne!(first.digest, edited.digest);
        assert_ne!(first.id, other_host.id);
    }
}
