//! `butler tilt gen` — Tiltfile generator.
//!
//! Config comes from exactly two levels (see [`crate::settings`]): the repo root
//! `butler.toml` and each app's `<app>/butler.toml`. Everything else is derived,
//! never declared:
//!
//!   * image build inputs — the app config's `[image]` when it departs from the
//!     repo convention, else `[imageDefaults.<kind>]`, so the fact has one
//!     home. The nx `container`/`scan` targets are generated from these same
//!     facts by `tools/nx/container-targets.ts` and checked against butler's own
//!     resolution by `butler container verify`, so the Tiltfile and CI cannot
//!     build different images;
//!   * app kind (compiled service, static web app, Node SSR app) and how its
//!     manifests reach the cluster (kustomize overlay vs live `kcl run`) —
//!     detected on disk;
//!   * `docker_build(only=...)` — computed from the butler project graph, so a
//!     service's build context is its own crate plus its transitive workspace
//!     deps and nothing else. Hand-written `only` lists rot, and a too-wide one
//!     makes every unrelated crate edit rebuild every image.
//!
//! No Rust `live_update`: a `FROM scratch` runtime stage has no shell and no
//! cargo, so syncing sources in rebuilds nothing while Tilt reports success. A
//! source change rebuilds the image — which the tight `only=` list makes cheap.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use eyre::{Result, bail, eyre};
use serde::Deserialize;

use crate::graph::{Project, ProjectGraph};
use crate::k8s;
use crate::settings::{self, App, AppOverrides, AppTilt, Root};

// ---------------------------------------------------------------------------
// Resolved app model

/// What kind of build the app needs. Decided by what is on disk, not by a flag
/// someone has to remember to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Cargo crate: image built from the workspace.
    Service,
    /// Vite SPA: built locally by the JS toolchain, image is the static output.
    Web,
    /// Astro config with a server output: the image installs and builds the app
    /// itself, then runs it as a Node process.
    Node,
}

impl Kind {
    /// The kind's name as the root config spells it in
    /// `[imageDefaults.<kind>]` and `[workloadDefaults.<kind>]`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Web => "web",
            Self::Node => "node",
        }
    }
}

/// How the app's objects reach the cluster.
#[derive(Debug)]
pub(crate) enum K8s {
    /// The app declares a `[workload]`: the published `app` KCL package renders
    /// it from the generated values files on every Tilt trigger, which is the
    /// same command `butler k8s gen` commits the output of.
    Package(String),
    /// `<app>/k8s/kustomize/overlays/<env>`.
    Kustomize(String),
    /// `<app>/k8s` is a KCL module of its own, rendered live.
    KclLive { namespace: String, env: String },
    /// The app ships nothing to deploy.
    None,
}

/// Image build inputs, after resolving the single source that owns them.
struct Image {
    reference: String,
    /// Which of the two sources owned these inputs.
    source: &'static str,
    dockerfile: String,
    /// Build context as the Tiltfile must spell it (app-relative).
    context: String,
    stage: String,
    build_args: BTreeMap<String, String>,
}

struct ResolvedApp {
    /// Workspace-root-relative app directory; `.` for a standalone repo.
    dir: String,
    /// Project name in the graph, used for build commands (`nx run <name>:build`).
    project: String,
    /// Tilt/Kubernetes resource name, from the image basename: cargo crates are
    /// `zerg_api` while the workload is `zerg-api`.
    resource: String,
    kind: Kind,
    k8s: K8s,
    /// `../..`-style prefix back to the workspace root; empty for a root app.
    up: String,
    image: Image,
    only: Vec<OnlyGroup>,
    ignore: Vec<String>,
    port_forwards: Option<String>,
    labels: Vec<String>,
    resource_deps: Vec<String>,
    /// `deps=` for a web app's local build resource; empty for a service.
    web_deps: Vec<String>,
    /// Did this app have a `butler.toml` of its own?
    has_app_config: bool,
}

impl ResolvedApp {
    /// A standalone repo's app lives at the root, so its stanzas are inlined
    /// into the root Tiltfile instead of being `include()`d.
    fn is_root_app(&self) -> bool {
        self.dir == "."
    }

    /// Path relative to the app directory, as the app's Tiltfile must spell it.
    fn local(&self, workspace_path: &str) -> String {
        if self.up.is_empty() {
            workspace_path.to_string()
        } else {
            format!("{}/{}", self.up, workspace_path)
        }
    }

    /// Human-readable provenance for the generated header — every file that fed
    /// this Tiltfile, so a reader knows where to make a change.
    fn config_sources(&self) -> String {
        let mut parts = Vec::new();
        if self.has_app_config {
            parts.push(format!("{}/{} (app)", self.dir, settings::FILE));
        }
        parts.push(format!("{} (repo root)", settings::FILE));
        format!("{}. Image inputs: {}", parts.join(" + "), self.image.source)
    }
}

/// A commented group inside `only=[...]`. The comments are the point: a reader
/// must be able to tell "workspace plumbing" from "this binary's actual code".
struct OnlyGroup {
    comment: Option<&'static str>,
    paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entry point

/// Which files to write, and which app set to consider.
///
/// The nx plugin drives this: a per-project `tilt-gen` target passes
/// `app = Some(<its dir>)`, and the workspace-level target passes `root_only`
/// plus the `apps` list nx inferred — so the root Tiltfile's `include()` lines
/// are nx's answer, not a second opinion.
#[derive(Default)]
pub struct Selection<'a> {
    /// Write only this app's Tiltfile.
    pub app: Option<&'a str>,
    /// Write only the root Tiltfile.
    pub root_only: bool,
    /// Authoritative app directories; empty = discover them.
    pub apps: &'a [String],
}

