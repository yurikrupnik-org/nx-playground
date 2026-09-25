//! The one Rust image resolution: the facts every app's image is built from.
//!
//! The image facts live in `butler.toml` (see [`crate::settings`]) and are
//! resolved once, here ([`resolve`]): [`crate::tilt`] builds the Tiltfile's
//! `docker_build` from them and butler's native inference
//! (`infer::native::apps`) builds the `container`/`scan` targets from them.
//! nx gets the same targets from `tools/nx/container-targets.ts`, a TS mirror —
//! exactly the kind of copy that drifts — and `butler graph verify` catches
//! that drift by comparing every option of every `container`/`scan` target in
//! an `nx graph --file` dump against butler's own graph.
//!
//! Among the resolved facts is the build stage. It has to be: `buildx` with no
//! `target` builds a Dockerfile's *last* stage, and the web Dockerfile's last
//! stage is a static file server with no `/api` reverse proxy. An image built
//! from it serves the SPA and silently drops every API call the Deployments
//! configure a proxy upstream for — a failure no build log shows.

use eyre::Result;

use crate::settings::{self, Root};
use crate::tilt;

/// Everything the `container`/`scan` pair must carry for one app, in the exact
/// spelling nx's graph uses (workspace-root-relative paths, `KEY=VALUE` build
/// args, `$REGISTRY` unsubstituted). The native inference builds the targets
/// from this value (`infer::native::apps`), so there is one Rust resolution
/// for the Tiltfile and the inferred targets.
#[derive(Debug)]
pub struct Expected {
    pub file: String,
    pub context: String,
    /// Resolved multi-stage build stage, via [`tilt::resolve_stage`] — the one
    /// place the fallback lives.
    pub stage: String,
    /// `KEY=VALUE` pairs, sorted by key.
    pub build_args: Vec<String>,
    pub tags: Vec<String>,
    /// The tag without its version, e.g. `$REGISTRY/zerg-api`: CI's metadata
    /// image and what the CVE scan names.
    pub repository: String,
    /// Bare image name, e.g. `zerg-api`: the layer-cache ref and the SARIF
    /// filename.
    pub image_name: String,
    pub push: bool,
    /// `["build"]` for a static web app, whose image is its local build output;
    /// empty for a service and for a node app, which both build inside the
    /// image.
    pub container_depends_on: Vec<String>,
    pub scan_depends_on: Vec<String>,
    pub scan_command: String,
    pub scan_ci_command: String,
}

/// One app's image facts. `name` is as in [`tilt::image_facts`].
pub fn resolve(
    root: &Root,
    kind: tilt::Kind,
    dir: &str,
    name: Option<&str>,
    declared: Option<&settings::Image>,
) -> Result<Expected> {
    let facts = tilt::image_facts(root, &kind, dir, name, declared)?;
    // Resolved through tilt's helper, so CI's `target` and the Tiltfile's
    // can never be resolved two different ways.
    let stage = tilt::resolve_stage(root, &facts, dir)?;

    let repository = repository(&facts.tag).to_string();
    let image_name = repository
        .rsplit('/')
        .next()
        .unwrap_or(&repository)
        .to_string();

    Ok(Expected {
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
             cut -c1-7) --severity CRITICAL,HIGH --format sarif --output trivy-{image_name}.sarif"
        ),
        repository,
        image_name,
    })
}

/// `$REGISTRY/zerg-api:latest` -> `$REGISTRY/zerg-api`. A port in a registry
/// host (`localhost:5000/x`) is not a tag.
fn repository(tag: &str) -> &str {
    match tag.rsplit_once(':') {
        Some((base, t)) if !t.contains('/') => base,
        _ => tag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
