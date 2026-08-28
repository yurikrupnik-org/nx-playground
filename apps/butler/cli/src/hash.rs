//! Task input hashing.
//!
//! A task hash covers: the resolved commands, the configuration name, the
//! project's own input files (per the target's `inputs` or the `default`
//! named input), and — for `^ref` inputs — the recursive input hashes of the
//! project's workspace dependencies. Content-addressed: same inputs, same
//! hash, on any machine.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use eyre::{bail, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};

use crate::config::resolve_placeholders;
use crate::graph::{Project, ProjectGraph, Task};

pub struct Hasher<'a> {
    root: &'a Path,
    graph: &'a ProjectGraph,
    named_inputs: &'a BTreeMap<String, Vec<String>>,
    /// Sorted universe of hashable files (workspace-relative).
    files: Vec<String>,
    file_hashes: HashMap<String, String>,
    /// Memo: (project, named-input) -> hash of own matching files.
    project_input_memo: HashMap<(String, String), String>,
    /// Memo: project -> recursive dependency input hash (`^default` closure).
    dep_input_memo: HashMap<String, String>,
}

/// Include/exclude globs compiled from a pattern list.
struct Matchers {
    include: GlobSet,
    exclude: GlobSet,
    has_include: bool,
}

impl Matchers {
    fn is_match(&self, path: &str) -> bool {
        self.has_include && self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

impl<'a> Hasher<'a> {
    pub fn new(
        root: &'a Path,
        graph: &'a ProjectGraph,
        named_inputs: &'a BTreeMap<String, Vec<String>>,
        mut files: Vec<String>,
    ) -> Self {
        files.sort();
        Self {
            root,
            graph,
            named_inputs,
            files,
            file_hashes: HashMap::new(),
            project_input_memo: HashMap::new(),
            dep_input_memo: HashMap::new(),
        }
    }

    pub fn task_hash(&mut self, task: &Task, configuration: Option<&str>) -> Result<String> {
        let project = self.graph.get(&task.id.project)?;
        let inputs = match &task.resolved.inputs {
            Some(explicit) => explicit.clone(),
            // nx default when a cached target declares no inputs.
            None => vec!["default".into(), "^default".into()],
        };
        let (patterns, dep_refs) = self.expand_input_refs(&inputs, &task.id.to_string())?;

        let mut hasher = Sha256::new();
        hasher.update(b"butler-task-v1\0");
        hasher.update(task.id.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(configuration.unwrap_or("").as_bytes());
        hasher.update([0]);
        for cmd in &task.resolved.commands {
            hasher.update(cmd.as_bytes());
            hasher.update([0]);
        }

        let own = self.files_hash(project, &patterns)?;
        hasher.update(own.as_bytes());

        for named in dep_refs {
            let deps: Vec<String> = project.deps.iter().cloned().collect();
            for dep in deps {
                let h = self.dep_input_hash(&dep, &named, &mut BTreeSet::new())?;
                hasher.update(h.as_bytes());
            }
        }
        Ok(hex(&hasher.finalize()))
    }

    /// Split a target input list into concrete glob patterns (named inputs
    /// recursively inlined) and `^named` dependency references.
    fn expand_input_refs(
        &self,
        inputs: &[String],
        ctx: &str,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let mut patterns = Vec::new();
        let mut dep_refs = Vec::new();
        let mut stack: Vec<String> = inputs.to_vec();
        let mut seen = BTreeSet::new();
        while let Some(entry) = stack.pop() {
            if let Some(named) = entry.strip_prefix('^') {
                dep_refs.push(named.to_string());
            } else if self.named_inputs.contains_key(&entry) {
                if seen.insert(entry.clone()) {
                    stack.extend(self.named_inputs[&entry].iter().cloned());
                }
            } else if entry.starts_with('{') || entry.starts_with("!{") || !entry.contains('{') {
                patterns.push(entry);
            } else {
                bail!("{ctx}: unsupported input pattern `{entry}`");
            }
        }
        patterns.sort();
        dep_refs.sort();
        dep_refs.dedup();
        Ok((patterns, dep_refs))
    }

    /// Hash of the project's own files matching `patterns`.
    fn files_hash(&mut self, project: &Project, patterns: &[String]) -> Result<String> {
        let key = (project.name.clone(), patterns.join("\n"));
        if let Some(h) = self.project_input_memo.get(&key) {
            return Ok(h.clone());
        }
        let matchers = compile_patterns(patterns, &project.root)?;
        let matched: Vec<String> = self
            .files
            .iter()
            .filter(|f| matchers.is_match(f))
            .cloned()
            .collect();

        let mut hasher = Sha256::new();
        hasher.update(b"butler-files-v1\0");
        for file in matched {
            let content_hash = self.file_hash(&file)?;
            hasher.update(file.as_bytes());
            hasher.update([0]);
            hasher.update(content_hash.as_bytes());
            hasher.update([0]);
        }
        let h = hex(&hasher.finalize());
        self.project_input_memo.insert(key, h.clone());
        Ok(h)
    }

    /// Recursive `^named` hash: dep's own named-input files plus its own deps'.
    fn dep_input_hash(
        &mut self,
        project_name: &str,
        named: &str,
        visiting: &mut BTreeSet<String>,
    ) -> Result<String> {
        let memo_key = format!("{project_name}\0{named}");
        if let Some(h) = self.dep_input_memo.get(&memo_key) {
            return Ok(h.clone());
        }
        if !visiting.insert(project_name.to_string()) {
            return Ok(String::new()); // dependency cycle: contribute nothing twice
        }
        let project = self.graph.get(project_name)?.clone();
        let (patterns, _) = self.expand_input_refs(&[named.to_string()], project_name)?;
        let own = self.files_hash(&project, &patterns)?;

        let mut hasher = Sha256::new();
        hasher.update(b"butler-dep-v1\0");
        hasher.update(own.as_bytes());
        for dep in &project.deps {
            let h = self.dep_input_hash(dep, named, visiting)?;
            hasher.update(h.as_bytes());
        }
        visiting.remove(project_name);
        let h = hex(&hasher.finalize());
        self.dep_input_memo.insert(memo_key, h.clone());
        Ok(h)
    }

    fn file_hash(&mut self, rel: &str) -> Result<String> {
        if let Some(h) = self.file_hashes.get(rel) {
            return Ok(h.clone());
        }
        let path = self.root.join(rel);
        let h = match std::fs::read(&path) {
            Ok(bytes) => {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                hex(&hasher.finalize())
            }
            // Deleted-but-listed files hash to a fixed marker.
            Err(_) => "missing".into(),
        };
        self.file_hashes.insert(rel.to_string(), h.clone());
        Ok(h)
    }
}

/// Compile patterns into include/exclude glob sets. `!`-prefixed patterns
/// exclude. Bare patterns (no placeholder) are project-root-relative.
fn compile_patterns(patterns: &[String], project_root: &str) -> Result<Matchers> {
    let mut include = GlobSetBuilder::new();
    let mut exclude = GlobSetBuilder::new();
    let mut has_include = false;
    for raw in patterns {
        let (negated, body) = match raw.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, raw.as_str()),
        };
        let resolved = if body.contains('{') {
            resolve_placeholders(body, project_root)
        } else {
            format!("{project_root}/{body}")
        };
        let glob = Glob::new(&resolved)
            .map_err(|e| eyre::eyre!("input pattern `{raw}` -> `{resolved}`: {e}"))?;
        if negated {
            exclude.add(glob);
        } else {
            include.add(glob);
            has_include = true;
        }
    }
    Ok(Matchers {
        include: include.build()?,
        exclude: exclude.build()?,
        has_include,
    })
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_project_scoped_files() {
        let m = compile_patterns(
            &[
                "{projectRoot}/**/*".into(),
                "!{projectRoot}/dist/**/*".into(),
            ],
            "apps/todo/web",
        )
        .unwrap();
        assert!(m.is_match("apps/todo/web/src/index.tsx"));
        assert!(!m.is_match("apps/todo/web/dist/index.html"));
        assert!(!m.is_match("apps/terran/web/src/index.tsx"));
    }

    #[test]
    fn workspace_root_patterns_match_global_files() {
        let m = compile_patterns(
            &["{workspaceRoot}/.github/workflows/ci.yml".into()],
            "apps/todo/web",
        )
        .unwrap();
        assert!(m.is_match(".github/workflows/ci.yml"));
        assert!(!m.is_match("apps/todo/web/src/index.tsx"));
    }

    #[test]
    fn bare_patterns_are_project_relative() {
        let m = compile_patterns(&["src/**/*.ts".into()], "libs/x").unwrap();
        assert!(m.is_match("libs/x/src/a.ts"));
        assert!(!m.is_match("libs/x/test/a.ts"));
        assert!(!m.is_match("libs/y/src/a.ts"));
    }

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(hex(&[0x00, 0xff, 0x0a]), "00ff0a");
    }
}
