//! Repo conventions, resolved from `nx.json` plugin options so one binary serves
//! any Nx workspace. Nothing `zerg`-specific is baked into the engine: every name
//! prefix, directory, and Dockerfile path lives here and defaults to generic
//! values. A consuming repo configures them once under its plugin entry, e.g.:
//!
//! ```jsonc
//! // nx.json
//! { "plugin": "@zerg/nx", "options": {
//!     "appsDir": "apps", "imagePrefix": "", "cratePrefix": "",
//!     "rustDockerfile": "Dockerfile", "staticDockerfile": "Dockerfile",
//!     "registryEnv": "REGISTRY", "tiltRegistry": "" } }
//! ```
//!
//! Resolution order: explicit `--config-json` (the shim forwards Nx's options on
//! the graph path) > the matching `nx.json` plugin entry (the CLI path) > defaults.

use std::path::Path;

use eyre::{Result, eyre};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Conventions {
    /// Parent directory new apps are scaffolded under, e.g. `apps` or `apps/zerg`.
    pub apps_dir: String,
    /// Container-image prefix. Image = `{image_prefix}{dir}` (also the Nx project
    /// name for static apps).
    pub image_prefix: String,
    /// Crate/project name prefix for scaffolding. Name = `{crate_prefix}{dir_}`,
    /// where `dir_` is the dir with `-` replaced by `_`.
    pub crate_prefix: String,
    /// Workspace-relative Dockerfile for Rust apps' `container` target.
    pub rust_dockerfile: String,
    /// Workspace-relative Dockerfile for static apps' `container` target.
    pub static_dockerfile: String,
    /// Env var holding the registry host; referenced as `${registry_env}` in
    /// generated commands (kept as a literal var so CI resolves it at runtime).
    pub registry_env: String,
    /// Image namespace written into generated Tiltfiles (e.g. a Docker Hub user).
    /// Empty leaves the bare image name.
    pub tilt_registry: String,
}

impl Default for Conventions {
    fn default() -> Self {
        Self {
            apps_dir: "apps".to_string(),
            image_prefix: String::new(),
            crate_prefix: String::new(),
            rust_dockerfile: "Dockerfile".to_string(),
            static_dockerfile: "Dockerfile".to_string(),
            registry_env: "REGISTRY".to_string(),
            tilt_registry: String::new(),
        }
    }
}

impl Conventions {
    /// `$REGISTRY`-style reference used inside generated command/tag strings.
    pub fn registry_ref(&self) -> String {
        format!("${}", self.registry_env)
    }

    /// Container image base for an app directory: `{image_prefix}{dir}`.
    pub fn image(&self, dir: &str) -> String {
        format!("{}{}", self.image_prefix, dir)
    }

    /// Crate/project name for a scaffolded app: `{crate_prefix}{dir_underscored}`.
    pub fn crate_name(&self, dir: &str) -> String {
        format!("{}{}", self.crate_prefix, dir.replace('-', "_"))
    }

    /// Relative path from an app dir (`{apps_dir}/{slug}`) back to the workspace
    /// root, e.g. `apps/zerg` -> `../../..`. Used for Tiltfile `context`/dockerfile.
    pub fn workspace_rel_prefix(&self) -> String {
        let depth = self.apps_dir.split('/').filter(|s| !s.is_empty()).count() + 1;
        vec![".."; depth].join("/")
    }

    /// Resolve conventions for `workspace_root`. `inline` (the shim-forwarded Nx
    /// options as JSON) wins; otherwise read `nx.json`; otherwise defaults.
    pub fn resolve(workspace_root: &Path, inline: Option<&str>) -> Result<Self> {
        if let Some(json) = inline {
            return serde_json::from_str(json).map_err(|e| eyre!("invalid --config-json: {e}"));
        }
        Ok(Self::from_nx_json(workspace_root).unwrap_or_default())
    }

    /// Read the options of the plugin entry that references this tool from
    /// `nx.json`. Matched by name substring so it works whether the plugin is
    /// registered as a local path (`./tools/nx-zerg`) or a package (`@zerg/nx`).
    fn from_nx_json(workspace_root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(workspace_root.join("nx.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let plugins = v.get("plugins")?.as_array()?;
        for p in plugins {
            let Some(obj) = p.as_object() else { continue };
            let Some(name) = obj.get("plugin").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if name.contains("nx-zerg") || name.contains("@zerg/nx") {
                let opts = obj
                    .get("options")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                return serde_json::from_value(opts).ok();
            }
        }
        None
    }
}

#[cfg(test)]
impl Conventions {
    /// The `zerg` workspace's conventions — used by engine/scaffold unit tests to
    /// assert behavior parity with the hand-written `project.json` they replaced.
    pub fn zerg() -> Self {
        Self {
            apps_dir: "apps/zerg".to_string(),
            image_prefix: "zerg-".to_string(),
            crate_prefix: "zerg_".to_string(),
            rust_dockerfile: "manifests/dockers/rust.Dockerfile".to_string(),
            static_dockerfile: "manifests/dockers/Dockerfile".to_string(),
            registry_env: "REGISTRY".to_string(),
            tilt_registry: "yurikrupnik".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_generic() {
        let c = Conventions::default();
        assert_eq!(c.image("api"), "api");
        assert_eq!(c.crate_name("email-blast"), "email_blast");
        assert_eq!(c.registry_ref(), "$REGISTRY");
        assert_eq!(c.workspace_rel_prefix(), "../..");
    }

    #[test]
    fn zerg_prefixes_and_depth() {
        let c = Conventions::zerg();
        assert_eq!(c.image("api"), "zerg-api");
        assert_eq!(c.crate_name("email-blast"), "zerg_email_blast");
        assert_eq!(c.workspace_rel_prefix(), "../../..");
    }

    #[test]
    fn inline_json_overrides_with_camel_case_keys() {
        let c = Conventions::resolve(
            Path::new("/nonexistent"),
            Some(r#"{"appsDir":"services","imagePrefix":"acme-"}"#),
        )
        .unwrap();
        assert_eq!(c.apps_dir, "services");
        assert_eq!(c.image("worker"), "acme-worker");
        // Unset fields fall back to defaults.
        assert_eq!(c.rust_dockerfile, "Dockerfile");
    }

    #[test]
    fn reads_options_from_nx_json_plugin_entry() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(
            ws.path().join("nx.json"),
            r#"{ "plugins": [ "@monodon/rust",
                { "plugin": "./tools/nx-zerg",
                  "options": { "appsDir": "apps/zerg", "imagePrefix": "zerg-" } } ] }"#,
        )
        .unwrap();
        let c = Conventions::resolve(ws.path(), None).unwrap();
        assert_eq!(c.apps_dir, "apps/zerg");
        assert_eq!(c.image("api"), "zerg-api");
    }

    #[test]
    fn missing_nx_json_yields_defaults() {
        let ws = tempfile::tempdir().unwrap();
        let c = Conventions::resolve(ws.path(), None).unwrap();
        assert_eq!(c, Conventions::default());
    }
}
