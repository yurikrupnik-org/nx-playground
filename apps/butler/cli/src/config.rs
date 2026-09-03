//! Nx-compatible configuration surface: `nx.json` and `project.json`.
//!
//! Butler reads the same files nx does but owns the interpretation. The
//! supported subset is the one that appears in real workspaces of this shape:
//! `nx:run-commands` targets (command/commands/cwd/parallel), `cache`,
//! `inputs` (string patterns + named-input refs), `outputs`, string
//! `dependsOn`, and per-target `configurations`. Anything outside that subset
//! fails loudly instead of silently producing a wrong plan.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, bail, eyre};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NxJson {
    pub target_defaults: BTreeMap<String, TargetConfig>,
    pub named_inputs: BTreeMap<String, Vec<serde_json::Value>>,
}

impl NxJson {
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = workspace_root.join("nx.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        serde_json::from_str(&raw).map_err(|e| eyre!("parsing nx.json: {e}"))
    }

    /// Named inputs with the nx built-in defaults filled in when absent.
    pub fn named_inputs(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let mut out = BTreeMap::new();
        for (name, patterns) in &self.named_inputs {
            let mut list = Vec::with_capacity(patterns.len());
            for p in patterns {
                match p {
                    serde_json::Value::String(s) => list.push(s.clone()),
                    other => bail!(
                        "namedInputs.{name}: unsupported non-string input {other} \
                         (runtime/env inputs are not supported)"
                    ),
                }
            }
            out.insert(name.clone(), list);
        }
        out.entry("default".into())
            .or_insert_with(|| vec!["{projectRoot}/**/*".into()]);
        Ok(out)
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectJson {
    pub name: Option<String>,
    pub project_type: Option<String>,
    pub tags: Vec<String>,
    pub targets: BTreeMap<String, TargetConfig>,
}

impl ProjectJson {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        serde_json::from_str(&raw).map_err(|e| eyre!("parsing {}: {e}", path.display()))
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct TargetConfig {
    pub executor: Option<String>,
    pub cache: Option<bool>,
    pub inputs: Option<Vec<serde_json::Value>>,
    pub outputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<serde_json::Value>>,
    pub options: RunOptions,
    pub configurations: BTreeMap<String, RunOptions>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct RunOptions {
    pub command: Option<String>,
    pub commands: Option<Vec<serde_json::Value>>,
    pub cwd: Option<String>,
    pub parallel: Option<bool>,
}

/// A target after merging `targetDefaults`, options, and the selected
/// configuration — everything the runner needs, nothing raw.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget {
    pub executor: String,
    pub commands: Vec<String>,
    /// Run `commands` concurrently (nx run-commands default) or in order.
    pub parallel_commands: bool,
    /// Workspace-root-relative working directory.
    pub cwd: String,
    pub cache: bool,
    pub inputs: Option<Vec<String>>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
}

impl TargetConfig {
    /// Overlay `self` (project-level) on top of target defaults.
    pub fn merged_over(&self, defaults: Option<&TargetConfig>) -> TargetConfig {
        let Some(d) = defaults else {
            return self.clone();
        };
        TargetConfig {
            executor: self.executor.clone().or_else(|| d.executor.clone()),
            cache: self.cache.or(d.cache),
            inputs: self.inputs.clone().or_else(|| d.inputs.clone()),
            outputs: self.outputs.clone().or_else(|| d.outputs.clone()),
            depends_on: self.depends_on.clone().or_else(|| d.depends_on.clone()),
            options: self.options.clone(),
            configurations: self.configurations.clone(),
        }
    }

    pub fn resolve(
        &self,
        project_name: &str,
        project_root: &str,
        target_name: &str,
        configuration: Option<&str>,
    ) -> Result<ResolvedTarget> {
        let ctx = || format!("{project_name}:{target_name}");

        let mut opts = self.options.clone();
        if let Some(cfg) = configuration {
            if let Some(over) = self.configurations.get(cfg) {
                if over.command.is_some() {
                    opts.command = over.command.clone();
                    opts.commands = None;
                }
                if over.commands.is_some() {
                    opts.commands = over.commands.clone();
                    opts.command = None;
                }
                if over.cwd.is_some() {
                    opts.cwd = over.cwd.clone();
                }
                if over.parallel.is_some() {
                    opts.parallel = over.parallel;
                }
            }
        }

        let mut commands = Vec::new();
        if let Some(c) = &opts.command {
            commands.push(c.clone());
        }
        if let Some(list) = &opts.commands {
            for entry in list {
                match entry {
                    serde_json::Value::String(s) => commands.push(s.clone()),
                    serde_json::Value::Object(o) => match o.get("command") {
                        Some(serde_json::Value::String(s)) => commands.push(s.clone()),
                        _ => bail!("{}: commands entry without string `command`", ctx()),
                    },
                    other => bail!("{}: unsupported commands entry {other}", ctx()),
                }
            }
        }

        let executor = self
            .executor
            .clone()
            .unwrap_or_else(|| "nx:run-commands".into());
        match executor.as_str() {
            "nx:run-commands" | "butler:cargo" | "butler:script" => {
                if commands.is_empty() {
                    bail!("{}: no command to run", ctx());
                }
            }
            other => bail!(
                "{}: executor `{other}` is not supported by butler; \
                 run it through nx (butler coexists with nx for such targets)",
                ctx()
            ),
        }

        let mut depends_on = Vec::new();
        for dep in self.depends_on.iter().flatten() {
            match dep {
                serde_json::Value::String(s) => depends_on.push(s.clone()),
                other => bail!("{}: unsupported non-string dependsOn entry {other}", ctx()),
            }
        }

        let mut inputs = None;
        if let Some(raw) = &self.inputs {
            let mut list = Vec::with_capacity(raw.len());
            for entry in raw {
                match entry {
                    serde_json::Value::String(s) => list.push(s.clone()),
                    other => bail!("{}: unsupported non-string input {other}", ctx()),
                }
            }
            inputs = Some(list);
        }

        let cwd = resolve_placeholders(
            opts.cwd.as_deref().unwrap_or("{workspaceRoot}"),
            project_root,
        );
        let cwd = cwd.trim_start_matches('/').to_string();

        Ok(ResolvedTarget {
            executor,
            commands,
            parallel_commands: opts.parallel.unwrap_or(true),
            cwd,
            cache: self.cache.unwrap_or(false),
            inputs,
            outputs: self
                .outputs
                .iter()
                .flatten()
                .map(|o| resolve_placeholders(o, project_root))
                .collect(),
            depends_on,
        })
    }
}

/// Replace `{workspaceRoot}` / `{projectRoot}` with workspace-root-relative
/// paths. `{workspaceRoot}` maps to the empty prefix since butler always works
/// in workspace-root-relative terms.
pub fn resolve_placeholders(pattern: &str, project_root: &str) -> String {
    let s = pattern
        .replace("{workspaceRoot}/", "")
        .replace("{workspaceRoot}", ".")
        .replace("{projectRoot}", project_root);
    if s.is_empty() { ".".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_commands(command: &str) -> TargetConfig {
        TargetConfig {
            options: RunOptions {
                command: Some(command.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn defaults_fill_absent_fields_only() {
        let defaults = TargetConfig {
            cache: Some(true),
            outputs: Some(vec!["{projectRoot}/dist".into()]),
            ..Default::default()
        };
        let mut project = run_commands("bun run build");
        project.outputs = Some(vec!["{projectRoot}/out".into()]);

        let merged = project.merged_over(Some(&defaults));
        assert_eq!(merged.cache, Some(true)); // filled from defaults
        assert_eq!(merged.outputs, Some(vec!["{projectRoot}/out".into()])); // project wins
    }

    #[test]
    fn configuration_overrides_command() {
        let mut t = run_commands("cargo build --package x");
        t.configurations.insert(
            "production".into(),
            RunOptions {
                command: Some("cargo build --package x --release".into()),
                ..Default::default()
            },
        );
        let r = t
            .resolve("x", "apps/x", "build", Some("production"))
            .unwrap();
        assert_eq!(r.commands, vec!["cargo build --package x --release"]);
        let r = t.resolve("x", "apps/x", "build", None).unwrap();
        assert_eq!(r.commands, vec!["cargo build --package x"]);
    }

    #[test]
    fn placeholders_resolve_workspace_relative() {
        assert_eq!(resolve_placeholders("{workspaceRoot}", "apps/x"), ".");
        assert_eq!(
            resolve_placeholders("{workspaceRoot}/apps/x", "apps/x"),
            "apps/x"
        );
        assert_eq!(
            resolve_placeholders("{projectRoot}/dist", "apps/todo/web"),
            "apps/todo/web/dist"
        );
    }

    #[test]
    fn unsupported_executor_is_a_hard_error() {
        let t = TargetConfig {
            executor: Some("@nx-tools/nx-container:build".into()),
            ..Default::default()
        };
        let err = t.resolve("zerg_api", "apps/zerg/api", "container", None);
        assert!(err.is_err());
        assert!(format!("{:#}", err.unwrap_err()).contains("not supported"));
    }

    #[test]
    fn multi_commands_and_parallel_flag() {
        let t = TargetConfig {
            options: RunOptions {
                commands: Some(vec![
                    serde_json::json!("cargo test --package domain_todo export_bindings"),
                    serde_json::json!({"command": "git diff --exit-code -- libs/domains/todo/types"}),
                ]),
                parallel: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let r = t
            .resolve("domain_todo", "libs/domains/todo", "ts-gate", None)
            .unwrap();
        assert_eq!(r.commands.len(), 2);
        assert!(!r.parallel_commands);
    }
}
