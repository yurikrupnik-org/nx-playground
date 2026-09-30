//! `nx.json` `targetDefaults`, applied as nx 23 applies them
//! (`createTargetDefaultsResults`): not beneath the finished target, but as a
//! synthetic layer merged after the nx.json plugins and before nx's own
//! `package.json`/`project.json` layers. That placement is the whole
//! semantics — a default overrides what a plugin inferred (the repo's rust
//! `build` gets `inputs`/`outputs` from nx.json even though the plugin sets
//! its own), while a value spelled in a manifest overrides the default.
//!
//! One synthetic target is made per (root, target, matching entry). It is
//! stamped with the executor/command the target will end up with, so the
//! merge on either side of it cannot treat it as a rival target and replace
//! it (or be replaced by it).

use std::collections::BTreeMap;

use eyre::{Result, bail};

use super::matching::{Candidate, find_matching_projects};
use super::{Builder, Contribution, Draft, Node, glob, is_glob_pattern, to_project_name};
use crate::config::{JsonMap, TargetConfig, TargetDefault, TargetDefaultEntry, compatible};

/// The shape the real merge will land on for one target.
struct Effective {
    executor: Option<String>,
    command: Option<String>,
    /// A run-commands target's `options.command`/`options.commands`.
    options: Option<JsonMap>,
    /// The manifest layer's target is incompatible with the plugin's and will
    /// replace it.
    replaces_specified: bool,
}

/// nx's `runCommandsCommandIdentity`.
fn command_identity(t: &TargetConfig) -> Option<JsonMap> {
    if t.executor.as_deref() != Some("nx:run-commands") {
        return None;
    }
    let opts = t.options.as_ref()?;
    let identity: JsonMap = ["command", "commands"]
        .into_iter()
        .filter_map(|k| opts.get(k).map(|v| (k.to_string(), v.clone())))
        .collect();
    (!identity.is_empty()).then_some(identity)
}

fn winner(t: &TargetConfig, replaces_specified: bool) -> Effective {
    Effective {
        executor: t.executor.clone(),
        command: t.command.clone(),
        options: command_identity(t),
        replaces_specified,
    }
}

/// nx's `effectiveTargetForLookup`.
fn effective(
    specified: Option<&TargetConfig>,
    default: Option<&TargetConfig>,
) -> Option<Effective> {
    match (specified, default) {
        (Some(s), Some(d)) if !compatible(s, d) => Some(winner(d, true)),
        (Some(s), Some(d)) => Some(Effective {
            executor: d.executor.clone().or_else(|| s.executor.clone()),
            command: d.command.clone().or_else(|| s.command.clone()),
            options: command_identity(d).or_else(|| command_identity(s)),
            replaces_specified: false,
        }),
        (Some(t), None) | (None, Some(t)) => Some(winner(t, false)),
        (None, None) => None,
    }
}

/// A resolving `targetDefaults` key and its matching entries, by index.
type Matches<'a> = (&'a str, Vec<(usize, &'a TargetDefaultEntry)>);

/// The `targetDefaults` key that applies to `target_name` and its matching
/// entries (with their indices): the executor key first, then the exact
/// target name, then glob keys longest first — the first key with any
/// matching entry wins (nx's `resolveTargetDefaultMatches`).
fn matches<'a>(
    defaults: &'a BTreeMap<String, TargetDefault>,
    target_name: &str,
    executor: Option<&str>,
    project: &Candidate,
) -> Result<Option<Matches<'a>>> {
    let mut keys: Vec<&str> = Vec::new();
    if let Some(e) = executor.filter(|e| defaults.contains_key(*e)) {
        keys.push(e);
    }
    if defaults.contains_key(target_name) && Some(target_name) != executor {
        keys.push(target_name);
    }
    let mut globs: Vec<&str> = Vec::new();
    for key in defaults.keys() {
        if key != target_name
            && Some(key.as_str()) != executor
            && is_glob_pattern(key)
            && glob(key)?.is_match(target_name)
        {
            globs.push(key);
        }
    }
    // Longest (most specific) first. nx breaks ties by nx.json key order,
    // which butler does not keep; ties fall back to key order.
    globs.sort_by_key(|k| std::cmp::Reverse(k.len()));
    keys.extend(globs);

    for key in keys {
        let (key, value) = defaults.get_key_value(key).expect("listed above");
        let mut hits = Vec::new();
        for (index, entry) in value.entries().iter().enumerate() {
            if filter_matches(entry, key, executor, project)? {
                hits.push((index, entry));
            }
        }
        if !hits.is_empty() {
            return Ok(Some((key.as_str(), hits)));
        }
    }
    Ok(None)
}

