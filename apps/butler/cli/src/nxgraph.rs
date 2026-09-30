//! butler's graph in the `nx graph --file` shape, and `butler graph verify`:
//! the drift gate between butler's inference and nx's.
//!
//! butler builds its graph the way nx does — same layer order, same merge,
//! same normalization (`{workspaceRoot}` tokens resolved, `parallelism`,
//! `nx:noop`, empty `options`/`configurations` spelled out) — so both dumps
//! are compared nearly raw. Per project: `root`, `projectType`, `tags` and
//! `implicitDependencies` (both order-free, so sorted), project-level
//! `namedInputs`, and every target key for key; plus the project-to-project
//! edges with their kind (static, dynamic, implicit).
//!
//! Deliberately not compared, because none of it decides what a task runs or
//! when it is stale:
//! - `metadata`, on projects and targets: descriptions, UI hints, and nx's
//!   package metadata;
//! - the node `type` (`app`/`lib`/`e2e`), which nx guesses from names and
//!   tsconfig files;
//! - `sourceRoot`, `$schema` and `release` (nx release's configuration —
//!   butler does not release);
//! - edges to external nodes (`npm:*`, `cargo:*`), which butler does not
//!   model, and each edge's `sourceFile`.
//!
//! Targets a third-party nx plugin contributes for `nx release`, which butler
//! neither runs nor models, are listed in `butler.toml`
//! `[graph] verifyIgnoreTargets`; no other target is skipped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eyre::{Result, WrapErr, bail};
use serde_json::{Map, Value, json};

use crate::graph::ProjectGraph;

/// `butler graph --json`: the graph as `nx graph --file` writes it.
pub fn to_nx_dump(graph: &ProjectGraph) -> Result<Value> {
    let mut nodes = Map::new();
    let mut dependencies = Map::new();
    for p in graph.projects.values() {
        let node_type = match p.project_type.as_deref() {
            Some("application") => "app",
            _ => "lib",
        };
        let mut data = Map::new();
        data.insert("root".into(), json!(p.root));
        data.insert("name".into(), json!(p.name));
        if let Some(t) = &p.project_type {
            data.insert("projectType".into(), json!(t));
        }
        data.insert("tags".into(), json!(p.tags));
        data.insert(
            "implicitDependencies".into(),
            json!(p.implicit_dependencies),
        );
        data.insert("targets".into(), serde_json::to_value(&p.targets)?);
        nodes.insert(
            p.name.clone(),
            json!({"name": p.name, "type": node_type, "data": data}),
        );
        dependencies.insert(p.name.clone(), Value::Array(Vec::new()));
    }
    for e in &graph.edges {
        if let Some(Value::Array(list)) = dependencies.get_mut(&e.source) {
            list.push(json!({"source": e.source, "target": e.target, "type": e.kind.as_str()}));
        }
    }
    Ok(json!({"graph": {"nodes": nodes, "dependencies": dependencies}}))
}

/// Canonical form of one side, for comparison.
#[derive(Debug, Default)]
struct Canon {
    /// name -> (field -> value), targets flattened as `target:<name>`.
    projects: BTreeMap<String, BTreeMap<String, Value>>,
    edges: BTreeSet<String>,
}

