//! A [`Layer`] backed by an external command that speaks nx's plugin JSON —
//! so a repo whose inference already lives in a TypeScript nx plugin can feed
//! butler without a Rust port. Configured by `butler.toml` `[graph] infer`
//! (an argv) or `butler --infer '<command>'`; `tools/nx/infer.ts` is this
//! repo's example, running `tools/nx/plugin.ts` itself. The native port
//! ([`super::native`]) stays the default: it needs no node process.
//!
//! # Protocol
//!
//! butler runs the argv with the phase appended as the last argument and the
//! workspace root as cwd, writes one JSON document to stdin and reads one
//! from stdout. The payloads are nx's own plugin contract, so the adapter is
//! a few lines around the plugin's exports:
//!
//! * `nodes` — stdin `{"workspaceRoot": "<abs>", "files": [...]}`, every
//!   workspace file (`git ls-files -co --exclude-standard`, sorted); the
//!   command filters them with its own glob, as nx does with
//!   `createNodesV2[0]`. stdout is the `createNodesV2` result:
//!   `[[file, {"projects": {root: {name?, tags?, targets, ...}}}], ...]`, in
//!   the order nx would merge it.
//! * `dependencies` — stdin `{"workspaceRoot": "<abs>", "projects": {name:
//!   {"root": root}}}`, every project after all layers' nodes merged. stdout
//!   is the `createDependencies` result, an array of nx `RawDependency`
//!   (`{source, target, type, sourceFile?}`). butler extension: `"dev": true`
//!   marks a dev-only edge, which image build contexts skip. A plain nx plugin
//!   never sets it (all its edges count as build edges); `infer.ts` sets it
//!   from `readCargoCrate().devOnly`, so its graph is generator-exact.
//!
//! A non-zero exit or unparseable stdout fails the graph build with the
//! command's stderr; on success stderr passes through, as a plugin's warnings
//! would in nx.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::process::{Command as Process, Stdio};

use eyre::{Result, WrapErr, bail, eyre};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{Contribution, Ctx, Dependency, Layer, Node, ProjectRoots};

pub struct Command {
    pub argv: Vec<String>,
}

/// nx's `CreateNodesResult`. Keys butler does not model (`externalNodes`) are
/// ignored, like unknown project fields.
#[derive(Deserialize)]
struct NodesResult {
    #[serde(default)]
    projects: BTreeMap<String, Contribution>,
}

/// nx hands plugins `workspaceRoot` as a string; `json!` would panic on a
/// non-UTF-8 path instead of failing.
fn workspace_root<'a>(ctx: &Ctx<'a>) -> Result<&'a str> {
    ctx.workspace_root.to_str().ok_or_else(|| {
        eyre!(
            "workspace root {} is not valid UTF-8",
            ctx.workspace_root.display()
        )
    })
}

impl Command {
    /// Run one phase: `payload` on stdin, `T` parsed from stdout.
    fn call<T: DeserializeOwned>(
        &self,
        ctx: &Ctx,
        phase: &str,
        payload: &serde_json::Value,
    ) -> Result<T> {
        let Some((program, args)) = self.argv.split_first() else {
            bail!("the external inferrer command is empty (`[graph] infer` / `--infer`)");
        };
        let shown = format!("{} {phase}", self.argv.join(" "));
        let input = serde_json::to_vec(payload)?;
        let mut child = Process::new(program)
            .args(args)
            .arg(phase)
            .current_dir(ctx.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("cannot run external inferrer `{shown}`"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        // Feed stdin from its own thread: a command that writes before it has
        // read everything would otherwise fill its stdout pipe and deadlock.
        let (written, output) = std::thread::scope(|s| {
            let writer = s.spawn(move || stdin.write_all(&input));
            let output = child.wait_with_output();
            (writer.join().expect("stdin writer panicked"), output)
        });
        let output = output.wrap_err_with(|| format!("waiting for `{shown}`"))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            bail!(
                "external inferrer `{shown}` failed ({}):\n{stderr}",
                output.status
            );
        }
        // A command may stop reading once it has what it needs; only a write
        // failure other than the closed pipe is butler's problem.
        if let Err(e) = written
            && e.kind() != ErrorKind::BrokenPipe
        {
            return Err(eyre!(e).wrap_err(format!("writing stdin of `{shown}`")));
        }
        let parsed = serde_json::from_slice(&output.stdout).map_err(|e| {
            eyre!("external inferrer `{shown}` printed invalid {phase} JSON: {e}\n{stderr}")
        })?;
        eprint!("{stderr}");
        Ok(parsed)
    }
}

impl Layer for Command {
    fn name(&self) -> &str {
        "external"
    }

    fn nodes(&self, ctx: &Ctx) -> Result<Vec<Node>> {
        let payload = json!({ "workspaceRoot": workspace_root(ctx)?, "files": ctx.files });
        let entries: Vec<(String, NodesResult)> = self.call(ctx, "nodes", &payload)?;
        Ok(entries
            .into_iter()
            .map(|(file, r)| Node {
                file,
                projects: r.projects,
            })
            .collect())
    }

    fn dependencies(&self, ctx: &Ctx, projects: &ProjectRoots) -> Result<Vec<Dependency>> {
        let projects: serde_json::Map<_, _> = projects
            .iter()
            .map(|(name, root)| (name.clone(), json!({ "root": root })))
            .collect();
        let payload = json!({ "workspaceRoot": workspace_root(ctx)?, "projects": projects });
        self.call(ctx, "dependencies", &payload)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A command that dies before reading stdin closes the pipe under
    /// butler's writer. The error must be the command's own failure and
    /// stderr — what the user can act on — not "broken pipe".
    #[test]
    fn command_failing_before_reading_stdin_reports_its_stderr() {
        // Far past a pipe buffer, so the write really hits the closed pipe.
        let files: Vec<String> = (0..20_000)
            .map(|i| format!("libs/crate{i}/Cargo.toml"))
            .collect();
        let ctx = Ctx {
            workspace_root: Path::new("."),
            files: &files,
            settings: None,
            overrides: None,
        };
        let cmd = Command {
            argv: ["sh", "-c", "echo 'plugin exploded' >&2; exit 3"]
                .map(String::from)
                .to_vec(),
        };
        let err = format!("{:#}", cmd.nodes(&ctx).unwrap_err());
        assert!(err.contains("plugin exploded"), "{err}");
        assert!(err.contains("failed"), "{err}");
    }
}
