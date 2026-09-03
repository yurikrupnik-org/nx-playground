//! `butler container verify` — the gate that keeps the inferred nx
//! `container`/`scan` targets honest.
//!
//! The image facts live in `butler.toml` (see [`crate::settings`]) and have two
//! readers: [`crate::tilt`], which builds the Tiltfile's `docker_build`, and
//! `tools/nx/container-targets.ts`, which infers the targets CI runs. A TS mirror
//! of a Rust resolution is exactly the kind of copy that drifts, so this command
//! recomputes the expected facts here and diffs them against an `nx graph
//! --file` dump. Nothing reads the graph as a *source*: it is only ever the
//! thing being checked.
//!
//! Among the verified facts is the build stage. It has to be: `buildx` with no
//! `target` builds a Dockerfile's *last* stage, and the web Dockerfile's last
//! stage is a static file server with no `/api` reverse proxy. An image built
//! from it serves the SPA and silently drops every API call the Deployments
//! configure a proxy upstream for — a failure no build log shows.
//!
//! Every disagreement is reported, not just the first — a drifted plugin usually
//! drifts for every app at once, and fixing them one round-trip at a time is
//! how a check like this gets switched off.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eyre::{Result, bail, eyre};
use serde::Deserialize;

use crate::graph::ProjectGraph;
use crate::settings::{self, AppOverrides, Root};
use crate::tilt;

// ---------------------------------------------------------------------------
// Expected: butler's own resolution

/// Everything the `container`/`scan` pair must carry for one app, in the exact
/// spelling the graph dump uses (workspace-root-relative paths, `KEY=VALUE`
/// build args, `$REGISTRY` unsubstituted).
#[derive(Debug)]
pub struct Expected {
    /// Workspace-root-relative app directory; the key both sides agree on.
    pub dir: String,
    pub file: String,
    pub context: String,
    /// Resolved multi-stage build stage, via [`tilt::resolve_stage`] — the one
    /// place the fallback lives.
    pub stage: String,
    /// `KEY=VALUE` pairs, sorted by key.
    pub build_args: Vec<String>,
    pub tags: Vec<String>,
    pub push: bool,
    /// `["build"]` for a static web app, whose image is its local build output;
    /// empty for a service and for a node app, which both build inside the
    /// image.
    pub container_depends_on: Vec<String>,
    pub scan_depends_on: Vec<String>,
    pub scan_command: String,
    pub scan_ci_command: String,
}

/// The container set: an app with a recognizable kind that either declares a
/// `[workload]` or is named in the root `[container] extra`.
///
/// Declaring a workload is what deploys an app, so it is what makes an image
/// worth building. The manifests are no longer the signal: they are *generated*
/// from that workload (see [`crate::k8s`]), so keying this set on them would
/// make the derivation circular. An app deployed from outside this repo declares
/// no workload and must opt in explicitly — the one fact about this set that
/// cannot be derived.
pub fn deployable_dirs(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
) -> Result<Vec<String>> {
    let extra: BTreeSet<&str> = root.container.extra.iter().map(String::as_str).collect();
    let mut dirs = Vec::new();
    for project in graph.projects.values() {
        let dir = &project.root;
        if tilt::app_kind(workspace_root, dir).is_none() {
            continue;
        }
        let declares_workload = overrides.get(dir).is_some_and(|app| app.workload.is_some());
        if declares_workload || extra.contains(dir.as_str()) {
            dirs.push(dir.clone());
        }
    }
    dirs.sort();

    let missing: Vec<&str> = extra
        .iter()
        .copied()
        .filter(|e| !dirs.iter().any(|d| d == e))
        .collect();
    if !missing.is_empty() {
        bail!(
            "{file} [container] extra lists {}, which is not a discovered app \
             (needs {markers})",
            missing.join(", "),
            file = settings::FILE,
            markers = tilt::KIND_MARKERS,
        );
    }
    Ok(dirs)
}

