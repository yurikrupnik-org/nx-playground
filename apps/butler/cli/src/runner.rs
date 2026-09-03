//! Plan execution: dependency-ordered parallel scheduling with cache
//! short-circuiting and cargo batching.
//!
//! Cargo batching is the reason butler exists as a *compiler* rather than a
//! task loop: N ready tasks of the shape `cargo <verb> <flags> --package X`
//! collapse into one `cargo <verb> <flags> --package X1 ... --package XN`
//! invocation. Per-crate cargo processes serialize on the target-dir lock and
//! defeat cargo's own parallelism; one batched invocation does not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use eyre::{Result, eyre};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::Cache;
use crate::graph::{Task, TaskId};

pub struct RunOptions {
    pub parallel: usize,
    pub no_cache: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Cached,
    Failed,
    Skipped, // upstream failure
}

pub struct TaskReport {
    pub id: TaskId,
    pub outcome: Outcome,
}

/// A unit handed to the executor: one task, or a batch of cargo tasks that
/// share a canonical command.
enum Unit {
    Single(Task, String),
    CargoBatch {
        /// (task, its hash, its package name)
        members: Vec<(Task, String, String)>,
        command: String,
    },
}

/// `cargo <verb> [flags] --package X [flags]` -> (canonical command without
/// the package, package name). Only commands of exactly this shape batch.
fn cargo_batchable(task: &Task) -> Option<(String, String)> {
    if task.resolved.commands.len() != 1 || task.resolved.cwd != "." {
        return None;
    }
    let tokens: Vec<&str> = task.resolved.commands[0].split_whitespace().collect();
    if tokens.first() != Some(&"cargo") {
        return None;
    }
    let mut canonical = Vec::with_capacity(tokens.len());
    let mut package = None;
    let mut i = 0;
    while i < tokens.len() {
        if (tokens[i] == "--package" || tokens[i] == "-p") && i + 1 < tokens.len() {
            if package.is_some() {
                return None; // multiple packages: leave it alone
            }
            package = Some(tokens[i + 1].to_string());
            i += 2;
        } else {
            canonical.push(tokens[i]);
            i += 1;
        }
    }
    package.map(|p| (canonical.join(" "), p))
}

struct UnitResult {
    /// (task id, hash, outcome, output, duration)
    tasks: Vec<(TaskId, String, Outcome, String, u128)>,
}

