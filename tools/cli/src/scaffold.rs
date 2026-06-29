//! Deterministic project scaffolding. Generates a new app under
//! `{apps_dir}/<slug>` with NO `project.json` — the Nx targets come entirely from
//! inference (see `graph.rs`). That is the self-building loop: create the source +
//! Tiltfile, and it is instantly buildable / containerizable / scannable.
//!
//! Layout, name prefixes, Dockerfile paths, and the Tilt image namespace all come
//! from `Conventions` (see `config.rs`), so scaffolding adapts to any Nx repo.

use std::path::{Path, PathBuf};

use eyre::{Result, eyre};

use crate::config::Conventions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Rust,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSpec {
    /// Cargo crate / Nx project name, e.g. `zerg_notifier`.
    pub name: String,
    /// Directory name under `{apps_dir}`, e.g. `notifier`.
    pub dir: String,
    pub kind: AppKind,
    pub port: u16,
}

impl AppSpec {
    /// Build a spec from a user slug, applying the workspace conventions:
    /// `email-blast` + `zerg_` prefix -> dir `email-blast`, name `zerg_email_blast`.
    pub fn from_slug(slug: &str, kind: AppKind, port: u16, conv: &Conventions) -> Self {
        let dir = slug.trim().trim_matches('/').to_string();
        let name = conv.crate_name(&dir);
        Self {
            name,
            dir,
            kind,
            port,
        }
    }
}

fn cargo_toml(spec: &AppSpec) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ntokio = {{ workspace = true }}\neyre = {{ workspace = true }}\ntracing = {{ workspace = true }}\ntracing-subscriber = {{ workspace = true }}\n",
        name = spec.name,
    )
}

fn main_rs(spec: &AppSpec) -> String {
    format!(
        "#[tokio::main]\nasync fn main() -> eyre::Result<()> {{\n    tracing_subscriber::fmt::init();\n    let port: u16 = std::env::var(\"PORT\")\n        .ok()\n        .and_then(|p| p.parse().ok())\n        .unwrap_or({port});\n    tracing::info!(%port, \"{name} starting\");\n    Ok(())\n}}\n",
        port = spec.port,
        name = spec.name,
    )
}

/// Image reference for a generated Tiltfile: `{tilt_registry}/{image}` when a
/// registry namespace is configured, else the bare image name.
fn tilt_image(spec: &AppSpec, conv: &Conventions) -> String {
    let image = conv.image(&spec.dir);
    if conv.tilt_registry.is_empty() {
        image
    } else {
        format!("{}/{}", conv.tilt_registry, image)
    }
}

fn rust_tiltfile(spec: &AppSpec, conv: &Conventions) -> String {
    format!(
        "docker_build(\n  '{image}',\n  build_args={{'APP_NAME': '{name}'}},\n  context='{up}',\n  dockerfile='{up}/{dockerfile}',\n  target='rust',\n  live_update=[\n    sync('{apps}/{dir}/src', '/app/{apps}/{dir}/src'),\n  ],\n)\n",
        image = tilt_image(spec, conv),
        name = spec.name,
        up = conv.workspace_rel_prefix(),
        dockerfile = conv.rust_dockerfile,
        apps = conv.apps_dir,
        dir = spec.dir,
    )
}

fn static_package_json(spec: &AppSpec, conv: &Conventions) -> String {
    format!(
        "{{\n  \"name\": \"{image}\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"scripts\": {{\n    \"dev\": \"vite\",\n    \"build\": \"vite build\",\n    \"lint\": \"biome check --write .\"\n  }}\n}}\n",
        image = conv.image(&spec.dir),
    )
}

fn static_tiltfile(spec: &AppSpec, conv: &Conventions) -> String {
    format!(
        "docker_build(\n  '{image}',\n  '{up}',\n  target='nginx',\n  dockerfile='{up}/{dockerfile}',\n  build_args={{'DIST_PATH': '{apps}/{dir}/dist'}},\n)\n",
        image = tilt_image(spec, conv),
        up = conv.workspace_rel_prefix(),
        dockerfile = conv.static_dockerfile,
        apps = conv.apps_dir,
        dir = spec.dir,
    )
}

/// Add a member to the workspace `members` array, preserving formatting.
fn register_workspace_member(workspace_root: &Path, member: &str) -> Result<()> {
    let path = workspace_root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
    let members = doc["workspace"]["members"]
        .as_array_mut()
        .ok_or_else(|| eyre!("Cargo.toml has no [workspace] members array"))?;
    if members.iter().any(|v| v.as_str() == Some(member)) {
        return Ok(());
    }
    // Single-quoted, on its own indented line, to match the existing members style.
    let mut value: toml_edit::Value = format!("'{member}'")
        .parse()
        .map_err(|e| eyre!("could not format member entry: {e}"))?;
    value.decor_mut().set_prefix("\n    ");
    members.push_formatted(value);
    members.set_trailing("\n");
    members.set_trailing_comma(false);
    std::fs::write(&path, doc.to_string())?;
    Ok(())
}