/// Generate Tiltfiles. Returns workspace-root-relative path -> content.
pub fn generate(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
    selection: &Selection<'_>,
) -> Result<BTreeMap<String, String>> {
    let candidates = if selection.apps.is_empty() {
        discover(workspace_root, graph, root, overrides)
    } else {
        explicit(workspace_root, graph, overrides, selection.apps)?
    };
    if candidates.is_empty() {
        bail!(
            "found no deployable app: a monorepo app needs a recognizable kind \
             ({KIND_MARKERS}) plus k8s manifests under <app>/k8s; \
             a single-app repo needs an [app] section in the root {file}",
            file = settings::FILE
        );
    }
    if let Some(app) = selection.app {
        if !candidates.iter().any(|c| c.dir == app) {
            bail!(
                "{app} is not a deployable app (needs {KIND_MARKERS} \
                 plus k8s manifests under {app}/k8s)"
            );
        }
    }

    let resolved: Vec<ResolvedApp> = candidates
        .iter()
        // A single app's Tiltfile depends on nothing but that app, so resolving
        // one is enough when that is all we write.
        .filter(|c| selection.app.is_none_or(|want| c.dir == want))
        .map(|c| resolve(workspace_root, graph, root, c))
        .collect::<Result<_>>()?;

    let root_apps = resolved.iter().filter(|a| a.is_root_app()).count();
    if root_apps > 1 {
        bail!("more than one app claims the repo root");
    }

    let want_root = selection.app.is_none();
    let want_apps = !selection.root_only;

    let mut files = BTreeMap::new();
    if want_root {
        files.insert("Tiltfile".to_string(), render_root(root, &resolved));
    }
    if want_apps {
        for app in resolved.iter().filter(|a| !a.is_root_app()) {
            files.insert(format!("{}/Tiltfile", app.dir), render_app_file(app));
        }
    }
    Ok(files)
}

/// Candidates from an explicit directory list. Every entry must be a real,
/// deployable app — a stale list is a loud failure, not a silent omission.
fn explicit<'a>(
    workspace_root: &Path,
    graph: &'a ProjectGraph,
    overrides: &AppOverrides,
    dirs: &[String],
) -> Result<Vec<Candidate<'a>>> {
    let mut out = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let project = graph
            .projects
            .values()
            .find(|p| p.root == *dir)
            .ok_or_else(|| eyre!("{dir} is not a discovered project"))?;
        let kind = app_kind(workspace_root, dir)
            .ok_or_else(|| eyre!("{dir}: no {KIND_MARKERS}; unsupported app kind"))?;
        let explicit = overrides.get(dir);
        out.push(Candidate {
            dir: dir.clone(),
            project,
            kind,
            app: explicit.cloned().unwrap_or_default(),
            has_app_config: explicit.is_some(),
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(out)
}

/// An app the generator will emit for: its directory, its kind, and whatever
/// overrides it declared (usually none).
struct Candidate<'a> {
    dir: String,
    project: &'a Project,
    kind: Kind,
    app: App,
    /// Did the app ship a `butler.toml`? Only affects the generated header's
    /// provenance line — participation never depends on it.
    has_app_config: bool,
}

