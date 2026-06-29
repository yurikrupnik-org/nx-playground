//! Nx `createNodesV2` inference engine. Given the Tiltfiles Nx matched, emit the
//! project nodes (targets) for each deployable app. This is the single source of
//! truth the thin JS shim (`tools/nx-zerg/index.js`) shells out to.
//!
//! Marker = any `Tiltfile`. Deployability (not the path) decides inference:
//!   - sibling `Cargo.toml` with `[package]` -> Rust app (build/test/lint/run + container + scan)
//!   - sibling `package.json` with a `build` script -> static app (container dependsOn build + scan)
//!
//! All image names / Dockerfile paths / registry come from `Conventions`
//! (see `config.rs`), so the same engine works in any Nx repo.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::config::Conventions;

/// `[package] name` from a Cargo.toml, or `None` for virtual/workspace manifests.
fn cargo_package_name(cargo_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(cargo_path).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    doc.get("package")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// Whether a package.json declares a `build` script.
fn has_build_script(pkg_path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(pkg_path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    v.get("scripts")
        .and_then(|s| s.get("build"))
        .and_then(Value::as_str)
        .is_some()
}

fn ci_image_metadata(image: &str, conv: &Conventions) -> Value {
    let reg = conv.registry_ref();
    json!({
        "push": true,
        "metadata": {
            "images": [format!("{reg}/{image}")],
            "tags": [
                "type=sha",
                "type=ref,event=branch",
                "type=ref,event=pr",
                "type=raw,value=$APP_VERSION,enable=$ENABLE_VERSION"
            ]
        }
    })
}

fn scan_target(image: &str, conv: &Conventions) -> Value {
    let reg = conv.registry_ref();
    json!({
        "executor": "nx:run-commands",
        "dependsOn": ["container"],
        "options": {
            "command": format!("trivy image {reg}/{image}:latest --severity CRITICAL,HIGH --exit-code 0"),
            "cwd": "{workspaceRoot}"
        },
        "configurations": {
            "ci": {
                "command": format!("trivy image {reg}/{image}:sha-$(echo $SHORT_SHA | cut -c1-7) --severity CRITICAL,HIGH --format sarif --output trivy-{image}.sarif")
            }
        }
    })
}

fn rust_targets(crate_name: &str, image: &str, conv: &Conventions) -> Value {
    let reg = conv.registry_ref();
    json!({
        "build": {
            "cache": true,
            "executor": "nx:run-commands",
            "options": { "command": format!("cargo build --package {crate_name}"), "cwd": "{workspaceRoot}" },
            "configurations": { "production": { "command": format!("cargo build --package {crate_name} --release") } },
            "outputs": ["{workspaceRoot}/target"]
        },
        "test": {
            "cache": true,
            "executor": "nx:run-commands",
            "options": { "command": format!("cargo test --package {crate_name}"), "cwd": "{workspaceRoot}" }
        },
        "lint": {
            "cache": true,
            "executor": "nx:run-commands",
            "options": { "command": format!("cargo clippy --package {crate_name}"), "cwd": "{workspaceRoot}" }
        },
        "run": {
            "executor": "nx:run-commands",
            "options": { "command": format!("cargo run --package {crate_name}"), "cwd": "{workspaceRoot}" },
            "configurations": { "production": { "command": format!("cargo run --package {crate_name} --release") } }
        },
        "container": {
            "executor": "@nx-tools/nx-container:build",
            "options": {
                "file": conv.rust_dockerfile,
                "context": ".",
                "build-args": [format!("APP_NAME={crate_name}")],
                "tags": [format!("{reg}/{image}:latest")],
                "push": false
            },
            "configurations": { "ci": ci_image_metadata(image, conv) }
        },
        "scan": scan_target(image, conv)
    })
}

fn static_targets(root: &str, image: &str, conv: &Conventions) -> Value {
    let reg = conv.registry_ref();
    json!({
        "container": {
            "executor": "@nx-tools/nx-container:build",
            "dependsOn": ["build"],
            "options": {
                "file": conv.static_dockerfile,
                "context": ".",
                "build-args": [format!("DIST_PATH={root}/dist")],
                "tags": [format!("{reg}/{image}:latest")],
                "push": false
            },
            "configurations": { "ci": ci_image_metadata(image, conv) }
        },
        "scan": scan_target(image, conv)
    })
}

/// Infer the project contributed by one (workspace-relative) Tiltfile, or `None`
/// when the directory is not a deployable app. Returns `(project_root, project)`.
pub fn infer_app(
    tiltfile_rel: &str,
    workspace_root: &Path,
    conv: &Conventions,
) -> Option<(String, Value)> {
    let rel = Path::new(tiltfile_rel);
    let parent = rel.parent()?;
    let root = parent.to_string_lossy().replace('\\', "/");
    let dir = parent.file_name()?.to_string_lossy().to_string();
    let abs_root = workspace_root.join(&root);
    let image = conv.image(&dir);

    if let Some(crate_name) = cargo_package_name(&abs_root.join("Cargo.toml")) {
        let project = json!({
            "name": crate_name,
            "projectType": "application",
            "sourceRoot": format!("{root}/src"),
            "targets": rust_targets(&crate_name, &image, conv)
        });
        return Some((root, project));
    }

    if has_build_script(&abs_root.join("package.json")) {
        let project = json!({
            "name": image,
            "projectType": "application",
            "sourceRoot": format!("{root}/src"),
            "targets": static_targets(&root, &image, conv)
        });
        return Some((root, project));
    }

    None
}

/// Build the full `CreateNodesV2` result array for the given Tiltfiles.
pub fn nodes_for(files: &[String], workspace_root: &Path, conv: &Conventions) -> Value {
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let result = match infer_app(f, workspace_root, conv) {
            Some((root, project)) => {
                let mut projects = Map::new();
                projects.insert(root, project);
                let mut r = Map::new();
                r.insert("projects".to_string(), Value::Object(projects));
                Value::Object(r)
            }
            None => Value::Object(Map::new()),
        };
        out.push(Value::Array(vec![Value::String(f.clone()), result]));
    }
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn rust_app_gets_six_targets_and_derived_image() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join("apps/zerg/notifier");
        write(
            &root.join("Cargo.toml"),
            "[package]\nname = \"zerg_notifier\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        write(&root.join("Tiltfile"), "k8s_yaml([])\n");

        let (proj_root, project) = infer_app(
            "apps/zerg/notifier/Tiltfile",
            ws.path(),
            &Conventions::zerg(),
        )
        .expect("inferred");
        assert_eq!(proj_root, "apps/zerg/notifier");
        assert_eq!(project["name"], "zerg_notifier");
        let targets = project["targets"].as_object().unwrap();
        let mut keys: Vec<_> = targets.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["build", "container", "lint", "run", "scan", "test"]);
        assert_eq!(
            project["targets"]["container"]["options"]["file"],
            "manifests/dockers/rust.Dockerfile"
        );
        assert_eq!(
            project["targets"]["container"]["options"]["build-args"][0],
            "APP_NAME=zerg_notifier"
        );
        assert_eq!(
            project["targets"]["container"]["options"]["tags"][0],
            "$REGISTRY/zerg-notifier:latest"
        );
        assert_eq!(project["targets"]["scan"]["dependsOn"][0], "container");
    }

    #[test]
    fn static_app_gets_container_and_scan_only() {
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join("apps/zerg/web");
        write(
            &root.join("package.json"),
            "{\"name\":\"web\",\"scripts\":{\"build\":\"vite build\"}}",
        );
        write(&root.join("Tiltfile"), "k8s_yaml([])\n");

        let (_, project) =
            infer_app("apps/zerg/web/Tiltfile", ws.path(), &Conventions::zerg()).expect("inferred");
        assert_eq!(project["name"], "zerg-web");
        let targets = project["targets"].as_object().unwrap();
        let mut keys: Vec<_> = targets.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["container", "scan"]);
        assert_eq!(project["targets"]["container"]["dependsOn"][0], "build");
        assert_eq!(
            project["targets"]["container"]["options"]["build-args"][0],
            "DIST_PATH=apps/zerg/web/dist"
        );
    }

    #[test]
    fn non_app_directory_is_skipped() {
        let ws = tempfile::tempdir().unwrap();
        write(
            &ws.path().join("apps/zerg/shared/Tiltfile"),
            "k8s_yaml([])\n",
        );
        assert!(infer_app("apps/zerg/shared/Tiltfile", ws.path(), &Conventions::zerg()).is_none());
    }

    #[test]
    fn nodes_for_emits_one_entry_per_file() {
        let ws = tempfile::tempdir().unwrap();
        write(
            &ws.path().join("apps/zerg/shared/Tiltfile"),
            "k8s_yaml([])\n",
        );
        let nodes = nodes_for(
            &["apps/zerg/shared/Tiltfile".to_string()],
            ws.path(),
            &Conventions::zerg(),
        );
        let arr = nodes.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0][0], "apps/zerg/shared/Tiltfile");
        assert_eq!(arr[0][1], json!({})); // skipped -> empty result, but still present
    }

    #[test]
    fn generic_defaults_need_no_zerg_assumptions() {
        // A repo with apps under `services/`, no prefixes, plain `Dockerfile`.
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join("services/api");
        write(
            &root.join("Cargo.toml"),
            "[package]\nname = \"api\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        write(&root.join("Tiltfile"), "k8s_yaml([])\n");

        let (_, project) = infer_app("services/api/Tiltfile", ws.path(), &Conventions::default())
            .expect("inferred");
        assert_eq!(project["name"], "api");
        assert_eq!(
            project["targets"]["container"]["options"]["file"],
            "Dockerfile"
        );
        assert_eq!(
            project["targets"]["container"]["options"]["tags"][0],
            "$REGISTRY/api:latest"
        );
    }
}