fn write_file(path: &Path, contents: &str, written: &mut Vec<PathBuf>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    written.push(path.to_path_buf());
    Ok(())
}

/// Scaffold the app. Returns the list of files created/modified. Errors if the
/// target directory already exists (never clobber existing work).
pub fn scaffold(spec: &AppSpec, workspace_root: &Path, conv: &Conventions) -> Result<Vec<PathBuf>> {
    if spec.dir.is_empty() {
        return Err(eyre!("app slug must not be empty"));
    }
    let proj_root = workspace_root.join(&conv.apps_dir).join(&spec.dir);
    if proj_root.exists() {
        return Err(eyre!("{} already exists", proj_root.display()));
    }

    let mut written = Vec::new();
    match spec.kind {
        AppKind::Rust => {
            write_file(
                &proj_root.join("Cargo.toml"),
                &cargo_toml(spec),
                &mut written,
            )?;
            write_file(&proj_root.join("src/main.rs"), &main_rs(spec), &mut written)?;
            write_file(
                &proj_root.join("Tiltfile"),
                &rust_tiltfile(spec, conv),
                &mut written,
            )?;
            register_workspace_member(workspace_root, &format!("{}/{}", conv.apps_dir, spec.dir))?;
            written.push(workspace_root.join("Cargo.toml"));
        }
        AppKind::Static => {
            write_file(
                &proj_root.join("package.json"),
                &static_package_json(spec, conv),
                &mut written,
            )?;
            write_file(
                &proj_root.join("src/main.ts"),
                "console.log(\"app entry\");\n",
                &mut written,
            )?;
            write_file(
                &proj_root.join("Tiltfile"),
                &static_tiltfile(spec, conv),
                &mut written,
            )?;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_to_spec_applies_conventions() {
        let conv = Conventions::zerg();
        let s = AppSpec::from_slug("email-blast", AppKind::Rust, 8080, &conv);
        assert_eq!(s.dir, "email-blast");
        assert_eq!(s.name, "zerg_email_blast");
        assert_eq!(conv.image(&s.dir), "zerg-email-blast");
    }

    #[test]
    fn rust_tiltfile_tracks_apps_dir_depth() {
        // Shallow layout: app at apps/<dir> climbs two levels, not three.
        let conv = Conventions::default();
        let spec = AppSpec::from_slug("worker", AppKind::Rust, 8080, &conv);
        let tf = rust_tiltfile(&spec, &conv);
        assert!(tf.contains("context='../..'"));
        assert!(tf.contains("dockerfile='../../Dockerfile'"));
        assert!(tf.contains("sync('apps/worker/src', '/app/apps/worker/src')"));
        assert!(tf.contains("'worker'")); // no registry namespace, bare image
    }

    #[test]
    fn scaffold_rust_writes_files_and_registers_member() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(
            ws.path().join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\n    'libs/rpc',\n]\n",
        )
        .unwrap();

        let conv = Conventions::zerg();
        let spec = AppSpec::from_slug("notifier", AppKind::Rust, 50051, &conv);
        let written = scaffold(&spec, ws.path(), &conv).unwrap();

        assert!(ws.path().join("apps/zerg/notifier/Cargo.toml").exists());
        assert!(ws.path().join("apps/zerg/notifier/src/main.rs").exists());
        assert!(ws.path().join("apps/zerg/notifier/Tiltfile").exists());
        // NO project.json — targets are inferred.
        assert!(!ws.path().join("apps/zerg/notifier/project.json").exists());
        assert!(written.iter().any(|p| p.ends_with("Cargo.toml")));

        let root_cargo = std::fs::read_to_string(ws.path().join("Cargo.toml")).unwrap();
        assert!(root_cargo.contains("apps/zerg/notifier"));
        let main =
            std::fs::read_to_string(ws.path().join("apps/zerg/notifier/src/main.rs")).unwrap();
        assert!(main.contains("50051"));
    }

    #[test]
    fn scaffold_refuses_existing_directory() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let conv = Conventions::zerg();
        std::fs::create_dir_all(ws.path().join("apps/zerg/dup")).unwrap();
        let spec = AppSpec::from_slug("dup", AppKind::Rust, 8080, &conv);
        assert!(scaffold(&spec, ws.path(), &conv).is_err());
    }

    #[test]
    fn register_member_is_idempotent() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(
            ws.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\n    'apps/zerg/x',\n]\n",
        )
        .unwrap();
        register_workspace_member(ws.path(), "apps/zerg/x").unwrap();
        let txt = std::fs::read_to_string(ws.path().join("Cargo.toml")).unwrap();
        assert_eq!(txt.matches("apps/zerg/x").count(), 1);
    }
}