fn canon(dump: &Value, ignore_targets: &BTreeSet<String>) -> Result<Canon> {
    let nodes = dump
        .pointer("/graph/nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| eyre::eyre!("not an nx graph dump: no graph.nodes"))?;
    let mut out = Canon::default();
    for (name, node) in nodes {
        let data = node
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| eyre::eyre!("node {name}: no data"))?;
        let mut fields = BTreeMap::new();
        fields.insert(
            "root".into(),
            data.get("root").cloned().unwrap_or(Value::Null),
        );
        for key in ["projectType", "namedInputs"] {
            if let Some(v) = data.get(key) {
                fields.insert(key.into(), v.clone());
            }
        }
        fields.insert("tags".into(), sorted(data.get("tags")));
        fields.insert(
            "implicitDependencies".into(),
            sorted(data.get("implicitDependencies")),
        );
        if let Some(targets) = data.get("targets").and_then(Value::as_object) {
            for (tname, t) in targets {
                if ignore_targets.contains(tname) {
                    continue;
                }
                let mut t = t.clone();
                if let Some(o) = t.as_object_mut() {
                    o.remove("metadata");
                }
                fields.insert(format!("target:{tname}"), t);
            }
        }
        out.projects.insert(name.clone(), fields);
    }
    if let Some(deps) = dump
        .pointer("/graph/dependencies")
        .and_then(Value::as_object)
    {
        for edges in deps.values() {
            for e in edges.as_array().into_iter().flatten() {
                let (Some(s), Some(t)) = (
                    e.get("source").and_then(Value::as_str),
                    e.get("target").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !nodes.contains_key(t) {
                    continue;
                }
                let kind = e.get("type").and_then(Value::as_str).unwrap_or("static");
                out.edges.insert(format!("{s} -> {t} ({kind})"));
            }
        }
    }
    Ok(out)
}

fn sorted(v: Option<&Value>) -> Value {
    let mut list: Vec<String> = v
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string))
        .collect();
    list.sort();
    json!(list)
}

/// One human-readable line per disagreement between `nx` and `butler`.
fn diff(nx: &Canon, butler: &Canon) -> Vec<String> {
    let mut out = Vec::new();
    let names: BTreeSet<&String> = nx.projects.keys().chain(butler.projects.keys()).collect();
    for name in names {
        match (nx.projects.get(name), butler.projects.get(name)) {
            (Some(_), None) => out.push(format!("project {name}: only in nx")),
            (None, Some(_)) => out.push(format!("project {name}: only in butler")),
            (Some(a), Some(b)) => {
                let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
                for key in keys {
                    let at = format!("{name} {key}");
                    match (a.get(key), b.get(key)) {
                        (Some(_), None) => out.push(format!("{at}: only in nx")),
                        (None, Some(_)) => out.push(format!("{at}: only in butler")),
                        (Some(x), Some(y)) => diff_value(&at, x, y, &mut out),
                        (None, None) => {}
                    }
                }
            }
            (None, None) => {}
        }
    }
    for e in nx.edges.difference(&butler.edges) {
        out.push(format!("edge {e}: only in nx"));
    }
    for e in butler.edges.difference(&nx.edges) {
        out.push(format!("edge {e}: only in butler"));
    }
    out
}

/// Recurse into objects so a mismatch names the exact option, not the target.
fn diff_value(at: &str, nx: &Value, butler: &Value, out: &mut Vec<String>) {
    if nx == butler {
        return;
    }
    if let (Some(a), Some(b)) = (nx.as_object(), butler.as_object()) {
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for k in keys {
            let path = format!("{at}.{k}");
            match (a.get(k), b.get(k)) {
                (Some(x), Some(y)) => diff_value(&path, x, y, out),
                (Some(x), None) => out.push(format!("{path}: nx={x} butler=<absent>")),
                (None, Some(y)) => out.push(format!("{path}: nx=<absent> butler={y}")),
                (None, None) => {}
            }
        }
        return;
    }
    out.push(format!("{at}: nx={nx} butler={butler}"));
}

