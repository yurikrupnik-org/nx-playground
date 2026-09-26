//! Targets of a deployable app: `container`/`scan` (image build and CVE scan),
//! `tilt-gen`/`tilt-check` and `k8s-gen`/`k8s-check`. Mirrors
//! `tools/nx/container-targets.ts`, `tilt-targets.ts`, `k8s-targets.ts` and the
//! app predicates of `butler-config.ts`.
//!
//! The graph owns the LIST — an app qualifies by having a kind and declaring a
//! `[workload]` in its own `butler.toml` — and butler owns the LOGIC: the
//! generator targets only shell out to `butler tilt gen` / `butler k8s gen`,
//! whose `only=` closure and workload merge the graph cannot express. The image
//! facts are not derived a second time here either: they come from
//! [`container::resolve`], the resolution the Tiltfile generator shares.

use std::collections::BTreeMap;

use eyre::{Result, eyre};
use serde_json::json;

use super::crates::CargoCrate;
use crate::config::{Json, JsonMap, TargetConfig};
use crate::container::{self, Expected};
use crate::infer::Ctx;
use crate::k8s;
use crate::settings::App;
use crate::tilt::{self, Kind};

/// Candidate apps: any file that could give a directory under `apps/` a kind.
/// The kind and deployability checks do the filtering, so this only has to be
/// a superset — and it must cover the `[container] extra` apps, which have no
/// `butler.toml` to match on.
pub const APP_MARKER_FILES: &[&str] = &["Cargo.toml", "vite.config.ts", "astro.config.mjs"];

/// `containerEntry` in `plugin.ts`: `container`/`scan` for an app that has a
/// kind and is deployed, `None` otherwise. Declaring a `[workload]` is what
/// deploys an app, so it is what makes an image worth building; an app
/// deployed from outside this repo opts in through `[container] extra`.
/// `crate_at` reads (memoised) the app's own crate, whose name a service image
/// compiles.
pub fn container_targets(
    ctx: &Ctx,
    dir: &str,
    crate_at: &mut dyn FnMut(&str) -> Result<Option<CargoCrate>>,
) -> Result<Option<BTreeMap<String, TargetConfig>>> {
    let Some(root) = ctx.settings else {
        return Ok(None);
    };
    let Some(kind) = tilt::app_kind(ctx.workspace_root, dir) else {
        return Ok(None);
    };
    if !is_project(ctx, dir) {
        return Ok(None);
    }
    if !has_workload(ctx, dir) && !root.container.extra.iter().any(|e| e == dir) {
        return Ok(None);
    }
    // Only a service's image reads a name: the crate its Dockerfile compiles.
    let name = match kind {
        Kind::Service => crate_at(dir)?.map(|c| c.name),
        Kind::Web | Kind::Node => None,
    };
    let declared = app(ctx, dir).and_then(|a| a.image.as_ref());
    let image = container::resolve(root, kind, dir, name.as_deref(), declared)?;
    container_and_scan(&image).map(Some)
}

/// `appKind && isProject && hasWorkload`: the apps that get the generator
/// targets. Nothing without the root `butler.toml`.
pub fn is_workload_app(ctx: &Ctx, dir: &str) -> Result<bool> {
    Ok(ctx.settings.is_some()
        && tilt::app_kind(ctx.workspace_root, dir).is_some()
        && is_project(ctx, dir)
        && has_workload(ctx, dir))
}