/// Resolve the expected facts for an explicit list of app directories, so the
/// set and the resolution can be tested apart from each other.
pub fn expected_apps(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
    dirs: &[String],
) -> Result<Vec<Expected>> {
    let mut out = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let project = graph
            .projects
            .values()
            .find(|p| p.root == *dir)
            .ok_or_else(|| eyre!("{dir} is not a discovered project"))?;
        let kind = tilt::app_kind(workspace_root, dir).ok_or_else(|| {
            eyre!(
                "{dir}: no {markers}; unsupported app kind",
                markers = tilt::KIND_MARKERS
            )
        })?;
        let declared = overrides.get(dir).and_then(|a| a.image.as_ref());
        let facts = tilt::image_facts(root, &kind, dir, project, declared)?;
        // Resolved through tilt's helper, so CI's `target` and the Tiltfile's
        // can never be resolved two different ways.
        let stage = tilt::resolve_stage(root, &facts, dir)?;

        let repository = repository(&facts.tag);
        // Bare image name: the layer-cache ref and the SARIF filename need it.
        let name = repository.rsplit('/').next().unwrap_or(repository);

        out.push(Expected {
            dir: dir.clone(),
            file: facts.dockerfile,
            context: facts.context,
            stage,
            build_args: facts
                .build_args
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect(),
            tags: vec![facts.tag.clone()],
            push: false,
            container_depends_on: match kind {
                tilt::Kind::Web => vec!["build".to_string()],
                // Self-contained images: the service compiles and the node app
                // installs plus builds inside the Dockerfile.
                tilt::Kind::Service | tilt::Kind::Node => Vec::new(),
            },
            scan_depends_on: vec!["container".to_string()],
            scan_command: format!(
                "trivy image --cache-backend memory {} --severity CRITICAL,HIGH --exit-code 0",
                facts.tag
            ),
            scan_ci_command: format!(
                "trivy image --cache-backend memory {repository}:sha-$(echo $SHORT_SHA | \
                 cut -c1-7) --severity CRITICAL,HIGH --format sarif --output trivy-{name}.sarif"
            ),
        });
    }
    Ok(out)
}

/// `$REGISTRY/zerg-api:latest` -> `$REGISTRY/zerg-api`. A port in a registry
/// host (`localhost:5000/x`) is not a tag.
fn repository(tag: &str) -> &str {
    match tag.rsplit_once(':') {
        Some((base, t)) if !t.contains('/') => base,
        _ => tag,
    }
}

// ---------------------------------------------------------------------------
// Actual: an `nx graph --file` dump
//
// Only the fields this check compares are modelled; everything else in the dump
// is ignored, which is what keeps it stable across nx versions.

#[derive(Debug, Deserialize)]
pub struct NxGraph {
    graph: GraphBody,
}

#[derive(Debug, Deserialize)]
struct GraphBody {
    #[serde(default)]
    nodes: BTreeMap<String, Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    data: NodeData,
}

#[derive(Debug, Deserialize)]
struct NodeData {
    /// Workspace-root-relative project root: how a node is matched to an app.
    root: String,
    #[serde(default)]
    targets: BTreeMap<String, Target>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Target {
    /// Left as raw JSON: nx allows object entries here, and a dump containing
    /// one anywhere must not fail the whole parse.
    #[serde(default)]
    depends_on: Vec<serde_json::Value>,
    #[serde(default)]
    options: Options,
    #[serde(default)]
    configurations: BTreeMap<String, Options>,
}

#[derive(Debug, Deserialize, Default)]
struct Options {
    file: Option<String>,
    context: Option<String>,
    /// Build stage. Absent means buildx builds the Dockerfile's last stage.
    target: Option<String>,
    #[serde(rename = "build-args", default)]
    build_args: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    push: Option<bool>,
    command: Option<String>,
}

impl NxGraph {
    fn node_by_root(&self, root: &str) -> Option<(&str, &NodeData)> {
        self.graph
            .nodes
            .iter()
            .find(|(_, n)| n.data.root == root)
            .map(|(name, n)| (name.as_str(), &n.data))
    }
}

// ---------------------------------------------------------------------------
// Diff

/// Every way the graph disagrees with butler's resolution, as messages naming
/// the app, the field, and both sides. Empty = the plugin is a faithful mirror.
pub fn diff(expected: &[Expected], actual: &NxGraph) -> Vec<String> {
    let mut problems = Vec::new();

    for want in expected {
        let dir = want.dir.as_str();
        let Some((_, node)) = actual.node_by_root(dir) else {
            problems.push(format!(
                "{dir}: no project in the nx graph has this root, so its image is \
                 never built or scanned"
            ));
            continue;
        };

        match node.targets.get("container") {
            None => problems.push(format!(
                "{dir}: missing a `container` target — the container plugin did not \
                 infer one for a deployable app"
            )),
            Some(target) => {
                let opts = &target.options;
                check(
                    &mut problems,
                    dir,
                    "container.options.file",
                    &want.file,
                    opts.file.as_deref().unwrap_or(ABSENT),
                );
                check(
                    &mut problems,
                    dir,
                    "container.options.context",
                    &want.context,
                    opts.context.as_deref().unwrap_or(ABSENT),
                );
                check(
                    &mut problems,
                    dir,
                    "container.options.target",
                    &want.stage,
                    opts.target.as_deref().unwrap_or(ABSENT),
                );
                check(
                    &mut problems,
                    dir,
                    "container.options.build-args",
                    &list(&want.build_args),
                    &list(&opts.build_args),
                );
                check(
                    &mut problems,
                    dir,
                    "container.options.tags",
                    &list(&want.tags),
                    &list(&opts.tags),
                );
                check(
                    &mut problems,
                    dir,
                    "container.options.push",
                    want.push.to_string().as_str(),
                    opts.push
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| ABSENT.to_string())
                        .as_str(),
                );
                check(
                    &mut problems,
                    dir,
                    "container.dependsOn",
                    &list(&want.container_depends_on),
                    &list(&depends_on(target)),
                );
            }
        }

        match node.targets.get("scan") {
            None => problems.push(format!(
                "{dir}: missing a `scan` target — its image would be built but never \
                 scanned for CVEs"
            )),
            Some(target) => {
                check(
                    &mut problems,
                    dir,
                    "scan.options.command",
                    &want.scan_command,
                    target.options.command.as_deref().unwrap_or(ABSENT),
                );
                let ci = target
                    .configurations
                    .get("ci")
                    .and_then(|o| o.command.as_deref())
                    .unwrap_or(ABSENT);
                check(
                    &mut problems,
                    dir,
                    "scan.configurations.ci.command",
                    &want.scan_ci_command,
                    ci,
                );
                check(
                    &mut problems,
                    dir,
                    "scan.dependsOn",
                    &list(&want.scan_depends_on),
                    &list(&depends_on(target)),
                );
            }
        }
    }

