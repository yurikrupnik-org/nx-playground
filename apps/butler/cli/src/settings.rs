//! `butler.toml` — the CLI's own config, in exactly two levels.
//!
//! ```text
//! butler.toml                  root: everything repo-wide (registry, env,
//!                              infra port-forwards, shared cluster resources,
//!                              per-Dockerfile defaults)
//! <app-root>/butler.toml       app:  everything app-local, and nothing else
//! ```
//!
//! One fact lives in exactly one file. The root file never names an app; an app
//! file never repeats a repo-wide default. Image build inputs come from exactly
//! two levels too: the app's `[image]` when it departs from the convention, else
//! the root `[imageDefaults.<kind>]`.
//!
//! A standalone (non-monorepo) repo uses the same two levels collapsed into one
//! file: the root `butler.toml` grows an `[app]` section describing the single
//! app that lives at the repo root. That is why this CLI needs no per-language
//! "presets" — a one-app repo is a monorepo with one app at `.`.
//!
//! Nothing here is nx-specific: these files are the source, and the nx
//! `container`/`scan` targets are generated from them by
//! `tools/nx/container-targets.ts`, so the same CLI works in a repo that has
//! never seen nx.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};
use serde::Deserialize;

/// Config file name, identical at both levels.
pub const FILE: &str = "butler.toml";

// ---------------------------------------------------------------------------
// Root level

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Root {
    /// Container registry / image-name prefix. Substituted for `$REGISTRY`
    /// wherever an image reference names it.
    pub registry: String,
    /// Deployment environment: kustomize overlay directory and `-D env=` value.
    #[serde(default = "default_env")]
    pub env: String,
    /// How to build an image for an app that declares none — the repo's
    /// convention, stated once. This is what lets a new app need no config at
    /// all: drop it in with k8s manifests and it is picked up.
    ///
    /// Repo-wide, not Tilt-local: two readers derive from it — butler's
    /// Tiltfile renderer and the nx container plugin
    /// (`tools/nx/container-targets.ts`). The resolved stage reaches CI as the
    /// inferred `container` target's `target` option, so a Dockerfile whose
    /// last stage is not the deployed one still builds correctly there.
    #[serde(default)]
    pub image_defaults: ImageDefaults,
    /// Dockerfile path -> build stage, for images that pin no stage themselves.
    ///
    /// Same two readers as `imageDefaults`, for the same reason: the stage is a
    /// repo-wide fact about the Dockerfile, and both the Tiltfile's
    /// `docker_build` and the inferred `container` target's `target` option are
    /// built from it.
    #[serde(default)]
    pub dockerfile_target: BTreeMap<String, String>,
    /// The workload shape every app of a kind (`service`, `web`, `node`)
    /// inherits, so an app's own `[workload]` carries only what differs.
    ///
    /// Deliberately untyped, and that is the division of labour: these tables
    /// are a **pass-through payload** for the `app` KCL package's `Workload`
    /// schema. butler serialises the merged TOML into the values file 1:1 and
    /// models none of it, so the package stays the single validator — a field
    /// added upstream needs no butler release, and a mistyped workload key
    /// fails in KCL where the schema actually lives. The structs around it keep
    /// `deny_unknown_fields`, so a typo in butler's *own* config still fails
    /// here.
    #[serde(default)]
    pub workload_defaults: BTreeMap<String, toml::Value>,
    /// How a `[workload]` becomes Kubernetes objects.
    #[serde(default)]
    pub k8s: K8s,
    #[serde(default)]
    pub tilt: RootTilt,
    #[serde(default)]
    pub container: Container,
    /// Standalone repos only: the single app living at the repo root.
    pub app: Option<App>,
}

fn default_env() -> String {
    "dev".to_string()
}

