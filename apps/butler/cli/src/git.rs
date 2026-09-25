//! Git integration: the workspace file listing (for hashing). Shells out to
//! `git`, like nx does; change detection for `affected` is in
//! [`crate::affected`].

use std::path::Path;
use std::process::Command;

use eyre::{Result, WrapErr, bail};

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .wrap_err_with(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

fn lines_z(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Tracked + untracked-but-not-ignored files, workspace-root relative.
/// This is the file universe used for input hashing.
pub fn ls_files(root: &Path) -> Result<Vec<String>> {
    let raw = git(root, &["ls-files", "-z", "-co", "--exclude-standard"])?;
    Ok(lines_z(&raw))
}
