//! Nx-compatible configuration surface: `nx.json`, `project.json`, and the
//! target shape every inference layer produces.
//!
//! A [`TargetConfig`] is stored in nx's own JSON shape (options and
//! configurations stay untyped maps), because two consumers need two different
//! things from it: `butler graph` must reproduce exactly what nx would put in
//! its graph (so `butler graph verify` can diff the two), while the runner
//! interprets it per executor ([`crate::executor`], via [`crate::task`]);
//! anything outside what butler ports fails loudly instead of silently
//! producing a wrong plan.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};

pub type Json = serde_json::Value;
pub type JsonMap = serde_json::Map<String, Json>;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NxJson {
    pub target_defaults: BTreeMap<String, TargetDefault>,
    pub named_inputs: BTreeMap<String, Vec<Json>>,
}

/// One `nx.json` `targetDefaults` value. The key is a target name, an
/// executor, or a glob over target names; the value is either a bare target
/// (nx's classic shape) or, since nx 23, a list of entries each narrowed by an
/// optional `filter`. nx normalizes the bare form to a one-entry list, filter
/// included, and so does [`TargetDefault::entries`].
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum TargetDefault {
    Entries(Vec<TargetDefaultEntry>),
    One(Box<TargetDefaultEntry>),
}

impl TargetDefault {
    pub fn entries(&self) -> &[TargetDefaultEntry] {
        match self {
            Self::Entries(list) => list,
            Self::One(entry) => std::slice::from_ref(entry.as_ref()),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TargetDefaultEntry {
    #[serde(default)]
    pub filter: Option<TargetDefaultFilter>,
    #[serde(flatten)]
    pub target: TargetConfig,
}

/// Every criterion present must hold for the entry to apply.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetDefaultFilter {
    /// A `findMatchingProjects` pattern list (names, globs, `tag:`, dirs).
    pub projects: Option<Json>,
    /// The nx.json plugin that originated the target.
    pub plugin: Option<String>,
    pub executor: Option<String>,
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
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectJson {
    pub name: Option<String>,
    /// Overrides the directory holding the file, as nx allows.
    pub root: Option<String>,
    pub project_type: Option<String>,
    pub tags: Vec<String>,
    pub implicit_dependencies: Vec<String>,
    pub targets: BTreeMap<String, TargetConfig>,
}

/// One target, in nx's JSON shape. `None`/absent is meaningful: nx merges
/// layers key by key, and "not set here" is what lets a lower layer show
/// through.
#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TargetConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    /// The `"command": "..."` shorthand. [`TargetConfig::desugar`] turns it into
    /// `nx:run-commands` + `options.command`, as nx does before merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<JsonMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configurations: Option<BTreeMap<String, JsonMap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_configuration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuous: Option<bool>,
}

impl TargetConfig {
    /// Prepare one contribution for merging, as nx does before any merge:
    /// resolve the `command` shorthand (nx's `resolveCommandSyntacticSugar`;
    /// an empty `command` is no shorthand, as in nx), and refuse nx's object
    /// `'...'` spread (`{"...": true, ...}`), whose meaning depends on key order
    /// that butler's JSON maps do not keep. Array spreads are supported, see
    /// [`TargetConfig::merged_over`].
    pub fn desugar(mut self, ctx: &str) -> Result<Self> {
        let object_spread = |m: &JsonMap| m.get(SPREAD) == Some(&Json::Bool(true));
        let has_spread = |m: &JsonMap| {
            object_spread(m) || m.values().any(|v| v.as_object().is_some_and(object_spread))
        };
        if self.options.as_ref().is_some_and(has_spread)
            || self
                .configurations
                .iter()
                .flatten()
                .any(|(_, c)| has_spread(c))
        {
            bail!(
                "{ctx}: the object form of nx's '...' spread is not supported by butler \
                 (its meaning depends on key order); spell the merged value out"
            );
        }
        if self.command.as_deref().is_none_or(str::is_empty) {
            return Ok(self);
        }
        let command = self.command.take().expect("checked above");
        if self.executor.is_some() {
            bail!("{ctx}: a target must not set both `executor` and `command`");
        }
        self.executor = Some("nx:run-commands".into());
        self.options
            .get_or_insert_with(JsonMap::new)
            .insert("command".into(), Json::String(command));
        Ok(self)
    }

    /// nx's layer merge (`mergeTargetConfigurations`): `self` is the newer
    /// layer, `base` what earlier layers produced. When the two are
    /// incompatible (different executors, or different run-commands commands
    /// or run-script scripts) the newer layer replaces the older outright;
    /// otherwise top-level keys, options and each named configuration merge
    /// key by key, newer winning. An array value replaces the older one unless
    /// it holds the `'...'` token, which splices the older array in at that
    /// position (nothing, when there is no older value).
    pub fn merged_over(&self, base: Option<&TargetConfig>) -> TargetConfig {
        let Some(base) = base else {
            return self.merged_over(Some(&TargetConfig::default()));
        };
        if !compatible(base, self) {
            return self.merged_over(None);
        }
        let spread = |v: &Json| v.as_str() == Some(SPREAD);
        TargetConfig {
            executor: self.executor.clone().or_else(|| base.executor.clone()),
            command: self.command.clone().or_else(|| base.command.clone()),
            cache: self.cache.or(base.cache),
            inputs: merge_list(self.inputs.as_ref(), base.inputs.as_ref(), spread),
            outputs: merge_list(self.outputs.as_ref(), base.outputs.as_ref(), |s| {
                s == SPREAD
            }),
            depends_on: merge_list(self.depends_on.as_ref(), base.depends_on.as_ref(), spread),
            options: merge_maps(self.options.as_ref(), base.options.as_ref()),
            configurations: match (&self.configurations, &base.configurations) {
                (None, None) => None,
                (new, old) => {
                    let mut out = old.clone().unwrap_or_default();
                    for (name, cfg) in new.iter().flatten() {
                        let merged = merge_maps(Some(cfg), out.get(name)).unwrap_or_default();
                        out.insert(name.clone(), merged);
                    }
                    Some(out)
                }
            },
            default_configuration: self
                .default_configuration
                .clone()
                .or_else(|| base.default_configuration.clone()),
            metadata: self.metadata.clone().or_else(|| base.metadata.clone()),
            parallelism: self.parallelism.or(base.parallelism),
            continuous: self.continuous.or(base.continuous),
        }
    }

    fn option(&self, key: &str) -> Option<&Json> {
        self.options.as_ref().and_then(|o| o.get(key))
    }
}

/// nx's `isCompatibleTarget`. An empty executor, command or script counts as
/// unset, as JavaScript truthiness makes it in nx.
pub(crate) fn compatible(a: &TargetConfig, b: &TargetConfig) -> bool {
    let executor = |t: &TargetConfig| t.executor.clone().filter(|e| !e.is_empty());
    let (Some(ea), Some(eb)) = (executor(a), executor(b)) else {
        return true;
    };
    if ea != eb {
        return false;
    }
    let identity: fn(&TargetConfig) -> Option<Json> = match ea.as_str() {
        "nx:run-commands" => run_commands_identity,
        "nx:run-script" => |t| t.option("script").cloned(),
        _ => return true,
    };
    match (identity(a).filter(truthy), identity(b).filter(truthy)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// `options.command ?? options.commands.join(' && ')`, nx's identity of a
/// run-commands target, rendered the way JavaScript's `join` renders.
fn run_commands_identity(t: &TargetConfig) -> Option<Json> {
    if let Some(c) = t.option("command").filter(|c| !c.is_null()) {
        return Some(c.clone());
    }
    let list = t.option("commands")?.as_array()?;
    Some(Json::String(
        list.iter()
            .map(js_join_item)
            .collect::<Vec<_>>()
            .join(" && "),
    ))
}

fn js_join_item(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        Json::Object(_) => "[object Object]".into(),
        Json::Array(a) => a.iter().map(js_join_item).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

/// JavaScript truthiness of a JSON value — what nx's `!x` tests.
pub(crate) fn truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Json::String(s) => !s.is_empty(),
        Json::Array(_) | Json::Object(_) => true,
    }
}

/// nx's spread token.
const SPREAD: &str = "...";

/// nx's `getMergeValueResult` for a list: absent keeps `base`, otherwise the
/// new list wins with every `'...'` element replaced by `base`'s elements.
fn merge_list<T: Clone>(
    new: Option<&Vec<T>>,
    base: Option<&Vec<T>>,
    is_spread: impl Fn(&T) -> bool,
) -> Option<Vec<T>> {
    let Some(new) = new else {
        return base.cloned();
    };
    let mut out = Vec::with_capacity(new.len());
    for item in new {
        if is_spread(item) {
            out.extend(base.into_iter().flatten().cloned());
        } else {
            out.push(item.clone());
        }
    }
    Some(out)
}

/// Options (or one configuration) merged key by key, newer winning; an array
/// value may splice the older one in with `'...'`.
fn merge_maps(new: Option<&JsonMap>, base: Option<&JsonMap>) -> Option<JsonMap> {
    match (new, base) {
        (None, None) => None,
        (new, base) => {
            let mut out = base.cloned().unwrap_or_default();
            for (k, v) in new.into_iter().flatten() {
                let merged = match v {
                    Json::Array(items) => Json::Array(
                        merge_list(Some(items), out.get(k).and_then(Json::as_array), |x| {
                            x.as_str() == Some(SPREAD)
                        })
                        .unwrap_or_default(),
                    ),
                    other => other.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target(v: Json) -> TargetConfig {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn compatible_layers_merge_key_by_key_newer_winning() {
        let base = target(json!({
            "executor": "nx:run-commands",
            "cache": true,
            "outputs": ["{projectRoot}/dist"],
            "options": {"command": "cargo build", "cwd": "", "parallel": false},
            "configurations": {"ci": {"a": 1}}
        }));
        let new = target(json!({
            "outputs": ["{projectRoot}/out"],
            "options": {"cwd": "apps/x"},
            "configurations": {"ci": {"b": 2}, "prod": {"c": 3}}
        }));
        let merged = new.merged_over(Some(&base));
        assert_eq!(merged.cache, Some(true));
        assert_eq!(merged.outputs, Some(vec!["{projectRoot}/out".into()]));
        let opts = merged.options.unwrap();
        assert_eq!(opts["command"], json!("cargo build"));
        assert_eq!(opts["cwd"], json!("apps/x"));
        let cfgs = merged.configurations.unwrap();
        assert_eq!(Json::Object(cfgs["ci"].clone()), json!({"a": 1, "b": 2}));
        assert_eq!(Json::Object(cfgs["prod"].clone()), json!({"c": 3}));
    }

    #[test]
    fn a_different_command_replaces_the_older_layer_outright() {
        let base = target(json!({
            "executor": "nx:run-commands", "cache": true,
            "options": {"command": "cargo build", "env": {"A": "1"}}
        }));
        let new = target(json!({
            "executor": "nx:run-commands",
            "options": {"command": "cargo check"}
        }));
        let merged = new.merged_over(Some(&base));
        assert_eq!(merged.cache, None);
        assert_eq!(
            Json::Object(merged.options.unwrap()),
            json!({"command": "cargo check"})
        );
    }

    #[test]
    fn a_different_executor_replaces_the_older_layer_outright() {
        let base = target(json!({"executor": "@monodon/rust:build", "options": {"release": true}}));
        let new = target(json!({"executor": "nx:run-commands", "options": {"command": "x"}}));
        assert_eq!(new.merged_over(Some(&base)), new);
    }

    #[test]
    fn command_shorthand_desugars_to_run_commands() {
        let t = target(json!({"command": "echo hi"}))
            .desugar("p:t")
            .unwrap();
        assert_eq!(t.executor.as_deref(), Some("nx:run-commands"));
        assert_eq!(t.options.unwrap()["command"], json!("echo hi"));
        assert!(
            target(json!({"command": "x", "executor": "nx:run-script"}))
                .desugar("p:t")
                .is_err()
        );
    }
}