pub async fn execute(
    root: &Path,
    mut tasks: Vec<Task>,
    hashes: BTreeMap<TaskId, String>,
    opts: RunOptions,
) -> Result<Vec<TaskReport>> {
    let cache = Cache::new(root);

    if opts.dry_run {
        for task in &tasks {
            let hash = &hashes[&task.id];
            let hit = if task.resolved.cache && cache.lookup(hash).is_some() {
                " [cache hit]"
            } else {
                ""
            };
            println!("{}  {}{hit}", task.id, task.resolved.commands.join(" && "));
        }
        return Ok(tasks
            .into_iter()
            .map(|t| TaskReport {
                id: t.id,
                outcome: Outcome::Skipped,
            })
            .collect());
    }

    // Cache short-circuit before scheduling.
    let mut reports: Vec<TaskReport> = Vec::new();
    let mut done: BTreeSet<TaskId> = BTreeSet::new();
    if !opts.no_cache {
        let mut remaining = Vec::with_capacity(tasks.len());
        for task in tasks {
            let hash = &hashes[&task.id];
            if task.resolved.cache {
                if let Some(meta) = cache.lookup(hash) {
                    cache.restore(root, &meta)?;
                    let out = cache.stdout(hash);
                    print_task(&task.id, "cache", &out);
                    done.insert(task.id.clone());
                    reports.push(TaskReport {
                        id: task.id,
                        outcome: Outcome::Cached,
                    });
                    continue;
                }
            }
            remaining.push(task);
        }
        tasks = remaining;
    }

    // Ready-set scheduling over the remaining tasks.
    let mut pending: BTreeMap<TaskId, Task> =
        tasks.into_iter().map(|t| (t.id.clone(), t)).collect();
    let semaphore = Arc::new(Semaphore::new(opts.parallel.max(1)));
    let cache = Arc::new(cache);
    let root = root.to_path_buf();
    let mut in_flight: JoinSet<Result<UnitResult>> = JoinSet::new();
    let mut scheduled: BTreeSet<TaskId> = BTreeSet::new();
    let mut failed = false;

    loop {
        if !failed {
            // Collect ready tasks (all deps done, not yet scheduled).
            let ready: Vec<TaskId> = pending
                .values()
                .filter(|t| !scheduled.contains(&t.id))
                .filter(|t| t.deps.iter().all(|d| done.contains(d)))
                .map(|t| t.id.clone())
                .collect();

            // Group ready cargo tasks by canonical command; the rest run alone.
            let mut batches: BTreeMap<String, Vec<(Task, String, String)>> = BTreeMap::new();
            let mut singles: Vec<Task> = Vec::new();
            for id in ready {
                let task = pending[&id].clone();
                match cargo_batchable(&task) {
                    Some((canonical, package)) => {
                        let hash = hashes[&id].clone();
                        batches
                            .entry(canonical)
                            .or_default()
                            .push((task, hash, package));
                    }
                    None => singles.push(task),
                }
            }

            for (canonical, members) in batches {
                for (t, _, _) in &members {
                    scheduled.insert(t.id.clone());
                }
                let unit = if members.len() == 1 {
                    let (task, hash, _) = members.into_iter().next().expect("len checked");
                    Unit::Single(task, hash)
                } else {
                    let packages: Vec<String> = members
                        .iter()
                        .map(|(_, _, p)| format!("--package {p}"))
                        .collect();
                    let command = format!("{canonical} {}", packages.join(" "));
                    Unit::CargoBatch { members, command }
                };
                spawn_unit(&mut in_flight, unit, &root, &semaphore, &cache);
            }
            for task in singles {
                scheduled.insert(task.id.clone());
                let hash = hashes[&task.id].clone();
                spawn_unit(
                    &mut in_flight,
                    Unit::Single(task, hash),
                    &root,
                    &semaphore,
                    &cache,
                );
            }
        }

        let Some(joined) = in_flight.join_next().await else {
            break; // nothing running and nothing schedulable
        };
        let result = joined.map_err(|e| eyre!("task panicked: {e}"))??;
        for (id, _hash, outcome, output, duration_ms) in result.tasks {
            let label = match outcome {
                Outcome::Success => "ok",
                Outcome::Failed => "FAILED",
                Outcome::Cached => "cache",
                Outcome::Skipped => "skipped",
            };
            print_task(&id, &format!("{label} {duration_ms}ms"), &output);
            if outcome == Outcome::Failed {
                failed = true;
            }
            done.insert(id.clone());
            pending.remove(&id);
            reports.push(TaskReport { id, outcome });
        }
    }

    // Anything never scheduled was blocked by a failure.
    for (id, _) in pending {
        if !done.contains(&id) {
            reports.push(TaskReport {
                id,
                outcome: Outcome::Skipped,
            });
        }
    }
    Ok(reports)
}

fn spawn_unit(
    set: &mut JoinSet<Result<UnitResult>>,
    unit: Unit,
    root: &Path,
    semaphore: &Arc<Semaphore>,
    cache: &Arc<Cache>,
) {
    let root = root.to_path_buf();
    let semaphore = Arc::clone(semaphore);
    let cache = Arc::clone(cache);
    set.spawn(async move {
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|e| eyre!("semaphore closed: {e}"))?;
        match unit {
            Unit::Single(task, hash) => {
                let started = Instant::now();
                let (ok, output) = run_task_commands(&root, &task).await?;
                let duration = started.elapsed().as_millis();
                let outcome = if ok {
                    if task.resolved.cache {
                        cache.store(
                            &root,
                            &task.id.to_string(),
                            &hash,
                            &output,
                            duration,
                            &task.resolved.outputs,
                        )?;
                    }
                    Outcome::Success
                } else {
                    Outcome::Failed
                };
                Ok(UnitResult {
                    tasks: vec![(task.id, hash, outcome, output, duration)],
                })
            }
            Unit::CargoBatch { members, command } => {
                let started = Instant::now();
                let (ok, output) = run_command(&root, ".", &command).await?;
                let duration = started.elapsed().as_millis();
                if ok {
                    let mut tasks = Vec::with_capacity(members.len());
                    let n = members.len() as u128;
                    for (task, hash, _) in members {
                        if task.resolved.cache {
                            cache.store(
                                &root,
                                &task.id.to_string(),
                                &hash,
                                &output,
                                duration,
                                &task.resolved.outputs,
                            )?;
                        }
                        tasks.push((
                            task.id,
                            hash,
                            Outcome::Success,
                            format!("[batched: {command}]\n{output}"),
                            duration / n.max(1),
                        ));
                    }
                    return Ok(UnitResult { tasks });
                }
                // Batch failed: rerun members individually to attribute the
                // failure precisely (and cache the ones that pass).
                let mut tasks = Vec::with_capacity(members.len());
                for (task, hash, _) in members {
                    let started = Instant::now();
                    let (ok, output) = run_task_commands(&root, &task).await?;
                    let duration = started.elapsed().as_millis();
                    let outcome = if ok {
                        if task.resolved.cache {
                            cache.store(
                                &root,
                                &task.id.to_string(),
                                &hash,
                                &output,
                                duration,
                                &task.resolved.outputs,
                            )?;
                        }
                        Outcome::Success
                    } else {
                        Outcome::Failed
                    };
                    tasks.push((task.id.clone(), hash, outcome, output, duration));
                }
                Ok(UnitResult { tasks })
            }
        }
    });
}