/// The KCL package that renders a `[workload]`, and the image tag each
/// environment deploys.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct K8s {
    /// OCI reference of the renderer, without a version.
    #[serde(default = "default_package")]
    pub package: String,
    /// Package version. Pinned on purpose: an unpinned reference makes every
    /// render depend on whatever was published last.
    pub tag: Option<String>,
    /// Environment -> image tag. An environment not listed here deploys the tag
    /// named after it, so `dev` renders `:dev` with no configuration at all.
    #[serde(default)]
    pub image_tag: BTreeMap<String, String>,
    /// Directory the rendered manifests and their aggregate are written to,
    /// workspace-root-relative. `{env}` expands to the deployment environment,
    /// so a repo that renders several of them keeps `manifests/k8s/dev/` and
    /// `manifests/k8s/prod/` apart with one line of config.
    #[serde(default = "default_out_dir")]
    pub out_dir: String,
    /// Paths appended verbatim to the generated aggregate `kustomization.yaml`,
    /// relative to it. For objects that are environment fixtures rather than
    /// workload shape — e.g. the dev-only placeholder Secrets, which the package
    /// does not render and which no app should carry in its own config.
    #[serde(default)]
    pub extra_resources: Vec<String>,
}

impl Default for K8s {
    fn default() -> Self {
        Self {
            package: default_package(),
            tag: None,
            image_tag: BTreeMap::new(),
            out_dir: default_out_dir(),
            extra_resources: Vec::new(),
        }
    }
}

fn default_out_dir() -> String {
    "manifests/k8s/apps".to_string()
}

fn default_package() -> String {
    "oci://docker.io/yurikrupnik/app".to_string()
}

/// Which apps ship a container image beyond the ones that obviously do.
///
/// An app is a container app when it has a recognizable kind *and* ships k8s
/// manifests — that is derived, never declared. This section covers the one case
/// that cannot be derived: an app that ships an image but no manifests of its
/// own, so nothing in this repo deploys it while CI must still build and scan it.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Container {
    /// Workspace-root-relative app directories to include in the container set.
    #[serde(default)]
    pub extra: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootTilt {
    /// Infra services reached through a long-running local command.
    #[serde(default)]
    pub port_forward: Vec<PortForward>,
    /// Cluster resources that belong to no single app but that apps depend on.
    #[serde(default)]
    pub shared_resource: Vec<SharedResource>,
    /// Dockerfile path -> extra context paths it `COPY`s that belong to no app.
    #[serde(default)]
    pub dockerfile_only: BTreeMap<String, Vec<String>>,
    /// Churn excluded from every image context built from the workspace root —
    /// the `service` and `node` kinds, which both `COPY` whole subtrees in.
    #[serde(default)]
    pub context_ignore: Vec<String>,
}

/// Per-kind image conventions. Every field is optional: an unset kind means
/// apps of that kind must declare their own image inputs.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageDefaults {
    /// Compiled service (a cargo crate).
    pub service: Option<KindImage>,
    /// Static web app built by the JS toolchain.
    pub web: Option<KindImage>,
    /// Node SSR app (an Astro server build), run as a Node process.
    pub node: Option<KindImage>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KindImage {
    /// Dockerfile path, workspace-root-relative.
    pub dockerfile: String,
    /// Multi-stage build stage; falls back to `dockerfileTarget`.
    pub target: Option<String>,
    /// Name of the one build arg whose value is derived per app: the crate name
    /// for a service, `<app>/dist` for a web app, the app directory for a node
    /// app.
    pub build_arg: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortForward {
    pub name: String,
    /// Long-running command that holds the forward open.
    pub command: String,
    /// TCP port a readiness probe checks on localhost.
    pub probe_port: u16,
    #[serde(default = "probe_period_default")]
    pub probe_period_secs: u32,
}

const fn probe_period_default() -> u32 {
    5
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedResource {
    pub name: String,
    /// Kustomize directory, workspace-root-relative.
    pub kustomize: String,
    /// Individual objects to group under `name`, e.g. `zerg-shared-config:configmap`.
    pub objects: Vec<String>,
}

impl Root {
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = workspace_root.join(FILE);
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            eyre!(
                "reading {}: {e}\n\
                 the repo root needs a {FILE} declaring at least `registry`",
                path.display()
            )
        })?;
        let root: Self = toml::from_str(&raw).map_err(|e| eyre!("parsing {FILE}: {e}"))?;
        if root.registry.is_empty() {
            bail!("{FILE}: `registry` must not be empty");
        }
        Ok(root)
    }
}

