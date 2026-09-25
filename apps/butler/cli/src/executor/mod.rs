//! Executor support: what a target's nx executor would actually run.
//!
//! nx executors are JavaScript functions; butler cannot call them, so each
//! supported executor is ported here as a *planner*: given the task's
//! effective options it returns the exact processes (or in-process steps) the
//! executor performs. Anything a planner does not understand is an error —
//! a wrong plan that "mostly works" is worse than a loud refusal, because the
//! same target still runs correctly through nx.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use eyre::{Result, bail};

use crate::config::JsonMap;
use crate::graph::{Project, ProjectGraph};

pub mod args;
pub mod env;
pub mod nx_container;
pub mod nxlv_python;
pub(crate) mod run_commands;
pub mod schema;

/// Everything a planner may read. Planning happens before any task runs, so
/// a planner must not touch the workspace; work that depends on the state
/// other tasks leave behind belongs in a [`Step::Native`].
pub struct PlanCtx<'a> {
    pub workspace_root: &'a Path,
    pub graph: &'a ProjectGraph,
    pub project: &'a Project,
    pub target: &'a str,
    /// The configuration nx would pass the executor (already checked to exist
    /// on this target, per nx's `resolveConfiguration`).
    pub configuration: Option<&'a str>,
    /// Target `options` with the selected configuration assigned over them
    /// (nx: `Object.assign({...options}, configurations[c])`). CLI overrides
    /// are NOT merged in: pass the executor's schema to
    /// [`schema::combine_options`] to get what nx hands the executor.
    pub options: &'a JsonMap,
    /// CLI overrides as nx's `createOverrides` parses them (yargs-parser,
    /// dot-notation, no camel-case expansion; `_` present only when there are
    /// positionals). Empty for tasks that were not requested on the command
    /// line unless a `dependsOn` entry forwards params.
    pub overrides: &'a JsonMap,
    /// The same overrides as the user typed them (nx's
    /// `__overrides_unparsed__`), for schemas declaring
    /// `$default: {$source: "unparsed"}`.
    pub unparsed: &'a [String],
    /// The task's full environment (butler's own env plus nx's dotenv files):
    /// what an nx executor sees as `process.env`.
    pub env: &'a BTreeMap<String, String>,
}

/// What a task runs.
#[derive(Clone)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Run `steps` concurrently (all must succeed) instead of in order,
    /// stopping at the first failure.
    pub parallel: bool,
}

/// One unit of work. `cwd` is workspace-root-relative (`.` = the root); `env`
/// is layered over the task environment.
#[derive(Clone)]
pub enum Step {
    /// `sh -c <script>` — run-commands semantics, the shell does expansion.
    Shell {
        script: String,
        cwd: String,
        env: BTreeMap<String, String>,
    },
    /// A process without a shell: arguments reach the program verbatim.
    Exec {
        argv: Vec<String>,
        cwd: String,
        env: BTreeMap<String, String>,
    },
    /// Work an executor does in-process (file copies, reading a lock export
    /// before rewriting a manifest, ...). `label` must describe it fully and
    /// deterministically: it is what `--dry-run` prints. (Task hashes cover
    /// the target configuration and overrides, as nx's do, not the plan.)
    Native { label: String, run: NativeFn },
}

pub type NativeFn = Arc<dyn Fn(&NativeCtx<'_>) -> Result<StepOutput> + Send + Sync>;

pub struct NativeCtx<'a> {
    pub workspace_root: &'a Path,
    /// The task environment (same as [`PlanCtx::env`]); pass it to any
    /// process the step spawns.
    pub env: &'a BTreeMap<String, String>,
}

/// Result of a step: success plus combined stdout/stderr.
pub struct StepOutput {
    pub success: bool,
    pub output: String,
}

impl fmt::Display for Step {
    /// The human form of a step, as `--dry-run` prints it. `PATH` is left
    /// out: planners only prepend `node_modules/.bin` directories (or drop
    /// them), and the full value is noise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (body, cwd, env) = match self {
            Step::Shell { script, cwd, env } => (script.clone(), cwd, env),
            Step::Exec { argv, cwd, env } => (shell_join(argv), cwd, env),
            Step::Native { label, .. } => return f.write_str(label),
        };
        if cwd != "." {
            write!(f, "(cd {} && ", shell_quote(cwd))?;
        }
        for (k, v) in env.iter().filter(|(k, _)| *k != "PATH") {
            write!(f, "{k}={} ", shell_quote(v))?;
        }
        f.write_str(&body)?;
        if cwd != "." {
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// Plan `executor` for one task.
pub fn plan(executor: &str, ctx: &PlanCtx<'_>) -> Result<Plan> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    match executor {
        "nx:run-commands" => run_commands::run_commands(ctx),
        "nx:run-script" => run_commands::run_script(ctx),
        "@nx-tools/nx-container:build" => nx_container::build(ctx),
        e if e.starts_with("@nxlv/python:") => nxlv_python::plan(&e["@nxlv/python:".len()..], ctx),
        e if e.ends_with(":release-publish") => bail!(
            "{task}: `{e}` publishes as part of `nx release`; run `bun nx release publish` \
             (butler does not own releases)"
        ),
        e => bail!(
            "{task}: executor `{e}` is not supported by butler; run it through nx \
             (butler coexists with nx for such targets)"
        ),
    }
}

/// POSIX-shell quoting for display and for composing shell scripts.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&b))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}
