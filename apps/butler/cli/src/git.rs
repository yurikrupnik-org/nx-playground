//! Git integration: tracked file listing (for hashing) and change detection
//! (for `affected`). Shells out to `git`, like nx does.

use std::path::Path;
use std::process::Command;

use eyre::{bail, Result, WrapErr};

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

/// Files changed relative to `merge-base(HEAD, base)`, including uncommitted
/// tracked changes and untracked files.
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<String>> {
    let mb_raw = git(root, &["merge-base", "HEAD", base])?;
    let merge_base = String::from_utf8_lossy(&mb_raw).trim().to_string();

    let mut files = lines_z(&git(root, &["diff", "--name-only", "-z", &merge_base])?);
    files.extend(lines_z(&git(
        root,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )?));
    files.sort();
    files.dedup();
    Ok(files)
}