// ---------------------------------------------------------------------------
// App level

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct App {
    #[serde(default)]
    pub tilt: AppTilt,
    /// Image build inputs. Omit unless this app departs from the repo
    /// convention in the root `[imageDefaults.<kind>]`.
    pub image: Option<Image>,
    /// What this app deploys — a pass-through payload for the `app` KCL
    /// package's `Workload` schema, merged over the root
    /// `[workloadDefaults.<kind>]` (see [`Root::workload_defaults`] for why
    /// butler models none of it). Its presence is what makes the app
    /// deployable: no `[workload]`, no manifests and no Tilt k8s resource.
    ///
    /// `image` must not appear here — it lives in the image facts, once.
    pub workload: Option<toml::Value>,
    /// Literal ConfigMap data, rendered by the package as `<name>-config`.
    /// Typed `{string: string}` because a ConfigMap's values *are* strings:
    /// `RATE_LIMIT_ENABLED = "true"` must stay the string `"true"`, and a bare
    /// TOML `true` here is a mistake worth failing on.
    pub config: Option<BTreeMap<String, String>>,
    /// ExternalSecret payload; pass-through like `workload`.
    pub external_secret: Option<toml::Value>,
    /// Per-environment overlays, deep-merged over the tables above.
    #[serde(default)]
    pub env: BTreeMap<String, AppEnv>,
}

/// One environment's diffs, layered over the app's own tables the way a
/// kustomize overlay layers over a base: nested tables merge, scalars and
/// arrays replace, right wins.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppEnv {
    pub workload: Option<toml::Value>,
    pub config: Option<BTreeMap<String, String>>,
    /// An ExternalSecret that only one environment has (a prod-only secret
    /// store) belongs here, not in the base.
    pub external_secret: Option<toml::Value>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppTilt {
    /// Host port for Tilt's `port_forwards`. The container side is the
    /// workload's own `port`, so the port is written once, not twice.
    pub host_port: Option<u16>,
    /// Other Tilt resources that must be ready first.
    #[serde(default)]
    pub resource_deps: Vec<String>,
    /// Overrides the kind-derived default (`backend` for a service, none for web).
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Image build inputs for an app that departs from the repo convention. Whatever
/// this declares is what both the Tiltfile and the nx `container` target build.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Image {
    /// Dockerfile path, workspace-root-relative.
    pub dockerfile: String,
    /// Build context, workspace-root-relative. `.` = the repo root.
    #[serde(default = "default_context")]
    pub context: String,
    /// Multi-stage build stage. Falls back to the root `dockerfileTarget` map.
    pub target: Option<String>,
    #[serde(default)]
    pub build_args: BTreeMap<String, String>,
    /// Image reference; `$REGISTRY` is substituted from the root config.
    pub tag: String,
}

fn default_context() -> String {
    ".".to_string()
}

/// Per-app config, keyed by workspace-root-relative directory.
///
/// Everything derivable is still derived — the generator finds every app in the
/// repo on its own, and its kind and image inputs come from what is on disk and
/// the repo convention. What this file carries is what no derivation can know:
///
///   * `[workload]` — what the app deploys, and the one thing that makes it
///     deployable at all;
///   * `[tilt]` — the dev-loop facts (a host port, an ordering constraint);
///   * `[image]` — only when the app departs from the repo convention.
///
/// An app with no `[workload]` still builds an image (that is the `[container]
/// extra` case), and an app with no file at all is not deployed by this repo.
pub type AppOverrides = BTreeMap<String, App>;