/// `butler graph verify --graph <nx dump>`.
pub fn verify(graph: &ProjectGraph, nx_dump: &Path, ignore_targets: &[String]) -> Result<()> {
    let raw = std::fs::read_to_string(nx_dump)
        .wrap_err_with(|| format!("reading {}", nx_dump.display()))?;
    let nx: Value = serde_json::from_str(&raw).wrap_err("parsing the nx graph dump")?;
    let ignore: BTreeSet<String> = ignore_targets.iter().cloned().collect();
    let expected = canon(&nx, &ignore)?;
    let actual = canon(&to_nx_dump(graph)?, &ignore)?;
    let problems = diff(&expected, &actual);
    if !problems.is_empty() {
        for p in &problems {
            println!("drift: {p}");
        }
        bail!(
            "{} disagreement(s) between butler's graph and nx's",
            problems.len()
        );
    }
    let targets: usize = actual
        .projects
        .values()
        .map(|f| f.keys().filter(|k| k.starts_with("target:")).count())
        .sum();
    println!(
        "graph verify: {} projects, {} targets, {} edges match nx",
        actual.projects.len(),
        targets,
        actual.edges.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(targets: Value, edges: Value) -> Value {
        json!({"graph": {
            "nodes": {
                "a": {"name": "a", "type": "lib", "data": {"root": "libs/a", "tags": ["x", "y"], "targets": targets}},
                "b": {"name": "b", "type": "lib", "data": {"root": "libs/b", "tags": [], "targets": {}}}
            },
            "dependencies": {"a": edges, "b": []}
        }})
    }

    #[test]
    fn metadata_tag_order_and_external_edges_are_not_disagreements() {
        let target = json!({"executor": "nx:run-commands", "options": {"command": "x"},
            "parallelism": true, "configurations": {}});
        let mut nx_target = target.clone();
        nx_target["metadata"] = json!({"description": "d"});
        let nx = dump(
            json!({"lint": nx_target}),
            json!([{"source": "a", "target": "b", "type": "static"},
                   {"source": "a", "target": "npm:vite", "type": "static"}]),
        );
        let mut butler = dump(
            json!({"lint": target}),
            json!([{"source": "a", "target": "b", "type": "static", "sourceFile": "libs/a/Cargo.toml"}]),
        );
        butler["graph"]["nodes"]["a"]["data"]["tags"] = json!(["y", "x"]);
        let none = BTreeSet::new();
        assert!(diff(&canon(&nx, &none).unwrap(), &canon(&butler, &none).unwrap()).is_empty());
    }

    #[test]
    fn a_changed_option_or_edge_kind_is_reported_at_its_own_path() {
        let nx = dump(
            json!({"lint": {"executor": "nx:run-commands", "options": {"command": "x", "cwd": ""}}}),
            json!([{"source": "a", "target": "b", "type": "dynamic"}]),
        );
        let butler = dump(
            json!({"lint": {"executor": "nx:run-commands", "options": {"command": "y", "cwd": ""}}}),
            json!([{"source": "a", "target": "b", "type": "static"}]),
        );
        let none = BTreeSet::new();
        let d = diff(&canon(&nx, &none).unwrap(), &canon(&butler, &none).unwrap());
        assert_eq!(
            d,
            vec![
                r#"a target:lint.options.command: nx="x" butler="y""#.to_string(),
                "edge a -> b (dynamic): only in nx".to_string(),
                "edge a -> b (static): only in butler".to_string(),
            ]
        );
    }

    #[test]
    fn a_spelled_out_default_is_still_a_disagreement() {
        // butler normalizes like nx, so a missing `parallelism` is real drift.
        let nx = dump(
            json!({"lint": {"executor": "nx:noop", "parallelism": true}}),
            json!([]),
        );
        let butler = dump(json!({"lint": {"executor": "nx:noop"}}), json!([]));
        let none = BTreeSet::new();
        let d = diff(&canon(&nx, &none).unwrap(), &canon(&butler, &none).unwrap());
        assert_eq!(
            d,
            vec!["a target:lint.parallelism: nx=true butler=<absent>".to_string()]
        );
    }

    #[test]
    fn ignored_targets_are_skipped_on_both_sides() {
        let nx = dump(
            json!({"nx-release-publish": {"executor": "@nx/js:release-publish"}}),
            json!([]),
        );
        let butler = dump(json!({}), json!([]));
        let ignore = BTreeSet::from(["nx-release-publish".to_string()]);
        assert!(
            diff(
                &canon(&nx, &ignore).unwrap(),
                &canon(&butler, &ignore).unwrap()
            )
            .is_empty()
        );
    }
}
