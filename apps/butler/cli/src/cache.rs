//! Local content-addressed task cache under `dist/butler/cache/<task-hash>/`.
//!
//! An entry stores the recorded terminal output plus copies of the declared
//! `outputs` paths. Only successful runs are cached; a hit replays the output
//! and restores the artifacts.

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

pub struct Cache {
    dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct CacheMeta {
    pub task: String,
    pub hash: String,
    pub duration_ms: u128,
    /// Workspace-relative paths of stored outputs.
    pub outputs: Vec<String>,
}

impl Cache {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            dir: workspace_root.join("dist/butler/cache"),
        }
    }

    fn entry(&self, hash: &str) -> PathBuf {
        self.dir.join(hash)
    }

    pub fn lookup(&self, hash: &str) -> Option<CacheMeta> {
        let raw = std::fs::read_to_string(self.entry(hash).join("meta.json")).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn stdout(&self, hash: &str) -> String {
        std::fs::read_to_string(self.entry(hash).join("stdout.log")).unwrap_or_default()
    }

    /// Restore stored outputs into the workspace.
    pub fn restore(&self, root: &Path, meta: &CacheMeta) -> Result<()> {
        let stored = self.entry(&meta.hash).join("outputs");
        for rel in &meta.outputs {
            let src = stored.join(rel);
            let dst = root.join(rel);
            if src.exists() {
                if dst.exists() {
                    remove_path(&dst)?;
                }
                copy_recursive(&src, &dst).wrap_err_with(|| format!("restoring output {rel}"))?;
            }
        }
        Ok(())
    }

    /// Record a successful run: stdout plus every existing declared output.
    /// Output specs may be concrete paths or simple globs (e.g. `dir/*.tar`).
    pub fn store(
        &self,
        root: &Path,
        task: &str,
        hash: &str,
        stdout: &str,
        duration_ms: u128,
        output_specs: &[String],
    ) -> Result<()> {
        let entry = self.entry(hash);
        let tmp = self.dir.join(format!(".tmp-{hash}"));
        if tmp.exists() {
            remove_path(&tmp)?;
        }
        std::fs::create_dir_all(&tmp)?;
        std::fs::write(tmp.join("stdout.log"), stdout)?;

        let mut stored = Vec::new();
        for spec in output_specs {
            for rel in expand_output_spec(root, spec)? {
                let src = root.join(&rel);
                if !src.exists() {
                    continue;
                }
                let dst = tmp.join("outputs").join(&rel);
                copy_recursive(&src, &dst).wrap_err_with(|| format!("caching output {rel}"))?;
                stored.push(rel);
            }
        }

        let meta = CacheMeta {
            task: task.to_string(),
            hash: hash.to_string(),
            duration_ms,
            outputs: stored,
        };
        std::fs::write(tmp.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        if entry.exists() {
            remove_path(&entry)?;
        }
        std::fs::rename(&tmp, &entry)?;
        Ok(())
    }
}

/// Expand an output spec into concrete workspace-relative paths. Concrete
/// paths pass through; a glob component expands against the filesystem.
fn expand_output_spec(root: &Path, spec: &str) -> Result<Vec<String>> {
    if !spec.contains('*') {
        return Ok(vec![spec.to_string()]);
    }
    let glob = globset::Glob::new(spec)
        .map_err(|e| eyre::eyre!("output glob `{spec}`: {e}"))?
        .compile_matcher();
    // Walk only the non-glob parent prefix.
    let prefix: PathBuf = Path::new(spec)
        .components()
        .take_while(|c| !c.as_os_str().to_string_lossy().contains('*'))
        .collect();
    let base = root.join(&prefix);
    let mut out = Vec::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if path.is_dir() {
                stack.push(path);
            } else if glob.is_match(&rel) {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}
