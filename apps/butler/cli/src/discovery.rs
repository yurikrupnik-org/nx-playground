//! Workspace discovery: locate the root, then build the project graph the way
//! nx 23 does — this repo's inference (the native Rust port of the nx.json
//! plugin, or an external command speaking nx's plugin JSON), then nx.json
//! `targetDefaults`, then nx's own `package.json` and `project.json` layers
//! (see [`infer::build`]).

use std::path::{Path, PathBuf};

use eyre::{Result, bail};

use crate::config::NxJson;
use crate::graph::ProjectGraph;
use crate::infer::{self, Ctx, Layer};
use crate::settings::{AppOverrides, Root};

/// Markers that define the workspace root, nearest-first from the cwd.
/// `butler.toml` comes first so the CLI works in a repo that has never seen nx;
/// `nx.json` keeps working for one that has not adopted the config file yet.
const ROOT_MARKERS: &[&str] = &[crate::settings::FILE, "nx.json"];

pub fn find_workspace_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if ROOT_MARKERS.iter().any(|m| dir.join(m).exists()) {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "no {} found in the current directory or any parent",
                ROOT_MARKERS.join(" or ")
            );
        }
    }
}

/// Where the repo-specific inference comes from.
pub enum Inference {
    /// The Rust port in [`infer::native`].
    Native,
    /// An external command speaking nx's plugin JSON ([`infer::external`]).
    External(Vec<String>),
}

pub fn discover(
    root: &Path,
    files: &[String],
    nx: &NxJson,
    settings: Option<&Root>,
    overrides: Option<&AppOverrides>,
    inference: &Inference,
) -> Result<ProjectGraph> {
    let ctx = Ctx {
        workspace_root: root,
        files,
        settings,
        overrides,
    };
    let external;
    let repo_layer: &dyn Layer = match inference {
        Inference::Native => &infer::native::NativePlugin,
        Inference::External(argv) => {
            external = infer::external::Command { argv: argv.clone() };
            &external
        }
    };
    infer::build(&ctx, &[repo_layer], &nx.target_defaults)
}
