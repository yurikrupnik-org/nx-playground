//! `taskgraph shim [ARGS…]`: a drop-in for `task`, so every `task X` a CI job
//! (or a shell) makes is observed without editing the call sites.
//!
//! One or more plain task names — optionally followed by `-- args`, which go
//! to the name when there is exactly one — run through [`run`] in order and
//! stop at the first non-zero exit, which is returned. Anything else (a flag,
//! a `VAR=value`, no arguments, several names with `--`, a name the parsed
//! graph cannot account for, no Taskfile at all) is handed to the real go-task
//! unchanged, by `exec` on Unix, so its behaviour and exit code are exactly
//! go-task's.
//!
//! The real go-task is `$TASKGRAPH_TASK_BIN` (default `task`). When the shim
//! itself is installed as `task` on `PATH`, `TASKGRAPH_TASK_BIN` MUST point at
//! the real binary, or `task` resolves back to the shim. As a backstop every
//! go-task the shim starts gets `TASKGRAPH_SHIM_DEPTH` + 1; a legitimately
//! nested `task` inside a task increments it too, so only a chain deeper than
//! any real Taskfile nests ([`MAX_DEPTH`]) is refused as a loop.

use std::process::{Command, ExitCode};

use domain_taskgraph::GraphIndex;
use eyre::{Result, WrapErr, bail};
use tracing::debug;

use crate::context::Ctx;
use crate::run::{self, RunOptions};

/// Nesting depth of shim-started go-task processes.
pub const DEPTH_ENV: &str = "TASKGRAPH_SHIM_DEPTH";
/// Deeper than this is a `TASKGRAPH_TASK_BIN` that points back at the shim.
pub const MAX_DEPTH: u32 = 16;

/// What the shim does with its arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Observed runs, in order: `(task, CLI_ARGS)`.
    Run(Vec<(String, Vec<String>)>),
    /// Hand every argument to go-task untouched.
    PassThrough,
}

pub fn plan(args: &[String]) -> Plan {
    let (names, rest) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], Some(&args[i + 1..])),
        None => (args, None),
    };
    let plain = |n: &String| !n.is_empty() && !n.starts_with('-') && !n.contains('=');
    if names.is_empty() || !names.iter().all(plain) {
        return Plan::PassThrough;
    }
    match (names, rest) {
        ([name], Some(rest)) => Plan::Run(vec![(name.clone(), rest.to_vec())]),
        (_, Some(_)) => Plan::PassThrough,
        (names, None) => Plan::Run(names.iter().map(|n| (n.clone(), Vec::new())).collect()),
    }
}

pub struct ShimOptions {
    pub args: Vec<String>,
    pub task_bin: String,
    pub trace_url: Option<String>,
}

/// `ctx` is `None` when no Taskfile was found: go-task then reports it.
pub async fn shim(ctx: Option<&Ctx>, opts: ShimOptions) -> Result<ExitCode> {
    let depth = std::env::var(DEPTH_ENV)
        .ok()
        .and_then(|d| d.trim().parse::<u32>().ok())
        .unwrap_or(0)
        + 1;
    if depth > MAX_DEPTH {
        bail!(
            "{DEPTH_ENV}={}: `{}` keeps resolving back to `taskgraph shim`; set TASKGRAPH_TASK_BIN to the real go-task binary",
            depth - 1,
            opts.task_bin
        );
    }

    let (Plan::Run(runs), Some(ctx)) = (plan(&opts.args), ctx) else {
        return pass_through(&opts.task_bin, &opts.args, depth);
    };
    if !observable(ctx, &runs) {
        return pass_through(&opts.task_bin, &opts.args, depth);
    }
    for (target, args) in runs {
        let code = run::run(
            ctx,
            RunOptions {
                target,
                args,
                verbose: false,
                task_bin: opts.task_bin.clone(),
                commands: false,
                trace_url: opts.trace_url.clone(),
                shim_depth: Some(depth),
            },
        )
        .await?;
        if code != 0 {
            return Ok(ExitCode::from(code));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Every name is in the parsed graph, or could come from an include the
/// parser could not read. Otherwise (including a Taskfile we cannot parse)
/// go-task gets to say what is wrong.
fn observable(ctx: &Ctx, runs: &[(String, Vec<String>)]) -> bool {
    let Ok(graph) = ctx.parse() else {
        return false;
    };
    if graph.includes.iter().any(|i| !i.resolved) {
        return true;
    }
    let index = GraphIndex::new(&graph);
    runs.iter().all(|(name, _)| index.resolve(name).is_some())
}

fn pass_through(task_bin: &str, args: &[String], depth: u32) -> Result<ExitCode> {
    debug!(task_bin, ?args, "passing through to go-task");
    let mut command = Command::new(task_bin);
    command.args(args).env(DEPTH_ENV, depth.to_string());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns on failure.
        let err = command.exec();
        Err(err).wrap_err_with(|| format!("exec {task_bin}; is go-task installed and on PATH?"))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .wrap_err_with(|| format!("starting {task_bin}; is go-task installed and on PATH?"))?;
        Ok(match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    fn runs(r: &[(&str, &[&str])]) -> Plan {
        Plan::Run(r.iter().map(|(n, a)| (n.to_string(), args(a))).collect())
    }

    #[test]
    fn plain_names_run_in_order() {
        assert_eq!(plan(&args(&["lint"])), runs(&[("lint", &[])]));
        assert_eq!(
            plan(&args(&["lint", "rust:test"])),
            runs(&[("lint", &[]), ("rust:test", &[])])
        );
    }

    #[test]
    fn cli_args_go_to_a_single_name_only() {
        assert_eq!(
            plan(&args(&["test", "--", "-p", "x", "--", "y"])),
            runs(&[("test", &["-p", "x", "--", "y"])])
        );
        assert_eq!(plan(&args(&["test", "--"])), runs(&[("test", &[])]));
        assert_eq!(plan(&args(&["a", "b", "--", "x"])), Plan::PassThrough);
    }

    #[test]
    fn anything_else_passes_through() {
        for a in [
            &[][..],
            &["--list"],
            &["-t", "x.yml", "lint"],
            &["lint", "--force"],
            &["lint", "VERSION=1"],
            &["--", "x"],
            &[""],
        ] {
            assert_eq!(plan(&args(a)), Plan::PassThrough, "{a:?}");
        }
    }
}