/// Run a task's command list, honoring `parallel` for multi-command targets.
async fn run_task_commands(root: &Path, task: &Task) -> Result<(bool, String)> {
    let cwd = &task.resolved.cwd;
    if task.resolved.commands.len() == 1 {
        return run_command(root, cwd, &task.resolved.commands[0]).await;
    }
    if task.resolved.parallel_commands {
        let mut set = JoinSet::new();
        for (i, cmd) in task.resolved.commands.iter().enumerate() {
            let root = root.to_path_buf();
            let cwd = cwd.clone();
            let cmd = cmd.clone();
            set.spawn(async move { (i, run_command(&root, &cwd, &cmd).await) });
        }
        let mut parts: Vec<(usize, bool, String)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            let (i, result) = joined.map_err(|e| eyre!("command panicked: {e}"))?;
            let (ok, out) = result?;
            parts.push((i, ok, out));
        }
        parts.sort_by_key(|(i, ..)| *i);
        let ok = parts.iter().all(|(_, ok, _)| *ok);
        let output = parts
            .into_iter()
            .map(|(_, _, out)| out)
            .collect::<Vec<_>>()
            .join("");
        Ok((ok, output))
    } else {
        let mut output = String::new();
        for cmd in &task.resolved.commands {
            let (ok, out) = run_command(root, cwd, cmd).await?;
            output.push_str(&out);
            if !ok {
                return Ok((false, output));
            }
        }
        Ok((true, output))
    }
}

async fn run_command(root: &Path, cwd: &str, command: &str) -> Result<(bool, String)> {
    let dir = if cwd == "." {
        root.to_path_buf()
    } else {
        root.join(cwd)
    };
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&dir)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| eyre!("spawning `{command}`: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

fn print_task(id: &TaskId, status: &str, output: &str) {
    println!("\n> {id}  [{status}]");
    let trimmed = output.trim_end();
    if !trimmed.is_empty() {
        println!("{trimmed}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedTarget;

    fn task(project: &str, command: &str, cwd: &str) -> Task {
        Task {
            id: TaskId {
                project: project.into(),
                target: "build".into(),
            },
            resolved: ResolvedTarget {
                executor: "butler:cargo".into(),
                commands: vec![command.into()],
                parallel_commands: true,
                cwd: cwd.into(),
                cache: false,
                inputs: None,
                outputs: vec![],
                depends_on: vec![],
            },
            deps: BTreeSet::new(),
        }
    }

    #[test]
    fn cargo_commands_canonicalize_for_batching() {
        let t = task("zerg_api", "cargo build --package zerg_api", ".");
        assert_eq!(
            cargo_batchable(&t),
            Some(("cargo build".into(), "zerg_api".into()))
        );

        // Flags after the package survive canonicalization.
        let t = task("zerg_api", "cargo build --package zerg_api --release", ".");
        assert_eq!(
            cargo_batchable(&t),
            Some(("cargo build --release".into(), "zerg_api".into()))
        );

        // Same canonical form -> same batch key.
        let a = cargo_batchable(&task("a", "cargo clippy --package a", ".")).unwrap();
        let b = cargo_batchable(&task("b", "cargo clippy --package b", ".")).unwrap();
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn non_batchable_commands_are_left_alone() {
        // Not cargo.
        assert!(cargo_batchable(&task("w", "bun run build", "apps/w")).is_none());
        // Cargo but project-local cwd.
        assert!(cargo_batchable(&task("x", "cargo build --package x", "apps/x")).is_none());
        // No package flag (workspace-wide already).
        assert!(cargo_batchable(&task("y", "cargo check --workspace", ".")).is_none());
    }
}