/// nx's `entryFilterMatches`.
fn filter_matches(
    entry: &TargetDefaultEntry,
    key: &str,
    executor: Option<&str>,
    project: &Candidate,
) -> Result<bool> {
    let Some(filter) = &entry.filter else {
        return Ok(true);
    };
    if filter.plugin.is_some() {
        // Which nx plugin originated a target is known only to nx (the same
        // target may be touched by `@monodon/rust` and the repo plugin).
        bail!(
            "nx.json targetDefaults.{key}: `filter.plugin` is not supported by butler; \
             use `filter.executor` or `filter.projects`"
        );
    }
    if let Some(projects) = &filter.projects {
        let Some(list) = projects.as_array() else {
            bail!("nx.json targetDefaults.{key}: `filter.projects` must be a list of patterns");
        };
        let patterns: Vec<String> = list
            .iter()
            .map(|p| p.as_str().map(str::to_string))
            .collect::<Option<_>>()
            .ok_or_else(|| {
                eyre::eyre!(
                    "nx.json targetDefaults.{key}: `filter.projects` entries must be strings"
                )
            })?;
        let one = [Candidate {
            name: project.name,
            root: project.root,
            tags: project.tags,
        }];
        if find_matching_projects(&patterns, &one)?.is_empty() {
            return Ok(false);
        }
    }
    if let Some(e) = &filter.executor
        && Some(e.as_str()) != executor
    {
        return Ok(false);
    }
    Ok(true)
}

/// nx's `authorsCommandIdentity`.
fn authors_command(t: &TargetConfig) -> bool {
    t.options
        .as_ref()
        .is_some_and(|o| o.contains_key("command") || o.contains_key("commands"))
}

