//! `butler submodule` — manage `.gitmodules` and generate the manifests that
//! deploy each submodule.
//!
//! `.gitmodules` stays git's file. Every edit goes through `git submodule`
//! (`add`, `set-url`, `set-branch`, `deinit` + `rm`, `update`), so the index,
//! `.git/config` and `.git/modules` never disagree with it, and butler reads it
//! back through `git config -f .gitmodules` — git's own parser.
//!
//! Every change regenerates the manifests ([`manifests`]) when the workspace
//! has a butler.toml, so the committed Flux objects always name the commit the
//! index pins. The CLI and the web UI ([`serve`]) call the same functions.

pub mod manifests;
pub mod serve;

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::settings::{self, Root};

pub const GITMODULES: &str = ".gitmodules";

/// One `.gitmodules` entry, joined with what the index and the working tree
/// say about it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Submodule {
    /// The `[submodule "<name>"]` key; also the `[submodule.<name>]` key in
    /// butler.toml.
    pub name: String,
    /// Workspace-root-relative checkout directory.
    pub path: String,
    pub url: String,
    /// Branch `git submodule update --remote` follows; `None` = the remote's
    /// HEAD.
    pub branch: Option<String>,
    /// The commit this repo's index pins (the gitlink): what a commit of this
    /// repo records, and what Flux deploys under `track = "commit"`. `None`
    /// when `.gitmodules` names a path the index has no gitlink for.
    pub pinned: Option<String>,
    pub checkout: Checkout,
}

/// The working-tree side of a submodule, from `git submodule status`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "state", content = "commit")]
pub enum Checkout {
    /// Not initialised: the directory is empty.
    Uninitialized,
    /// Checked out at the pinned commit.
    Pinned,
    /// Checked out at this commit, which is not the pinned one.
    Moved(String),
    /// The gitlink has a merge conflict.
    Conflict,
}

/// The workspace's butler.toml, when it has one. Submodule management works
/// without it; manifest generation does not.
pub fn load_settings(root: &Path) -> Result<Option<Root>> {
    if root.join(settings::FILE).is_file() {
        Root::load(root).map(Some)
    } else {
        Ok(None)
    }
}

/// Every submodule `.gitmodules` declares, sorted by name.
pub fn load(root: &Path) -> Result<Vec<Submodule>> {
    if !root.join(GITMODULES).is_file() {
        return Ok(Vec::new());
    }
    let raw = git(root, &["config", "-f", GITMODULES, "-z", "--list"])?;
    let mut subs = parse_config_list(&raw)?;
    if subs.is_empty() {
        return Ok(subs);
    }

    let paths: Vec<&str> = subs.iter().map(|s| s.path.as_str()).collect();
    let mut args = vec!["ls-files", "--stage", "-z", "--"];
    args.extend(&paths);
    let pins = parse_gitlinks(&git(root, &args)?);
    let status = String::from_utf8_lossy(&git(root, &["submodule", "status"])?).into_owned();
    let checkouts = parse_status(&status, &paths);

    for sub in &mut subs {
        sub.pinned = pins.get(&sub.path).cloned();
        if let Some(checkout) = checkouts.get(&sub.path) {
            sub.checkout = checkout.clone();
        }
    }
    Ok(subs)
}

/// The submodule named `name`, or an error listing the known names.
pub fn find(root: &Path, name: &str) -> Result<Submodule> {
    let subs = load(root)?;
    let known: Vec<&str> = subs.iter().map(|s| s.name.as_str()).collect();
    let known = if known.is_empty() {
        "none".to_string()
    } else {
        known.join(", ")
    };
    subs.iter()
        .find(|s| s.name == name)
        .cloned()
        .ok_or_else(|| eyre!("{GITMODULES} has no submodule named `{name}` (known: {known})"))
}

// ---------------------------------------------------------------------------
// Changes

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddRequest {
    pub url: String,
    /// Checkout directory; default: git's (the URL's basename).
    pub path: Option<String>,
    /// `.gitmodules` name; default: git's (the path).
    pub name: Option<String>,
    pub branch: Option<String>,
}