/// Every app in the repo, found by what is on disk — no registry to maintain and
/// no per-app opt-in.
///
/// A discovered project qualifies when it has a recognizable kind (a cargo
/// crate, a Vite app, an Astro app) **and** it ships k8s manifests under
/// `<dir>/k8s`, which is what makes it deployable and therefore a Tilt
/// resource. A project that declares a `butler.toml` is always included, even
/// without manifests — that file is an explicit statement of intent.
fn discover<'a>(
    workspace_root: &Path,
    graph: &'a ProjectGraph,
    root: &Root,
    overrides: &AppOverrides,
) -> Vec<Candidate<'a>> {
    let mut out: Vec<Candidate<'a>> = Vec::new();
    for project in graph.projects.values() {
        let dir = project.root.clone();
        let explicit = overrides.get(&dir);
        // The repo root only participates as an app when it says so.
        if dir == "." && explicit.is_none() {
            continue;
        }
        let Some(kind) = app_kind(workspace_root, &dir) else {
            continue;
        };
        let deployable = !matches!(resolve_k8s(workspace_root, &dir, &root.env), K8s::None);
        if !deployable && explicit.is_none() {
            continue;
        }
        out.push(Candidate {
            dir,
            project,
            kind,
            app: explicit.cloned().unwrap_or_default(),
            has_app_config: explicit.is_some(),
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// The on-disk markers that make a directory a recognizable app kind, spelled
/// once so every "unsupported kind" message stays in step with [`app_kind`].
pub(crate) const KIND_MARKERS: &str = "Cargo.toml, vite.config.ts, or astro.config.mjs";

/// Kind from what the app is built with; `None` = not an app (a manifest-only
/// directory such as a shared-config kustomize tree).
///
/// The markers are checked in this order, so a directory carrying two of them
/// gets the earlier kind: a cargo crate wins over everything, and an app that
/// keeps both a `vite.config.ts` and an `astro.config.mjs` is treated as the
/// Vite build — an Astro app has no reason to keep a Vite config of its own.
pub(crate) fn app_kind(workspace_root: &Path, dir: &str) -> Option<Kind> {
    let abs = workspace_root.join(dir);
    if abs.join("Cargo.toml").is_file() {
        Some(Kind::Service)
    } else if abs.join("vite.config.ts").is_file() {
        Some(Kind::Web)
    } else if abs.join("astro.config.mjs").is_file() {
        Some(Kind::Node)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Resolution

fn resolve(
    workspace_root: &Path,
    graph: &ProjectGraph,
    root: &Root,
    candidate: &Candidate<'_>,
) -> Result<ResolvedApp> {
    let dir = candidate.dir.clone();
    let abs = workspace_root.join(&dir);
    let project = candidate.project;
    let kind = candidate.kind;

    let up = up_prefix(&dir);
    let image = resolve_image(root, project, candidate, &up)?;
    let workload = k8s::merged_workload(root, &candidate.app, kind, &root.env);
    let k8s = k8s_backend(workspace_root, &dir, root, workload.is_some());

    let extra_only = root
        .tilt
        .dockerfile_only
        .get(&image.dockerfile)
        .cloned()
        .unwrap_or_default();

    let (only, ignore) = match kind {
        Kind::Service => (
            service_only(
                workspace_root,
                graph,
                project,
                &image.dockerfile,
                &extra_only,
            )?,
            root.tilt.context_ignore.clone(),
        ),
        Kind::Web => (web_only(&dir, &extra_only), Vec::new()),
        // Built from the workspace like a service: the app's own toolchain runs
        // inside the image, so there is no local output to point at.
        Kind::Node => (
            node_only(
                workspace_root,
                graph,
                project,
                &dir,
                &image.dockerfile,
                &extra_only,
            )?,
            root.tilt.context_ignore.clone(),
        ),
    };

    let tilt: &AppTilt = &candidate.app.tilt;
    let labels = if tilt.labels.is_empty() {
        match kind {
            Kind::Service => vec!["backend".to_string()],
            // A web app's Tilt label rides on its build resource, not the k8s one.
            Kind::Web => Vec::new(),
            // A node app has no build resource, so its label rides on the k8s one.
            Kind::Node => vec!["frontend".to_string()],
        }
    } else {
        tilt.labels.clone()
    };

    let resource = image
        .reference
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            eyre!(
                "{dir}: cannot derive a resource name from image `{}`",
                image.reference
            )
        })?
        .to_string();

    let mut resource_deps = tilt.resource_deps.clone();
    let mut web_deps = Vec::new();
    if kind == Kind::Web {
        // The image cannot build before the local build resource produced dist.
        resource_deps.insert(0, format!("build-{resource}"));
        // Watch what the SPA build actually reads, and only if it is there.
        for watched in ["src", "project.json", "vite.config.ts", "index.html"] {
            if abs.join(watched).exists() {
                web_deps.push(watched.to_string());
            }
        }
    }

    // The container side is the workload's own `port`, so the port is stated
    // once. An app with no workload yet takes the package's default, which is
    // what its hand-written manifests already use.
    let port_forwards = tilt.host_port.map(|host| {
        let container = workload
            .as_ref()
            .map_or(k8s::DEFAULT_PORT, k8s::workload_port);
        format!("{host}:{container}")
    });

    Ok(ResolvedApp {
        dir,
        project: project.name.clone(),
        resource,
        kind,
        k8s,
        up,
        image,
        only,
        ignore,
        port_forwards,
        labels,
        resource_deps,
        web_deps,
        has_app_config: candidate.has_app_config,
    })
}

/// Image build inputs in workspace-root-relative form — the one resolution both
/// the Tiltfile and the nx `container` target are derived from.
pub(crate) struct ImageFacts {
    /// Which of the two sources owned these inputs.
    pub source: &'static str,
    /// Dockerfile path, workspace-root-relative.
    pub dockerfile: String,
    /// Build context, workspace-root-relative (`.` = the repo root).
    pub context: String,
    /// Build stage, if the source pinned one. `None` = unpinned here, so the
    /// root `[dockerfileTarget]` map decides — see [`resolve_stage`], which is
    /// what both readers go through. Not "whatever the Dockerfile ends on":
    /// CI builds the resolved stage too, because the inferred `container`
    /// target carries it as its `target` option.
    pub stage: Option<String>,
    pub build_args: BTreeMap<String, String>,
    /// Image reference with `$REGISTRY` left unsubstituted, e.g.
    /// `$REGISTRY/zerg-api:latest`.
    pub tag: String,
}

/// Image inputs, in strict precedence with no silent merging:
///
/// 1. the app's `[image]` — an app that departs from the repo convention;
/// 2. the repo convention in the root config's `[imageDefaults.<kind>]`,
///    with the per-app values derived (crate name for a service, `<app>/dist`
///    for a web app, the app directory for a node app,
///    `$REGISTRY/<product>-<app>:latest` for the tag).
///
/// Nothing reads the nx `container` target: that target is *generated* from
/// these facts by `tools/nx/container-targets.ts`, and `butler container verify`
/// diffs the generated graph against this function. Reading it back would make
/// the derivation circular and let a hand-edited target silently move the image.
pub(crate) fn image_facts(
    root: &Root,
    kind: &Kind,
    dir: &str,
    project: &Project,
    declared: Option<&settings::Image>,
) -> Result<ImageFacts> {
    if let Some(image) = declared {
        return Ok(ImageFacts {
            source: "this app's [image]",
            dockerfile: image.dockerfile.clone(),
            context: image.context.clone(),
            stage: image.target.clone(),
            build_args: image.build_args.clone(),
            tag: image.tag.clone(),
        });
    }

    let kind_name = kind.name();
    let convention = match kind {
        Kind::Service => root.image_defaults.service.as_ref(),
        Kind::Web => root.image_defaults.web.as_ref(),
        Kind::Node => root.image_defaults.node.as_ref(),
    }
    .ok_or_else(|| {
        eyre!(
            "{dir}: no image inputs and no repo convention for a {kind_name} — \
             add [imageDefaults.{kind_name}] to the root {file} or an \
             `[image]` section to {dir}/{file}",
            file = settings::FILE
        )
    })?;

    let mut build_args = BTreeMap::new();
    if let Some(name) = &convention.build_arg {
        let value = match kind {
            // The cargo package name the Dockerfile compiles.
            Kind::Service => project.name.clone(),
            // Where the local build dropped the static output.
            Kind::Web => {
                if dir == "." {
                    "dist".to_string()
                } else {
                    format!("{dir}/dist")
                }
            }
            // Which app the image installs and builds internally: nothing is
            // built outside it, so it needs the directory, not an output path.
            Kind::Node => dir.to_string(),
        };
        build_args.insert(name.clone(), value);
    }

    Ok(ImageFacts {
        source: "repo convention [imageDefaults]",
        dockerfile: convention.dockerfile.clone(),
        context: ".".to_string(),
        stage: convention.target.clone(),
        build_args,
        tag: format!(
            "$REGISTRY/{}:latest",
            derived_image_name(dir, &project.name)
        ),
    })
}

/// The build stage these facts resolve to, in strict precedence: what the
/// source pinned, else the root `[dockerfileTarget]` entry for the Dockerfile,
/// else a loud error — a Dockerfile whose last stage is not the deployed one
/// must never be built by accident.
///
/// One implementation, two callers: the Tiltfile's `docker_build(target=...)`
/// and the `target` option `butler container verify` expects on the inferred
/// nx `container` target.
pub(crate) fn resolve_stage(root: &Root, facts: &ImageFacts, dir: &str) -> Result<String> {
    if let Some(stage) = &facts.stage {
        return Ok(stage.clone());
    }
    root.dockerfile_target
        .get(&facts.dockerfile)
        .cloned()
        .ok_or_else(|| {
            eyre!(
                "{dir}: no build stage for {} — pin one on the image or \
                 add a [dockerfileTarget] entry to the root {file}",
                facts.dockerfile,
                file = settings::FILE
            )
        })
}

/// The same facts, spelled the way the app's own Tiltfile needs them: the
/// registry substituted into the reference, the context app-relative, and a
/// build stage pinned (Tilt must never build a Dockerfile's final stage when
/// that stage is a different server).
fn resolve_image(
    root: &Root,
    project: &Project,
    candidate: &Candidate<'_>,
    up: &str,
) -> Result<Image> {
    let dir = &candidate.dir;
    let facts = image_facts(
        root,
        &candidate.kind,
        dir,
        project,
        candidate.app.image.as_ref(),
    )?;

    let stage = resolve_stage(root, &facts, dir)?;

    let context = if up.is_empty() {
        facts.context
    } else if facts.context == "." {
        up.to_string()
    } else {
        format!("{up}/{}", facts.context)
    };

    Ok(Image {
        reference: image_from_tag(&facts.tag, &root.registry),
        source: facts.source,
        // Workspace-relative: `only=` needs this form, and the app-relative form
        // is derived at render time via ResolvedApp::local.
        dockerfile: facts.dockerfile,
        context,
        stage,
        build_args: facts.build_args,
    })
}

/// Image repository name for an app that declares none: the app's path below
/// `apps/` joined by `-` (`apps/todo/api` -> `todo-api`), which is the naming
/// this repo's Deployments already use. A root app falls back to its project
/// name with cargo's underscores normalised.
pub(crate) fn derived_image_name(dir: &str, project: &str) -> String {
    let trimmed = dir.strip_prefix("apps/").unwrap_or(dir);
    if trimmed == "." || trimmed.is_empty() {
        return project.replace('_', "-");
    }
    trimmed.replace('/', "-")
}

/// `$REGISTRY/zerg-api:latest` -> `yurikrupnik/zerg-api`.
pub(crate) fn image_from_tag(tag: &str, registry: &str) -> String {
    let no_tag = match tag.rsplit_once(':') {
        // A port in a registry host (`localhost:5000/x`) is not a tag.
        Some((base, t)) if !t.contains('/') => base,
        _ => tag,
    };
    no_tag.replace("$REGISTRY", registry)
}

/// `apps/zerg/api` -> `../../..`; the repo root -> `` (already there).
fn up_prefix(dir: &str) -> String {
    let depth = dir
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .count();
    vec![".."; depth].join("/")
}

/// How this app's objects reach the cluster, in strict precedence:
///
/// 1. a declared `[workload]` — the published `app` KCL package renders it from
///    the generated values files, which is the same command `butler k8s gen`
///    commits the output of, so the dev loop and the committed manifests cannot
///    drift;
/// 2. whatever manifests the app still ships on disk (see [`resolve_k8s`]),
///    which is what an app that has not moved to a `[workload]` yet keeps.
fn k8s_backend(workspace_root: &Path, dir: &str, root: &Root, has_workload: bool) -> K8s {
    if has_workload {
        return K8s::Package(k8s::render_command(root));
    }
    resolve_k8s(workspace_root, dir, &root.env)
}

pub(crate) fn resolve_k8s(workspace_root: &Path, dir: &str, env: &str) -> K8s {
    let k8s_dir = workspace_root.join(dir).join("k8s");
    // KCL wins when both exist: a live `kcl run` reflects edits without a
    // kustomize build step, and the kustomize tree is then GitOps-only.
    if k8s_dir.join("kcl.mod").is_file() {
        // Namespace = the product segment of `apps/<product>/<app>`, else the
        // app directory name, else the repo's single namespace `default`.
        let namespace = k8s::product(dir);
        return K8s::KclLive {
            namespace,
            env: env.to_string(),
        };
    }
    if k8s_dir
        .join("kustomize")
        .join("overlays")
        .join(env)
        .is_dir()
    {
        return K8s::Kustomize(format!("k8s/kustomize/overlays/{env}"));
    }
    K8s::None
}

// ---------------------------------------------------------------------------
// Build context whitelists

/// Tight `only=` for a service image.
///
/// Three groups, and the middle one is the subtle part: `cargo` refuses to load a
/// workspace whose members it cannot resolve a target for, even for members it
/// will never compile. So each non-dependency member contributes its `Cargo.toml`
/// plus the one entry file (`src/lib.rs`, `src/main.rs`, or a declared
/// `[lib]`/`[[bin]]` path) that makes the manifest legal — never the rest of its
/// `src/`. Everything this binary actually compiles contributes its whole
/// directory.
fn service_only(
    workspace_root: &Path,
    graph: &ProjectGraph,
    project: &Project,
    dockerfile: &str,
    extra: &[String],
) -> Result<Vec<OnlyGroup>> {
    let plumbing: Vec<String> = ["Cargo.toml", "Cargo.lock", dockerfile]
        .into_iter()
        .chain(extra.iter().map(String::as_str))
        // A whitelist entry for a file that does not exist is noise at best;
        // `Cargo.lock` is absent in a fresh single-crate repo.
        .filter(|p| workspace_root.join(p).exists())
        .map(str::to_string)
        .collect();

    let sources = cargo_source_closure(workspace_root, graph, &[project])?;
    // A single-crate repo has nothing to whitelist beyond itself.
    if sources.len() == 1 && sources.contains(".") {
        return Ok(vec![OnlyGroup {
            comment: None,
            paths: plumbing.into_iter().chain(["src".to_string()]).collect(),
        }]);
    }

    let manifests = cargo_member_manifests(workspace_root, graph, &sources)?;

    Ok(vec![
        OnlyGroup {
            comment: None,
            paths: plumbing,
        },
        OnlyGroup {
            comment: Some(
                "workspace members this binary does not compile: manifest plus \
                 the entry file cargo needs to resolve a target, so the workspace \
                 loads without their sources busting this image's cache",
            ),
            paths: manifests,
        },
        OnlyGroup {
            comment: Some(
                "this crate plus its transitive workspace deps, from the butler \
                 project graph — the only sources that can change this binary",
            ),
            paths: sources.into_iter().collect(),
        },
    ])
}

/// Cargo directories whose whole contents can change what `seeds` compile to:
/// each seed plus its transitive non-dev workspace deps, from the butler project
/// graph. Non-cargo deps are dropped — a TS package cannot change a binary.
fn cargo_source_closure(
    workspace_root: &Path,
    graph: &ProjectGraph,
    seeds: &[&Project],
) -> Result<BTreeSet<String>> {
    let mut sources: BTreeSet<String> = BTreeSet::new();
    for seed in seeds {
        sources.insert(seed.root.clone());
        for dep in graph.transitive_build_deps(&seed.name)? {
            if let Some(p) = graph.projects.get(&dep) {
                if is_cargo_crate(workspace_root, &p.root) {
                    sources.insert(p.root.clone());
                }
            }
        }
    }
    Ok(sources)
}

/// What every *other* cargo member must contribute for the workspace to load in
/// the build context: its manifest plus the one entry file that makes the
/// manifest legal. `whole` names the directories already whitelisted entirely.
fn cargo_member_manifests(
    workspace_root: &Path,
    graph: &ProjectGraph,
    whole: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut manifests: Vec<String> = Vec::new();
    for p in graph.projects.values() {
        if !is_cargo_crate(workspace_root, &p.root) || whole.contains(&p.root) {
            continue;
        }
        manifests.extend(member_loadable_paths(workspace_root, &p.root)?);
    }
    manifests.sort();
    Ok(manifests)
}

/// Tight `only=` for a node image, which installs the bun workspace and runs the
/// app's own build inside the builder stage. That means the context has to
/// satisfy two workspace loaders, and the two "manifest only" groups below are
/// what keeps it from degenerating into the whole repo:
///
///   * bun's: `bun install --frozen-lockfile` resolves every member named by
///     `bun.lock`, so a member missing its `package.json` fails the install even
///     though this app never imports it;
///   * cargo's: only when a workspace npm dependency is itself built by cargo —
///     the N-API addon, whose `.node` binary must be compiled for the image's
///     platform and so cannot be copied from a developer's machine. Its Rust
///     sources are build inputs of this image; a cargo crate whose npm side is
///     just generated TS (`@domain/todo`) contributes only its directory.
fn node_only(
    workspace_root: &Path,
    graph: &ProjectGraph,
    project: &Project,
    dir: &str,
    dockerfile: &str,
    extra: &[String],
) -> Result<Vec<OnlyGroup>> {
    // This app plus its transitive workspace npm deps, walked over the graph
    // edges `discovery::discover_node` builds from `package.json` dependencies.
    let mut npm_sources: BTreeSet<String> = BTreeSet::new();
    let mut cargo_seeds: Vec<&Project> = Vec::new();
    npm_sources.insert(dir.to_string());
    let mut queue: Vec<&str> = vec![project.name.as_str()];
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    while let Some(name) = queue.pop() {
        let Some(p) = graph.projects.get(name) else {
            continue;
        };
        for dep in &p.deps {
            let Some(d) = graph.projects.get(dep) else {
                continue;
            };
            // Only npm packages: a cargo-only dep of a dep is not an import of
            // this app, and the cargo closure below covers the ones that are.
            if !is_npm_package(workspace_root, &d.root) || !seen.insert(dep.as_str()) {
                continue;
            }
            npm_sources.insert(d.root.clone());
            if is_cargo_built_npm_package(workspace_root, &d.root) {
                cargo_seeds.push(d);
            }
            queue.push(dep.as_str());
        }
    }

    let mut plumbing: Vec<&str> = vec!["package.json", "bun.lock"];
    if !cargo_seeds.is_empty() {
        plumbing.extend(["Cargo.toml", "Cargo.lock"]);
    }
    let plumbing: Vec<String> = plumbing
        .into_iter()
        .chain([dockerfile])
        .chain(extra.iter().map(String::as_str))
        // A whitelist entry for a file that does not exist is noise at best.
        .filter(|p| workspace_root.join(p).exists())
        .map(str::to_string)
        .collect();

    let mut npm_manifests: Vec<String> = graph
        .projects
        .values()
        .filter(|p| {
            p.root != "."
                && is_npm_package(workspace_root, &p.root)
                && !npm_sources.contains(&p.root)
        })
        .map(|p| format!("{}/package.json", p.root))
        .collect();
    npm_manifests.sort();

    // Computed before the groups are laid out: the cargo closure decides which
    // members still need a manifest-only entry.
    let cargo_sources = if cargo_seeds.is_empty() {
        BTreeSet::new()
    } else {
        cargo_source_closure(workspace_root, graph, &cargo_seeds)?
    };
    let cargo_manifests = if cargo_sources.is_empty() {
        Vec::new()
    } else {
        let whole: BTreeSet<String> = cargo_sources.union(&npm_sources).cloned().collect();
        cargo_member_manifests(workspace_root, graph, &whole)?
    };

    Ok(vec![
        OnlyGroup {
            comment: None,
            paths: plumbing,
        },
        OnlyGroup {
            comment: Some(
                "every other bun workspace member's manifest: a frozen install \
                 resolves the whole workspace, and a member missing from the \
                 context fails it — never their sources",
            ),
            paths: npm_manifests,
        },
        OnlyGroup {
            comment: Some(
                "cargo members the addon does not compile: manifest plus the \
                 entry file cargo needs to resolve a target, so the workspace \
                 loads without their sources busting this image's cache",
            ),
            paths: cargo_manifests,
        },
        OnlyGroup {
            comment: Some(
                "this app plus its transitive workspace npm deps, from the butler \
                 project graph — the only sources that can change this image",
            ),
            paths: npm_sources.iter().cloned().collect(),
        },
        OnlyGroup {
            comment: Some(
                "the cargo build closure of the native addon this app requires \
                 at run time: its binary is compiled in the image, so these are \
                 build inputs too",
            ),
            paths: cargo_sources
                .into_iter()
                .filter(|p| !npm_sources.contains(p))
                .collect(),
        },
    ])
}

/// Every file a workspace member must contribute for `cargo metadata` to load it
/// without compiling it: the manifest, plus whichever declared or conventional
/// target entry file exists. A member with no resolvable target aborts the whole
/// workspace load with "no targets specified in the manifest".
fn member_loadable_paths(workspace_root: &Path, root: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Manifest {
        lib: Option<Lib>,
        #[serde(default)]
        bin: Vec<Bin>,
    }
    #[derive(Deserialize)]
    struct Lib {
        path: Option<String>,
    }
    #[derive(Deserialize)]
    struct Bin {
        name: Option<String>,
        path: Option<String>,
    }

    let dir = workspace_root.join(root);
    let manifest_path = dir.join("Cargo.toml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| eyre!("reading {}: {e}", manifest_path.display()))?;
    let manifest: Manifest =
        toml::from_str(&raw).map_err(|e| eyre!("parsing {}: {e}", manifest_path.display()))?;

    let mut candidates: Vec<String> = Vec::new();
    if let Some(lib) = &manifest.lib {
        candidates.push(lib.path.clone().unwrap_or_else(|| "src/lib.rs".into()));
    }
    for bin in &manifest.bin {
        match &bin.path {
            Some(p) => candidates.push(p.clone()),
            // No path: cargo looks at src/main.rs and src/bin/<name>.rs.
            None => {
                candidates.push("src/main.rs".into());
                if let Some(name) = &bin.name {
                    candidates.push(format!("src/bin/{name}.rs"));
                }
            }
        }
    }
    if manifest.lib.is_none() && manifest.bin.is_empty() {
        // Pure autodiscovery.
        candidates.push("src/lib.rs".into());
        candidates.push("src/main.rs".into());
    }

    let mut out = vec![format!("{root}/Cargo.toml")];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for candidate in candidates {
        if dir.join(&candidate).is_file() && seen.insert(candidate.clone()) {
            out.push(format!("{root}/{candidate}"));
        }
    }
    if out.len() == 1 {
        bail!(
            "{root}: no cargo target entry file found; \
             a build context without one cannot load the workspace"
        );
    }
    Ok(out)
}

/// A web image needs its own build output and nothing else from the workspace.
fn web_only(dir: &str, extra: &[String]) -> Vec<OnlyGroup> {
    let dist = if dir == "." {
        "dist".to_string()
    } else {
        format!("{dir}/dist")
    };
    let mut paths = vec![dist];
    paths.extend(extra.iter().cloned());
    vec![OnlyGroup {
        comment: None,
        paths,
    }]
}

fn is_cargo_crate(workspace_root: &Path, root: &str) -> bool {
    workspace_root.join(root).join("Cargo.toml").is_file()
}

fn is_npm_package(workspace_root: &Path, root: &str) -> bool {
    workspace_root.join(root).join("package.json").is_file()
}

/// Is this npm package's own artifact produced by cargo? True for the N-API
/// addon: a cargo crate whose `package.json` declares a `build` script, so
/// building the npm package shells out to cargo and its Rust sources are build
/// inputs of whatever installs it. False for a cargo crate whose npm side is
/// checked-in or generated files (`@domain/todo` publishes ts-rs output).
fn is_cargo_built_npm_package(workspace_root: &Path, root: &str) -> bool {
    if !is_cargo_crate(workspace_root, root) {
        return false;
    }
    #[derive(Deserialize)]
    struct Scripts {
        #[serde(default)]
        scripts: BTreeMap<String, String>,
    }
    let path = workspace_root.join(root).join("package.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Scripts>(&raw).ok())
        .is_some_and(|pkg| pkg.scripts.contains_key("build"))
}

// ---------------------------------------------------------------------------
// Rendering

fn quote_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|i| format!("'{i}'")).collect();
    format!("[{}]", inner.join(", "))
}

fn render_root(root: &Root, apps: &[ResolvedApp]) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "# GENERATED by `butler tilt gen` — DO NOT EDIT BY HAND.\n\
         # Repo-wide config: {file}. Per-app config: <app>/{file}.\n",
        file = settings::FILE,
    );

    for pf in &root.tilt.port_forward {
        let _ = write!(
            out,
            "\nlocal_resource(\n    \
                 '{name}',\n    \
                 serve_cmd='{cmd}',\n    \
                 labels=['port-forward'],\n    \
                 readiness_probe=probe(\n        \
                     period_secs={period},\n        \
                     exec=exec_action(['sh', '-c', 'nc -z localhost {port}']),\n    \
                 ),\n)\n",
            name = pf.name,
            cmd = pf.command,
            period = pf.probe_period_secs,
            port = pf.probe_port,
        );
    }

    for sr in &root.tilt.shared_resource {
        let _ = write!(
            out,
            "\nk8s_yaml(kustomize('{kustomize}'))\nk8s_resource(\n    \
                 objects={objects},\n    \
                 new_name='{name}',\n    \
                 labels=['config'],\n)\n",
            kustomize = sr.kustomize,
            objects = quote_list(&sr.objects),
            name = sr.name,
        );
    }

    // A single-app repo has no per-app file to include: inline it here.
    for app in apps.iter().filter(|a| a.is_root_app()) {
        out.push_str(&render_app_body(app));
    }

    let included: Vec<&ResolvedApp> = apps.iter().filter(|a| !a.is_root_app()).collect();
    if !included.is_empty() {
        out.push('\n');
        for app in included {
            let _ = writeln!(out, "include('./{}/Tiltfile')", app.dir);
        }
    }
    out
}

fn render_app_file(app: &ResolvedApp) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "# GENERATED by `butler tilt gen` — DO NOT EDIT BY HAND.\n\
         # Config: {sources}\n",
        sources = app.config_sources(),
    );
    out.push_str(&render_app_body(app));
    out
}