/// The synthetic layer for `specified` (the plugins' merged projects) given
/// `staged` (nx's own layers merged alone).
pub fn synthesize(
    specified: &Builder,
    staged: &Builder,
    defaults: &BTreeMap<String, TargetDefault>,
) -> Result<Vec<Node>> {
    let empty = Draft::default();
    let roots: std::collections::BTreeSet<&String> = specified
        .drafts
        .keys()
        .chain(staged.drafts.keys())
        .collect();
    // (key, entry index) -> root -> contribution, so entries merge in document
    // order within their key.
    let mut out: BTreeMap<(String, usize), BTreeMap<String, Contribution>> = BTreeMap::new();
    for root in roots {
        let spec = specified.drafts.get(root).unwrap_or(&empty);
        let def = staged.drafts.get(root).unwrap_or(&empty);
        let name = def
            .name
            .clone()
            .or_else(|| spec.name.clone())
            .unwrap_or_else(|| to_project_name(&format!("{root}/project.json")));
        let mut tags = spec.tags.clone();
        tags.extend(def.tags.iter().filter(|t| !spec.tags.contains(t)).cloned());
        let project = Candidate {
            name: &name,
            root,
            tags: &tags,
        };
        let target_names: std::collections::BTreeSet<&String> =
            spec.targets.keys().chain(def.targets.keys()).collect();
        for target_name in target_names {
            let Some(eff) = effective(spec.targets.get(target_name), def.targets.get(target_name))
            else {
                continue;
            };
            let Some((key, hits)) =
                matches(defaults, target_name, eff.executor.as_deref(), &project)?
            else {
                continue;
            };
            let shape = TargetConfig {
                executor: eff.executor.clone(),
                command: eff.command.clone(),
                ..Default::default()
            };
            for (index, entry) in hits {
                let authored_executor = entry.target.executor.is_some();
                let mut synthetic = entry
                    .target
                    .clone()
                    .desugar(&format!("nx.json targetDefaults.{key}"))?;
                // An entry that sets a foreign executor would replace the
                // target outright; nx drops just that entry.
                if !compatible(&shape, &synthetic) {
                    continue;
                }
                let overwrites_authored = eff.options.is_some() && authors_command(&synthetic);
                if eff.replaces_specified || !overwrites_authored {
                    if eff.executor.is_some() {
                        synthetic.executor.clone_from(&eff.executor);
                    }
                    if eff.command.is_some() {
                        synthetic.command.clone_from(&eff.command);
                    }
                    if let Some(identity) = &eff.options {
                        let opts = synthetic.options.get_or_insert_with(JsonMap::new);
                        opts.remove("command");
                        opts.remove("commands");
                        opts.extend(identity.iter().map(|(k, v)| (k.clone(), v.clone())));
                    }
                } else if !authored_executor {
                    // Unstamped, the executor desugaring gave a `command`
                    // entry would read as a rival run-commands target.
                    synthetic.executor = None;
                }
                out.entry((key.to_string(), index))
                    .or_default()
                    .entry(root.clone())
                    .or_default()
                    .targets
                    .insert(target_name.clone(), synthetic);
            }
        }
    }
    Ok(out
        .into_iter()
        .map(|((key, index), projects)| Node {
            file: match defaults.get(&key) {
                Some(TargetDefault::Entries(_)) => format!("nx.json#targetDefaults.{key}[{index}]"),
                _ => format!("nx.json#targetDefaults.{key}"),
            },
            projects,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Json;

    fn layer(root: &str, targets: Json) -> Vec<Node> {
        vec![Node {
            file: format!("{root}/x"),
            projects: BTreeMap::from([(
                root.to_string(),
                Contribution {
                    name: Some(root.replace('/', "-")),
                    targets: serde_json::from_value(targets).unwrap(),
                    ..Default::default()
                },
            )]),
        }]
    }

    fn defaults(v: Json) -> BTreeMap<String, TargetDefault> {
        serde_json::from_value(v).unwrap()
    }

    fn run(plugin: Json, manifest: Json, td: Json) -> TargetConfig {
        let td = defaults(td);
        let mut b = Builder::default();
        b.apply("plugin", layer("libs/a", plugin)).unwrap();
        let mut staged = Builder::default();
        staged
            .apply("manifest", layer("libs/a", manifest.clone()))
            .unwrap();
        let synthetic = synthesize(&b, &staged, &td).unwrap();
        b.apply("td", synthetic).unwrap();
        b.apply("manifest", layer("libs/a", manifest)).unwrap();
        b.drafts["libs/a"].targets["build"].clone()
    }

    #[test]
    fn a_default_overrides_the_plugin_but_not_the_manifest() {
        let t = run(
            json!({"build": {"executor": "nx:run-commands", "cache": false,
                "inputs": ["plugin"], "outputs": ["plugin"], "options": {"command": "cargo check"}}}),
            json!({"build": {"outputs": ["manifest"]}}),
            json!({"build": {"cache": true, "inputs": ["production"], "outputs": ["{projectRoot}/dist"]}}),
        );
        assert_eq!(t.cache, Some(true));
        assert_eq!(t.inputs, Some(vec![json!("production")]));
        assert_eq!(t.outputs, Some(vec!["manifest".to_string()]));
        assert_eq!(t.options.unwrap()["command"], json!("cargo check"));
    }

    #[test]
    fn a_default_survives_a_manifest_that_replaces_the_plugin_target() {
        // The manifest's run-script replaces the plugin's run-commands; the
        // default is stamped with the manifest's shape, so it still applies.
        let t = run(
            json!({"build": {"executor": "nx:run-commands", "options": {"command": "x"}}}),
            json!({"build": {"executor": "nx:run-script", "options": {"script": "build"}}}),
            json!({"build": {"cache": true}}),
        );
        assert_eq!(t.executor.as_deref(), Some("nx:run-script"));
        assert_eq!(t.cache, Some(true));
        assert_eq!(Json::Object(t.options.unwrap()), json!({"script": "build"}));
    }

    #[test]
    fn executor_key_beats_name_key_and_filters_narrow_entries() {
        let t = run(
            json!({"build": {"executor": "nx:run-commands", "options": {"command": "x"}}}),
            json!({}),
            json!({
                "build": {"cache": true},
                "nx:run-commands": [
                    {"outputs": ["all"]},
                    {"filter": {"executor": "other"}, "outputs": ["never"]},
                    {"filter": {"projects": ["libs-*"]}, "inputs": ["matched"]}
                ]
            }),
        );
        assert_eq!(t.cache, None);
        assert_eq!(t.outputs, Some(vec!["all".to_string()]));
        assert_eq!(t.inputs, Some(vec![json!("matched")]));
    }

    #[test]
    fn an_entry_with_a_foreign_executor_is_dropped() {
        let t = run(
            json!({"build": {"executor": "nx:run-commands", "options": {"command": "x"}}}),
            json!({}),
            json!({"build*": {"executor": "@nx/vite:build", "cache": true}}),
        );
        assert_eq!(t.executor.as_deref(), Some("nx:run-commands"));
        assert_eq!(t.cache, None);
    }
}