/// Load every `<dir>/butler.toml` below the root, plus the root file's own
/// `[app]` section (a single-app repo, keyed `.`).
pub fn load_app_overrides(workspace_root: &Path, root: &Root) -> Result<AppOverrides> {
    let mut out = AppOverrides::new();

    if let Some(app) = &root.app {
        out.insert(".".to_string(), app.clone());
    }

    for dir in walk_dirs(workspace_root) {
        let path = dir.join(FILE);
        if !path.is_file() {
            continue;
        }
        let rel = rel_dir(workspace_root, &dir)?;
        if rel == "." {
            // The root file; its app (if any) was handled above.
            continue;
        }
        let raw =
            std::fs::read_to_string(&path).map_err(|e| eyre!("reading {}: {e}", path.display()))?;
        let app: App =
            toml::from_str(&raw).map_err(|e| eyre!("parsing {}: {e}", path.display()))?;
        out.insert(rel, app);
    }

    Ok(out)
}

/// Directories never traversed while scanning for config files.
const PRUNED: &[&str] = &["node_modules", ".git", "dist", "target", ".nx", ".venv"];

fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        out.push(dir.clone());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || PRUNED.contains(&name.as_ref()) {
                continue;
            }
            stack.push(path);
        }
    }
    out
}

fn rel_dir(root: &Path, dir: &Path) -> Result<String> {
    let rel = dir
        .strip_prefix(root)
        .map_err(|_| eyre!("{} is outside the workspace", dir.display()))?;
    let s = rel.to_string_lossy().replace('\\', "/");
    Ok(if s.is_empty() { ".".into() } else { s })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_requires_a_registry() {
        let err = toml::from_str::<Root>("env = 'dev'").expect_err("registry is mandatory");
        assert!(err.to_string().contains("registry"), "{err}");
    }

    #[test]
    fn unknown_keys_fail_instead_of_being_ignored() {
        let err = toml::from_str::<App>("[tilt]\nhostPorts = 1")
            .expect_err("typo must not be silently dropped");
        assert!(err.to_string().contains("hostPorts"), "{err}");
    }

    /// The typed structs deny unknown keys; the workload payload does not,
    /// because KCL is what validates it. Both halves of that division of
    /// labour, in one test.
    #[test]
    fn a_workload_key_butler_never_heard_of_reaches_the_payload() {
        let app: App = toml::from_str(
            "[workload]\nsomeFutureField = 'ok'\n\
             [[workload.env]]\nname = 'PORT'\nvalue = '8080'\n\
             [env.prod.externalSecret]\nstore = 'gcp'\n",
        )
        .expect("an unknown workload key is the package's business, not butler's");
        let workload = app.workload.expect("workload");
        assert_eq!(
            workload["someFutureField"].as_str(),
            Some("ok"),
            "unknown workload keys must survive to the values file"
        );
        assert_eq!(workload["env"].as_array().expect("env array").len(), 1);
        assert!(app.env["prod"].external_secret.is_some());
    }

    #[test]
    fn the_k8s_package_defaults_but_the_tag_does_not() {
        let root: Root = toml::from_str("registry = 'acme'").expect("minimal root");
        assert_eq!(root.k8s.package, "oci://docker.io/yurikrupnik/app");
        assert!(root.k8s.tag.is_none(), "the version must be pinned by hand");
        assert!(root.workload_defaults.is_empty());
    }

    #[test]
    fn env_defaults_to_dev() {
        let root: Root = toml::from_str("registry = 'acme'").expect("minimal root config");
        assert_eq!(root.env, "dev");
        assert!(root.app.is_none());
    }

    #[test]
    fn root_app_section_describes_a_standalone_repo() {
        let root: Root = toml::from_str(
            "registry = 'acme'\n\
             [app.tilt]\nhostPort = 8080\n\
             [app.image]\ndockerfile = 'Dockerfile'\ntag = '$REGISTRY/svc:latest'\n",
        )
        .expect("standalone config");
        let app = root.app.expect("app section");
        assert_eq!(app.tilt.host_port, Some(8080));
        assert_eq!(app.image.expect("image").context, ".");
    }
}