fn render_app_body(app: &ResolvedApp) -> String {
    let mut out = String::new();
    let _ = write!(out, "\n# {}\n", app.resource);

    if app.kind == Kind::Web {
        let dir_arg = if app.up.is_empty() {
            ".".to_string()
        } else {
            app.up.clone()
        };
        let _ = write!(
            out,
            "\nlocal_resource(\n    \
                 'build-{resource}',\n    \
                 cmd='bun nx run {project}:build',\n    \
                 dir='{dir_arg}',\n    \
                 deps={deps},\n    \
                 ignore=['k8s'],\n    \
                 labels=['bun'],\n)\n",
            resource = app.resource,
            project = app.project,
            deps = quote_list(&app.web_deps),
        );
    }

    out.push_str("\ndocker_build(\n");
    let _ = writeln!(out, "    '{}',", app.image.reference);
    if !app.image.build_args.is_empty() {
        let args: Vec<String> = app
            .image
            .build_args
            .iter()
            .map(|(k, v)| format!("'{k}': '{v}'"))
            .collect();
        let _ = writeln!(out, "    build_args={{{}}},", args.join(", "));
    }
    let _ = writeln!(out, "    context='{}',", app.image.context);
    let _ = writeln!(
        out,
        "    dockerfile='{}',",
        app.local(&app.image.dockerfile)
    );
    render_only(&mut out, &app.only);
    if !app.ignore.is_empty() {
        out.push_str("    ignore=[\n");
        for path in &app.ignore {
            let _ = writeln!(out, "        '{path}',");
        }
        out.push_str("    ],\n");
    }
    let _ = writeln!(out, "    target='{}',", app.image.stage);
    out.push_str(")\n\n");

    match &app.k8s {
        K8s::Package(command) => {
            // `dir='k8s'` because the package resolves `values.yaml` and its
            // `values.<env>.yaml` overlay relative to the working directory.
            let _ = writeln!(out, "k8s_yaml(local('{command}', dir='k8s'))");
        }
        K8s::Kustomize(path) => {
            let _ = writeln!(out, "k8s_yaml(kustomize('{path}'))");
        }
        K8s::KclLive { namespace, env } => {
            let _ = writeln!(
                out,
                "k8s_yaml(local('kcl run k8s -D env={env} -D image={image} -D namespace={namespace}', quiet=True))",
                image = app.image.reference,
            );
        }
        K8s::None => {}
    }

    let mut args = String::new();
    if let Some(pf) = &app.port_forwards {
        let _ = write!(args, ", port_forwards='{pf}'");
    }
    if !app.labels.is_empty() {
        let _ = write!(args, ", labels={}", quote_list(&app.labels));
    }
    if !app.resource_deps.is_empty() {
        let _ = write!(args, ", resource_deps={}", quote_list(&app.resource_deps));
    }
    let _ = writeln!(out, "k8s_resource('{}'{args})", app.resource);
    out
}