/// `tilt-gen` / `tilt-check`.
pub fn tilt_targets(dir: &str) -> BTreeMap<String, TargetConfig> {
    // Everything that can change a generated Tiltfile: the app's own config
    // and kind markers, the repo config, the manifests the `only=` closure is
    // derived from, and the generator itself.
    let inputs = |extra: &[String]| -> Vec<Json> {
        let mut v = vec![
            json!({ "externalDependencies": [] }),
            json!(format!("{{workspaceRoot}}/{dir}/butler.toml")),
            json!(format!("{{workspaceRoot}}/{dir}/project.json")),
            json!(format!("{{workspaceRoot}}/{dir}/Cargo.toml")),
            json!(format!("{{workspaceRoot}}/{dir}/vite.config.ts")),
            json!(format!("{{workspaceRoot}}/{dir}/index.html")),
            json!(format!("{{workspaceRoot}}/{dir}/astro.config.mjs")),
            json!("{workspaceRoot}/butler.toml"),
            json!("{workspaceRoot}/Cargo.toml"),
            json!("{workspaceRoot}/Cargo.lock"),
            json!("{workspaceRoot}/apps/*/*/Cargo.toml"),
            json!("{workspaceRoot}/libs/**/Cargo.toml"),
            json!("{workspaceRoot}/apps/butler/cli/src/**/*.rs"),
        ];
        v.extend(extra.iter().map(|s| json!(s)));
        v
    };
    let gen_cmd = format!("cargo run --quiet -p butler -- tilt gen --app {dir}");
    BTreeMap::from([
        (
            "tilt-gen".to_string(),
            generator(
                gen_cmd.clone(),
                inputs(&[]),
                Some(vec!["{projectRoot}/Tiltfile".to_string()]),
                format!("Generate {dir}/Tiltfile from butler.toml + the project graph"),
                "tilt",
            ),
        ),
        (
            "tilt-check".to_string(),
            generator(
                format!("{gen_cmd} --check"),
                inputs(&[format!("{{workspaceRoot}}/{dir}/Tiltfile")]),
                None,
                format!("Fail if {dir}/Tiltfile has drifted from the generator"),
                "tilt",
            ),
        ),
    ])
}

/// Root `butler.toml` `[container] extra`: apps that ship an image but declare
/// no workload, so nothing else would make them deployable.
pub fn container_extra(ctx: &Ctx) -> Vec<String> {
    ctx.settings
        .map(|s| s.container.extra.clone())
        .unwrap_or_default()
}

/// `k8s-gen` / `k8s-check`. The rendered file lands in `[k8s] outDir` with
/// `{env}` expanded — the same string butler writes to, because nx caches on
/// declared outputs.
pub fn k8s_targets(ctx: &Ctx, dir: &str) -> Result<BTreeMap<String, TargetConfig>> {
    let root = ctx
        .settings
        .ok_or_else(|| eyre!("{dir}: k8s targets need the root butler.toml"))?;
    let out_dir = k8s::out_dir(root);
    // `dir` is under apps/, so the root-app fallback to a project name never
    // applies — the name is the path, as `derivedImageName` spells it.
    let image_name = tilt::derived_image_name(dir, "");
    let rendered = format!("{{workspaceRoot}}/{out_dir}/{image_name}.yaml");

    // Everything that can change the values file or its rendered manifests:
    // the app's config, the kind markers (the kind picks
    // `[workloadDefaults.<kind>]`), the repo config (registry, env, image
    // conventions, workload defaults, the pinned KCL package), the generator.
    let inputs = |extra: &[&str]| -> Vec<Json> {
        let mut v = vec![
            json!({ "externalDependencies": [] }),
            json!(format!("{{workspaceRoot}}/{dir}/butler.toml")),
            json!(format!("{{workspaceRoot}}/{dir}/Cargo.toml")),
            json!(format!("{{workspaceRoot}}/{dir}/vite.config.ts")),
            json!(format!("{{workspaceRoot}}/{dir}/astro.config.mjs")),
            json!("{workspaceRoot}/butler.toml"),
            json!("{workspaceRoot}/apps/butler/cli/src/**/*.rs"),
        ];
        v.extend(extra.iter().map(|s| json!(s)));
        v
    };
    let values = format!("{{workspaceRoot}}/{dir}/k8s/values.yaml");
    let gen_cmd = format!("cargo run --quiet -p butler -- k8s gen --app {dir}");
    Ok(BTreeMap::from([
        (
            "k8s-gen".to_string(),
            generator(
                gen_cmd.clone(),
                inputs(&[]),
                Some(vec![
                    "{projectRoot}/k8s/values.yaml".to_string(),
                    rendered.clone(),
                ]),
                format!(
                    "Generate {dir}/k8s/values.yaml and render it to {out_dir}/{image_name}.yaml"
                ),
                "kubernetes",
            ),
        ),
        (
            "k8s-check".to_string(),
            generator(
                format!("{gen_cmd} --check"),
                inputs(&[&values, &rendered]),
                None,
                format!(
                    "Fail if {image_name}'s values or rendered manifests have drifted from butler.toml"
                ),
                "kubernetes",
            ),
        ),
    ]))
}