/// `git submodule add`: clones, writes `.gitmodules`, stages both.
pub fn add(root: &Path, req: &AddRequest) -> Result<()> {
    let url = req.url.trim();
    if url.is_empty() {
        bail!("a submodule needs a url");
    }
    let mut args = vec!["submodule", "add"];
    if let Some(branch) = non_empty(req.branch.as_deref()) {
        args.extend(["-b", branch]);
    }
    if let Some(name) = non_empty(req.name.as_deref()) {
        args.extend(["--name", name]);
    }
    args.extend(["--", url]);
    if let Some(path) = non_empty(req.path.as_deref()) {
        args.push(path);
    }
    git(root, &args)?;
    Ok(())
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRequest {
    pub url: Option<String>,
    /// `Some("")` removes the branch, so `--remote` follows the remote's HEAD.
    pub branch: Option<String>,
}

/// Change a submodule's URL (`git submodule set-url`, which also syncs
/// `.git/config`) and/or its branch (`git submodule set-branch`), and stage
/// `.gitmodules` the way `git submodule add` does — `git rm` of a submodule
/// refuses while `.gitmodules` has unstaged changes.
pub fn set(root: &Path, name: &str, req: &SetRequest) -> Result<()> {
    if req.url.is_none() && req.branch.is_none() {
        bail!("nothing to change: give a url and/or a branch");
    }
    let sub = find(root, name)?;
    if let Some(url) = &req.url {
        let url = url.trim();
        if url.is_empty() {
            bail!("a submodule url cannot be empty");
        }
        git(root, &["submodule", "set-url", "--", &sub.path, url])?;
    }
    match req.branch.as_deref().map(str::trim) {
        None => {}
        Some("") => {
            git(
                root,
                &["submodule", "set-branch", "--default", "--", &sub.path],
            )?;
        }
        Some(branch) => {
            git(
                root,
                &[
                    "submodule",
                    "set-branch",
                    "--branch",
                    branch,
                    "--",
                    &sub.path,
                ],
            )?;
        }
    }
    git(root, &["add", "--", GITMODULES])?;
    Ok(())
}

/// `git submodule deinit` + `git rm`: drops the checkout, the gitlink and the
/// `.gitmodules` entry. The clone under `.git/modules/<name>` stays, as git
/// leaves it.
///
/// Everything that would stop it half-way is checked first: a butler.toml
/// table for the submodule (the next generation would fail on it), and
/// unstaged `.gitmodules` edits (which `git rm` refuses to touch, after
/// `deinit` has already run).
pub fn remove(root: &Path, settings: Option<&Root>, name: &str) -> Result<()> {
    if settings.is_some_and(|s| s.submodule.contains_key(name)) {
        bail!(
            "{file} still declares [submodule.{name}]; delete that table first",
            file = settings::FILE
        );
    }
    let sub = find(root, name)?;
    if !git(root, &["diff", "--name-only", "--", GITMODULES])?.is_empty() {
        bail!("{GITMODULES} has unstaged changes; stage or discard them first");
    }
    git(root, &["submodule", "deinit", "-f", "--", &sub.path])?;
    git(root, &["rm", "-f", "--", &sub.path])?;
    Ok(())
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRequest {
    /// Submodule names; empty = all.
    #[serde(default)]
    pub names: Vec<String>,
    /// Move to the tip of each submodule's branch and stage the new pins,
    /// instead of checking out the pinned commits.
    #[serde(default)]
    pub remote: bool,
}

/// `git submodule update --init [--remote]`. With `remote`, the moved
/// gitlinks are staged, so the regenerated manifests pin the new commits and
/// the next commit records them.
pub fn update(root: &Path, req: &UpdateRequest) -> Result<()> {
    let subs = load(root)?;
    let mut paths = Vec::new();
    for name in &req.names {
        let sub = subs
            .iter()
            .find(|s| s.name == *name)
            .ok_or_else(|| eyre!("{GITMODULES} has no submodule named `{name}`"))?;
        paths.push(sub.path.as_str());
    }
    if req.names.is_empty() {
        paths.extend(subs.iter().map(|s| s.path.as_str()));
    }
    if paths.is_empty() {
        bail!("{GITMODULES} declares no submodules");
    }

    let mut args = vec!["submodule", "update", "--init"];
    if req.remote {
        args.push("--remote");
    }
    args.push("--");
    args.extend(&paths);
    git(root, &args)?;

    if req.remote {
        let mut stage = vec!["add", "--"];
        stage.extend(&paths);
        git(root, &stage)?;
    }
    Ok(())
}

/// Regenerate after a change. The error says the change itself went through,
/// so nobody retries an `add` that already happened.
pub fn regenerate_after_change(
    root: &Path,
    settings: Option<&Root>,
) -> Result<Option<manifests::GenReport>> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    load(root)
        .and_then(|subs| manifests::plan(root, settings, &subs))
        .and_then(|plan| plan.apply(root))
        .map(Some)
        .wrap_err("the submodule change is applied, but regenerating its manifests failed")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Parsing git's output

/// `git config -z --list`: `key\nvalue\0` records. Keys are
/// `submodule.<name>.<var>`, and `<name>` may itself contain dots, so the
/// variable is what follows the *last* dot.
fn parse_config_list(raw: &[u8]) -> Result<Vec<Submodule>> {
    #[derive(Default)]
    struct Entry {
        path: Option<String>,
        url: Option<String>,
        branch: Option<String>,
    }
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let record = String::from_utf8_lossy(record);
        let (key, value) = record.split_once('\n').unwrap_or((&record, ""));
        let Some((name, var)) = key
            .strip_prefix("submodule.")
            .and_then(|rest| rest.rsplit_once('.'))
        else {
            continue;
        };
        let entry = entries.entry(name.to_string()).or_default();
        match var {
            "path" => entry.path = Some(value.to_string()),
            "url" => entry.url = Some(value.to_string()),
            "branch" => entry.branch = Some(value.to_string()),
            _ => {}
        }
    }
    entries
        .into_iter()
        .map(|(name, entry)| {
            let path = entry
                .path
                .ok_or_else(|| eyre!("{GITMODULES}: submodule `{name}` has no path"))?;
            let url = entry
                .url
                .ok_or_else(|| eyre!("{GITMODULES}: submodule `{name}` has no url"))?;
            Ok(Submodule {
                name,
                path,
                url,
                branch: entry.branch,
                pinned: None,
                checkout: Checkout::Uninitialized,
            })
        })
        .collect()
}

/// `git ls-files --stage -z`: `<mode> <sha> <stage>\t<path>\0`; gitlinks are
/// mode 160000.
fn parse_gitlinks(raw: &[u8]) -> BTreeMap<String, String> {
    raw.split(|b| *b == 0)
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let (meta, path) = record.split_once('\t')?;
            let mut fields = meta.split(' ');
            (fields.next()? == "160000")
                .then(|| Some((path.to_string(), fields.next()?.to_string())))
                .flatten()
        })
        .collect()
}