    // The other direction: an image nothing asked for is as wrong as a missing
    // one — it means the plugin's app set is wider than butler's.
    let wanted: BTreeSet<&str> = expected.iter().map(|e| e.dir.as_str()).collect();
    for (name, node) in &actual.graph.nodes {
        let root = node.data.root.as_str();
        if node.data.targets.contains_key("container") && !wanted.contains(root) {
            // The node name is usually the cargo package, not the directory.
            let who = if name == root {
                root.to_string()
            } else {
                format!("{name} ({root})")
            };
            problems.push(format!(
                "{who}: has a `container` target but is not a deployable app — \
                 give it k8s manifests or list it in the root {file} [container] extra",
                file = settings::FILE
            ));
        }
    }

    problems
}

/// Stand-in for a field the graph never emitted, so a message reads as a
/// difference instead of an empty string.
const ABSENT: &str = "(absent)";

fn check(problems: &mut Vec<String>, dir: &str, field: &str, want: &str, got: &str) {
    if want != got {
        problems.push(format!("{dir} {field}: expected {want}, graph has {got}"));
    }
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

/// `dependsOn` as strings; a non-string entry keeps its JSON spelling so the
/// mismatch message still shows what the graph carried.
fn depends_on(target: &Target) -> Vec<String> {
    target
        .depends_on
        .iter()
        .map(|d| match d {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Command

/// Diff a graph dump against butler's resolution; non-zero on any disagreement.
pub fn verify(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
    graph_file: &Path,
) -> Result<()> {
    let dirs = deployable_dirs(workspace_root, graph, root, overrides)?;
    let expected = expected_apps(workspace_root, graph, root, overrides, &dirs)?;

    let raw = std::fs::read_to_string(graph_file).map_err(|e| {
        eyre!(
            "reading {}: {e}\nproduce it with `nx graph --file <path>`",
            graph_file.display()
        )
    })?;
    let actual: NxGraph =
        serde_json::from_str(&raw).map_err(|e| eyre!("parsing {}: {e}", graph_file.display()))?;

    let problems = diff(&expected, &actual);
    if !problems.is_empty() {
        bail!(
            "the nx container/scan targets disagree with {file} in {n} place(s):\n  {list}\n\
             fix tools/nx/container-targets.ts, or {file} if the graph is right",
            n = problems.len(),
            list = problems.join("\n  "),
            file = settings::FILE
        );
    }
    println!(
        "container verify: {} apps match {}",
        expected.len(),
        settings::FILE
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A service app's expected facts, spelled as the plugin must emit them.
    fn service(dir: &str, package: &str, image: &str) -> Expected {
        Expected {
            dir: dir.into(),
            file: "manifests/dockers/rust.Dockerfile".into(),
            context: ".".into(),
            stage: "rust".into(),
            build_args: vec![format!("APP_NAME={package}")],
            tags: vec![format!("$REGISTRY/{image}:latest")],
            push: false,
            container_depends_on: Vec::new(),
            scan_depends_on: vec!["container".into()],
            scan_command: format!(
                "trivy image --cache-backend memory $REGISTRY/{image}:latest \
                 --severity CRITICAL,HIGH --exit-code 0"
            ),
            scan_ci_command: format!(
                "trivy image --cache-backend memory $REGISTRY/{image}:sha-$(echo $SHORT_SHA | \
                 cut -c1-7) --severity CRITICAL,HIGH --format sarif --output trivy-{image}.sarif"
            ),
        }
    }

    /// One graph node carrying the pair the plugin infers.
    fn node(name: &str, dir: &str, package: &str, image: &str, extra_targets: &str) -> String {
        let want = service(dir, package, image);
        node_with_stage(name, dir, package, image, &want.stage, extra_targets)
    }

    /// The same node with the build stage forced, so a `target` the plugin got
    /// wrong can be exercised without disturbing any other field.
    fn node_with_stage(
        name: &str,
        dir: &str,
        package: &str,
        image: &str,
        stage: &str,
        extra_targets: &str,
    ) -> String {
        let want = service(dir, package, image);
        format!(
            r#""{name}": {{ "name": "{name}", "type": "app", "data": {{
                "root": "{dir}",
                "targets": {{
                  "container": {{
                    "executor": "@nx-tools/nx-container:build",
                    "options": {{
                      "file": "{file}",
                      "context": ".",
                      "target": "{stage}",
                      "build-args": ["APP_NAME={package}"],
                      "tags": ["$REGISTRY/{image}:latest"],
                      "push": false
                    }}
                  }},
                  "scan": {{
                    "executor": "nx:run-commands",
                    "dependsOn": ["container"],
                    "options": {{ "command": "{scan}", "cwd": "{{workspaceRoot}}" }},
                    "configurations": {{ "ci": {{ "command": "{scan_ci}" }} }}
                  }}{extra_targets}
                }}
              }} }}"#,
            file = want.file,
            scan = want.scan_command,
            scan_ci = want.scan_ci_command,
        )
    }

    fn parse(nodes: &[String]) -> NxGraph {
        let raw = format!(
            r#"{{ "graph": {{ "nodes": {{ {} }}, "dependencies": {{}} }} }}"#,
            nodes.join(",")
        );
        serde_json::from_str(&raw).expect("graph fixture parses")
    }

    #[test]
    fn a_faithful_graph_has_no_problems() {
        let expected = vec![
            service("apps/zerg/api", "zerg_api", "zerg-api"),
            service("apps/todo/worker", "todo_worker", "todo-worker"),
        ];
        let actual = parse(&[
            node("zerg_api", "apps/zerg/api", "zerg_api", "zerg-api", ""),
            node(
                "todo_worker",
                "apps/todo/worker",
                "todo_worker",
                "todo-worker",
                "",
            ),
        ]);
        assert_eq!(diff(&expected, &actual), Vec::<String>::new());
    }

    #[test]
    fn a_wrong_build_arg_names_the_app_and_the_field() {
        let expected = vec![service("apps/zerg/api", "zerg_api", "zerg-api")];
        // The classic drift: the image name used where the crate name belongs.
        let actual = parse(&[node(
            "zerg_api",
            "apps/zerg/api",
            "zerg-api",
            "zerg-api",
            "",
        )]);
        assert_eq!(
            diff(&expected, &actual),
            vec![
                "apps/zerg/api container.options.build-args: expected [APP_NAME=zerg_api], \
                 graph has [APP_NAME=zerg-api]"
            ]
        );
    }

    #[test]
    fn a_wrong_build_stage_is_reported() {
        let expected = vec![service("apps/zerg/api", "zerg_api", "zerg-api")];
        // The stage the web Dockerfile ends on, which serves the SPA without the
        // `/api` reverse proxy: a working build and a broken deploy.
        let actual = parse(&[node_with_stage(
            "zerg_api",
            "apps/zerg/api",
            "zerg_api",
            "zerg-api",
            "static-web-server",
            "",
        )]);
        assert_eq!(
            diff(&expected, &actual),
            vec![
                "apps/zerg/api container.options.target: expected rust, graph has \
                 static-web-server"
            ]
        );
    }

    #[test]
    fn a_missing_scan_target_is_reported() {
        let expected = vec![service("apps/todo/api", "todo_api", "todo-api")];
        let raw = format!(
            r#"{{ "graph": {{ "nodes": {{ "todo_api": {{ "data": {{
                 "root": "apps/todo/api",
                 "targets": {{ "container": {{ "options": {{
                   "file": "{file}", "context": ".", "target": "{stage}",
                   "build-args": ["APP_NAME=todo_api"],
                   "tags": ["$REGISTRY/todo-api:latest"], "push": false
                 }} }} }}
               }} }} }} }} }}"#,
            file = expected[0].file,
            stage = expected[0].stage,
        );
        let actual: NxGraph = serde_json::from_str(&raw).expect("fixture parses");
        let problems = diff(&expected, &actual);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("apps/todo/api: missing a `scan` target"),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_container_target_outside_the_app_set_is_reported() {
        let expected = vec![service("apps/zerg/api", "zerg_api", "zerg-api")];
        let actual = parse(&[
            node("zerg_api", "apps/zerg/api", "zerg_api", "zerg-api", ""),
            // Not deployable: no manifests, not in `[container] extra`.
            node("todo_cli", "apps/todo/cli", "todo_cli", "todo-cli", ""),
        ]);
        let problems = diff(&expected, &actual);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0]
                .starts_with("todo_cli (apps/todo/cli): has a `container` target but is not"),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_web_app_must_depend_on_its_local_build() {
        let mut want = service("apps/todo/web", "unused", "todo-web");
        want.container_depends_on = vec!["build".into()];
        let expected = vec![want];
        // Service-shaped node: no `dependsOn`, so the image could be built
        // before the SPA output exists.
        let actual = parse(&[node("todo-web", "apps/todo/web", "unused", "todo-web", "")]);
        assert_eq!(
            diff(&expected, &actual),
            vec!["apps/todo/web container.dependsOn: expected [build], graph has []"]
        );
    }

    /// A node app's expected facts: the app directory as the build arg, and no
    /// local `build` to wait for.
    fn node_app(dir: &str, image: &str) -> Expected {
        let mut want = service(dir, "unused", image);
        want.file = "manifests/dockers/node.Dockerfile".into();
        want.stage = "runtime".into();
        want.build_args = vec![format!("APP_DIR={dir}")];
        want.container_depends_on = Vec::new();
        want
    }

    /// One graph node for a node app, with `container.dependsOn` supplied so the
    /// self-contained-image contract can be exercised in both directions.
    fn node_app_graph_node(dir: &str, image: &str, container_depends_on: &str) -> String {
        let want = node_app(dir, image);
        format!(
            r#""{image}": {{ "name": "{image}", "type": "app", "data": {{
                "root": "{dir}",
                "targets": {{
                  "container": {{
                    "executor": "@nx-tools/nx-container:build",
                    "dependsOn": [{container_depends_on}],
                    "options": {{
                      "file": "{file}",
                      "context": ".",
                      "target": "{stage}",
                      "build-args": ["APP_DIR={dir}"],
                      "tags": ["$REGISTRY/{image}:latest"],
                      "push": false
                    }}
                  }},
                  "scan": {{
                    "executor": "nx:run-commands",
                    "dependsOn": ["container"],
                    "options": {{ "command": "{scan}", "cwd": "{{workspaceRoot}}" }},
                    "configurations": {{ "ci": {{ "command": "{scan_ci}" }} }}
                  }}
                }}
              }} }}"#,
            file = want.file,
            stage = want.stage,
            scan = want.scan_command,
            scan_ci = want.scan_ci_command,
        )
    }

    #[test]
    fn a_node_app_waits_for_no_local_build() {
        let expected = vec![node_app("apps/todo/web-astro", "todo-web-astro")];
        let actual = parse(&[node_app_graph_node(
            "apps/todo/web-astro",
            "todo-web-astro",
            "",
        )]);
        assert_eq!(diff(&expected, &actual), Vec::<String>::new());
    }

    #[test]
    fn a_node_app_depending_on_a_local_build_is_reported() {
        let expected = vec![node_app("apps/todo/web-astro", "todo-web-astro")];
        // Web-shaped node: the image installs and builds inside the Dockerfile,
        // so an nx `build` in front of it is a second, wasted build.
        let actual = parse(&[node_app_graph_node(
            "apps/todo/web-astro",
            "todo-web-astro",
            r#""build""#,
        )]);
        assert_eq!(
            diff(&expected, &actual),
            vec!["apps/todo/web-astro container.dependsOn: expected [], graph has [build]"]
        );
    }

    #[test]
    fn repository_ignores_a_registry_port() {
        assert_eq!(
            repository("$REGISTRY/zerg-api:latest"),
            "$REGISTRY/zerg-api"
        );
        assert_eq!(
            repository("localhost:5000/zerg-api"),
            "localhost:5000/zerg-api"
        );
    }
}