/// The `command` shape both generators' targets share: run from the workspace
/// root, cached on `inputs`.
fn generator(
    command: String,
    inputs: Vec<Json>,
    outputs: Option<Vec<String>>,
    description: String,
    technology: &str,
) -> TargetConfig {
    TargetConfig {
        command: Some(command),
        options: Some(JsonMap::from_iter([(
            "cwd".to_string(),
            json!("{workspaceRoot}"),
        )])),
        cache: Some(true),
        inputs: Some(inputs),
        outputs,
        metadata: Some(json!({ "description": description, "technologies": [technology] })),
        ..Default::default()
    }
}

/// `containerAndScan`: the build (`@nx-tools/nx-container`) and the trivy scan
/// of one resolved image.
fn container_and_scan(image: &Expected) -> Result<BTreeMap<String, TargetConfig>> {
    let mut container = json!({
        "executor": "@nx-tools/nx-container:build",
        "options": {
            "file": image.file,
            "context": image.context,
            "build-args": image.build_args,
            "target": image.stage,
            "tags": image.tags,
            "push": image.push,
        },
        "configurations": {
            "ci": {
                "push": true,
                "cache-from": [format!("type=registry,ref=$BUILDCACHE/{}", image.image_name)],
                "cache-to": [format!(
                    "type=registry,ref=$BUILDCACHE/{},mode=max,image-manifest=true,oci-mediatypes=true",
                    image.image_name
                )],
                "metadata": {
                    "images": [image.repository],
                    "tags": [
                        "type=sha",
                        "type=ref,event=branch",
                        "type=ref,event=pr",
                        "type=raw,value=$APP_VERSION,enable=$ENABLE_VERSION",
                    ],
                },
            },
        },
        "metadata": {
            "description": format!("Build {} from {}", image.repository, image.file),
            "technologies": ["docker"],
        },
    });
    // A web image is the static output of the local build; a service or a
    // node app is built inside the image, so nothing has to happen first.
    if !image.container_depends_on.is_empty() {
        container["dependsOn"] = json!(image.container_depends_on);
    }
    let scan = json!({
        "executor": "nx:run-commands",
        "dependsOn": image.scan_depends_on,
        "options": {
            "command": image.scan_command,
            "cwd": "{workspaceRoot}",
        },
        // CI scans the immutable sha tag the push produced and reports through
        // GitHub code scanning, so the SARIF filename must be app-unique.
        "configurations": { "ci": { "command": image.scan_ci_command } },
        "metadata": {
            "description": format!("Scan {} for CRITICAL/HIGH CVEs", image.repository),
            "technologies": ["docker"],
        },
    });
    Ok(BTreeMap::from([
        ("container".to_string(), serde_json::from_value(container)?),
        ("scan".to_string(), serde_json::from_value(scan)?),
    ]))
}

/// `isProject`: a directory nx already treats as a project, so inferred
/// targets merge onto it instead of conjuring a node.
fn is_project(ctx: &Ctx, dir: &str) -> bool {
    let abs = ctx.workspace_root.join(dir);
    ["project.json", "package.json", "Cargo.toml"]
        .iter()
        .any(|f| abs.join(f).exists())
}

/// The app's own `butler.toml`, if it has one.
fn app<'a>(ctx: &Ctx<'a>, dir: &str) -> Option<&'a App> {
    ctx.overrides.and_then(|o| o.get(dir))
}

/// `hasWorkload`: the "this gets deployed" signal. The manifests are generated
/// from this table, so its presence is the fact, not that of their output.
fn has_workload(ctx: &Ctx, dir: &str) -> bool {
    app(ctx, dir).is_some_and(|a| a.workload.is_some())
}