fn render_only(out: &mut String, groups: &[OnlyGroup]) {
    if groups.iter().all(|g| g.paths.is_empty()) {
        return;
    }
    out.push_str("    only=[\n");
    for group in groups {
        if group.paths.is_empty() {
            continue;
        }
        if let Some(comment) = group.comment {
            for line in wrap_comment(comment, 66) {
                let _ = writeln!(out, "        # {line}");
            }
        }
        for path in &group.paths {
            let _ = writeln!(out, "        '{path}',");
        }
    }
    out.push_str("    ],\n");
}

/// Greedy word wrap so generated comments stay inside a sane line length.
fn wrap_comment(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ---------------------------------------------------------------------------
// Commands

pub fn write_files(out_dir: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    for (rel, content) in files {
        let path = out_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content).map_err(|e| eyre!("writing {}: {e}", path.display()))?;
        println!("wrote {rel}");
    }
    Ok(())
}

/// Drift gate: every generated file must already be on disk, byte-identical.
/// Returns the drifted paths.
pub fn check_files(out_dir: &Path, files: &BTreeMap<String, String>) -> Vec<String> {
    let mut drifted = Vec::new();
    for (rel, content) in files {
        let path = out_dir.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == *content => {}
            Ok(_) => drifted.push(format!("{rel}: differs from generated output")),
            Err(e) => drifted.push(format!("{rel}: {e}")),
        }
    }
    drifted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Throwaway directory tree, so the on-disk kind detection can be exercised
    /// without depending on this repo's own layout.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "butler-tilt-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch root");
            Self(dir)
        }

        /// Create an empty file at `rel`, parents included.
        fn touch(&self, rel: &str) -> &Self {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().expect("file has a parent"))
                .expect("scratch parents");
            std::fs::write(&path, "").expect("scratch file");
            self
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn project(name: &str, root: &str) -> Project {
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            targets: BTreeMap::new(),
            deps: BTreeSet::new(),
            build_deps: BTreeSet::new(),
        }
    }

    #[test]
    fn an_astro_config_alone_makes_a_node_app() {
        let scratch = Scratch::new("kind");
        scratch.touch("apps/todo/web-astro/astro.config.mjs");
        assert_eq!(
            app_kind(&scratch.0, "apps/todo/web-astro"),
            Some(Kind::Node)
        );
        // A directory with none of the markers is not an app at all.
        scratch.touch("apps/shared/k8s/kustomization.yaml");
        assert_eq!(app_kind(&scratch.0, "apps/shared"), None);
    }

    #[test]
    fn a_cargo_crate_outranks_the_js_markers() {
        let scratch = Scratch::new("precedence");
        scratch
            .touch("apps/hybrid/Cargo.toml")
            .touch("apps/hybrid/vite.config.ts")
            .touch("apps/hybrid/astro.config.mjs");
        assert_eq!(app_kind(&scratch.0, "apps/hybrid"), Some(Kind::Service));
        // Without the crate, the Vite config still wins over the Astro one.
        let scratch = Scratch::new("precedence-js");
        scratch
            .touch("apps/hybrid/vite.config.ts")
            .touch("apps/hybrid/astro.config.mjs");
        assert_eq!(app_kind(&scratch.0, "apps/hybrid"), Some(Kind::Web));
    }

    #[test]
    fn a_node_app_derives_its_app_dir_as_the_build_arg() {
        let root: Root = toml::from_str(
            "registry = 'acme'\n\
             [imageDefaults.node]\n\
             dockerfile = 'manifests/dockers/node.Dockerfile'\n\
             target = 'runtime'\n\
             buildArg = 'APP_DIR'\n",
        )
        .expect("node convention parses");
        let project = project("todo-astro-web", "apps/todo/web-astro");
        let facts = image_facts(&root, &Kind::Node, &project.root, &project, None)
            .expect("the convention covers a node app");

        // The app directory, not a prebuilt output path: the image builds it.
        assert_eq!(
            facts.build_args,
            BTreeMap::from([("APP_DIR".to_string(), "apps/todo/web-astro".to_string())])
        );
        assert_eq!(facts.dockerfile, "manifests/dockers/node.Dockerfile");
        assert_eq!(facts.context, ".");
        assert_eq!(
            resolve_stage(&root, &facts, &project.root).expect("stage"),
            "runtime"
        );
        assert_eq!(facts.tag, "$REGISTRY/todo-web-astro:latest");
    }

    #[test]
    fn a_node_app_without_a_convention_names_the_section_to_add() {
        let root: Root = toml::from_str("registry = 'acme'").expect("minimal root");
        let project = project("todo-astro-web", "apps/todo/web-astro");
        let Err(err) = image_facts(&root, &Kind::Node, &project.root, &project, None) else {
            panic!("a node app with no convention must not resolve an image")
        };
        assert!(err.to_string().contains("[imageDefaults.node]"), "{err}");
    }

    #[test]
    fn tag_becomes_image_with_registry_substituted() {
        assert_eq!(
            image_from_tag("$REGISTRY/zerg-api:latest", "yurikrupnik"),
            "yurikrupnik/zerg-api"
        );
        assert_eq!(
            image_from_tag("$REGISTRY/zerg-api", "acme"),
            "acme/zerg-api"
        );
    }

    #[test]
    fn derived_image_name_matches_the_repo_naming() {
        // What the existing Deployments already pull.
        assert_eq!(derived_image_name("apps/todo/api", "todo_api"), "todo-api");
        assert_eq!(
            derived_image_name("apps/zerg/email-nats", "zerg_email_nats"),
            "zerg-email-nats"
        );
        // Outside apps/ the whole path is used, so two apps cannot collide.
        assert_eq!(derived_image_name("services/edge", "edge"), "services-edge");
        // A standalone repo has no path to derive from.
        assert_eq!(derived_image_name(".", "solo_svc"), "solo-svc");
    }

    #[test]
    fn registry_port_is_not_mistaken_for_a_tag() {
        assert_eq!(
            image_from_tag("localhost:5000/zerg-api", "unused"),
            "localhost:5000/zerg-api"
        );
    }

    #[test]
    fn up_prefix_matches_directory_depth() {
        assert_eq!(up_prefix("apps/zerg/api"), "../../..");
        assert_eq!(up_prefix("libs/database"), "../..");
        // A standalone repo's app is already at the root.
        assert_eq!(up_prefix("."), "");
    }

    #[test]
    fn comment_wraps_on_word_boundaries() {
        let lines = wrap_comment("one two three four", 9);
        assert_eq!(lines, vec!["one two", "three", "four"]);
    }

    #[test]
    fn a_declared_workload_outranks_whatever_is_on_disk() {
        let scratch = Scratch::new("backend");
        scratch
            .touch("apps/todo/api/k8s/kcl.mod")
            .touch("apps/todo/api/k8s/kustomize/overlays/dev/kustomization.yaml");
        let root: Root =
            toml::from_str("registry = 'acme'\n[k8s]\ntag = '0.1.2'\n").expect("root config");

        // The workload wins over both on-disk backends, and Tilt runs exactly
        // the command `butler k8s gen` renders the committed manifest with.
        let K8s::Package(command) = k8s_backend(&scratch.0, "apps/todo/api", &root, true) else {
            panic!("a declared [workload] must render through the package")
        };
        assert_eq!(command, crate::k8s::render_command(&root));

        // Without one, an app keeps the manifests it still ships: KCL module
        // first, then the kustomize overlay, then nothing at all.
        let K8s::KclLive { namespace, env } =
            k8s_backend(&scratch.0, "apps/todo/api", &root, false)
        else {
            panic!("a kcl.mod on disk still renders live")
        };
        assert_eq!((namespace.as_str(), env.as_str()), ("todo", "dev"));

        std::fs::remove_file(scratch.0.join("apps/todo/api/k8s/kcl.mod")).expect("drop kcl.mod");
        let K8s::Kustomize(path) = k8s_backend(&scratch.0, "apps/todo/api", &root, false) else {
            panic!("a kustomize overlay is still a backend")
        };
        assert_eq!(path, "k8s/kustomize/overlays/dev");

        assert!(matches!(
            k8s_backend(&scratch.0, "apps/nothing/here", &root, false),
            K8s::None
        ));
    }
}