/// `git submodule status`: `<flag><sha> <path>[ (<describe>)]` per line. The
/// path is matched against the known paths rather than split on spaces, so a
/// path containing one still resolves.
fn parse_status(raw: &str, paths: &[&str]) -> BTreeMap<String, Checkout> {
    let mut out = BTreeMap::new();
    for line in raw.lines() {
        let mut chars = line.chars();
        let Some(flag) = chars.next() else { continue };
        let Some((sha, rest)) = chars.as_str().split_once(' ') else {
            continue;
        };
        let Some(path) = paths
            .iter()
            .filter(|p| rest == **p || rest.strip_prefix(**p).is_some_and(|t| t.starts_with(" (")))
            .max_by_key(|p| p.len())
        else {
            continue;
        };
        let checkout = match flag {
            '-' => Checkout::Uninitialized,
            '+' => Checkout::Moved(sha.to_string()),
            'U' => Checkout::Conflict,
            _ => Checkout::Pinned,
        };
        out.insert((*path).to_string(), checkout);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_submodule_name_keeps_its_dots() {
        let raw = b"submodule.libs.v1.path\nlibs/v1\0submodule.libs.v1.url\nhttps://x/libs\0\
                    submodule.libs.v1.branch\nmain\0";
        let subs = parse_config_list(raw).expect("parses");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "libs.v1");
        assert_eq!(subs[0].path, "libs/v1");
        assert_eq!(subs[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn an_entry_without_a_url_is_an_error_not_a_skip() {
        let err = parse_config_list(b"submodule.a.path\na\0").expect_err("url is required");
        assert!(err.to_string().contains("no url"), "{err}");
    }

    #[test]
    fn only_gitlinks_are_pins() {
        let raw = b"100644 aaaa 0\tREADME.md\x00160000 bbbb 0\tvendor/lib\x00";
        let pins = parse_gitlinks(raw);
        assert_eq!(pins.len(), 1);
        assert_eq!(pins["vendor/lib"], "bbbb");
    }

    #[test]
    fn status_flags_map_to_checkout_states() {
        let raw = "-aaaa vendor/a\n+bbbb vendor/b (v1.0-3-gbbbb)\n cccc vendor/b c (heads/main)\n";
        let got = parse_status(raw, &["vendor/a", "vendor/b", "vendor/b c"]);
        assert_eq!(got["vendor/a"], Checkout::Uninitialized);
        assert_eq!(got["vendor/b"], Checkout::Moved("bbbb".into()));
        // The longer path wins, so a path with a space is not mistaken for a
        // shorter sibling followed by its describe suffix.
        assert_eq!(got["vendor/b c"], Checkout::Pinned);
    }
}
