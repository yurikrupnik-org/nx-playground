//! Plan execution: dependency-ordered parallel scheduling with cache
//! short-circuiting and cargo batching.
//!
//! Cargo batching is the reason butler exists as a *compiler* rather than a
//! task loop: N ready tasks of the shape `cargo <verb> <flags> --package X`
//! collapse into one `cargo <verb> <flags> --package X1 ... --package XN`
//! invocation. Per-crate cargo processes serialize on the target-dir lock and
//! defeat cargo's own parallelism; one batched invocation does not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use eyre::{Result, eyre};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::Cache;
use crate::executor::{NativeCtx, Step, StepOutput};
use crate::task::{Task, TaskId};

pub struct RunOptions {
    pub parallel: usize,
    /// Do not read the cache (results are still stored), like nx's
    /// `--skip-nx-cache`.
    pub skip_cache: bool,
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
/// share a canonical command and environment.
enum Unit {
    Single(Box<Task>, String),
    CargoBatch {
        /// (task, its hash, its package name)
        members: Vec<(Task, String, String)>,
        command: String,
        env: BTreeMap<String, String>,
    },
}

/// Ready cargo tasks that share a command template and environment.
struct Batch {
    template: String,
    env: BTreeMap<String, String>,
    /// (task, its hash, its package name)
    members: Vec<(Task, String, String)>,
}

/// Variables nx sets per task; they differ between otherwise identical cargo
/// tasks and must not keep them from batching.
const PER_TASK_VARS: [&str; 5] = [
    "NX_TASK_TARGET_PROJECT",
    "NX_TASK_TARGET_TARGET",
    "NX_TASK_TARGET_CONFIGURATION",
    "NX_TASK_HASH",
    "LERNA_PACKAGE_NAME",
];

/// Where the packages go in a batched command.
const PACKAGES: &str = "{packages}";

/// `cargo <verb> [flags] --package X [flags] [-- args]`, run from the
/// workspace root -> (batch key, command template with [`PACKAGES`] where
/// `--package X` was, package name, env). Only plain commands of exactly this
/// shape (no shell syntax), with the same environment, batch: the packages
/// must stay before any `--`, or cargo hands them to the tool it runs.
fn cargo_batchable(task: &Task) -> Option<(String, String, String, BTreeMap<String, String>)> {
    let [Step::Shell { script, cwd, env }] = task.resolved.plan.steps.as_slice() else {
        return None;
    };
    if cwd != "." || script.contains(|c: char| "|&;<>()$`\\\"'*?[]{}~#!\n".contains(c)) {
        return None;
    }
    let tokens: Vec<&str> = script.split_whitespace().collect();
    if tokens.first() != Some(&"cargo") {
        return None;
    }
    let mut template = Vec::with_capacity(tokens.len());
    let mut package = None;
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "--" {
            template.extend_from_slice(&tokens[i..]);
            break;
        }
        if (tokens[i] == "--package" || tokens[i] == "-p") && i + 1 < tokens.len() {
            if package.is_some() {
                return None; // multiple packages: leave it alone
            }
            package = Some(tokens[i + 1].to_string());
            template.push(PACKAGES);
            i += 2;
        } else {
            template.push(tokens[i]);
            i += 1;
        }
    }
    let package = package?;
    let mut full_env: BTreeMap<String, String> = task
        .env
        .iter()
        .filter(|(k, _)| !PER_TASK_VARS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    full_env.extend(env.clone());
    let template = template.join(" ");
    let key = format!(
        "{template}\0{}",
        crate::hash::canonical(&serde_json::to_value(&full_env).ok()?)
    );
    Some((key, template, package, full_env))
}

struct UnitResult {
    /// (task id, outcome, output, duration)
    tasks: Vec<(TaskId, Outcome, String, u128)>,
}

pub async fn execute(
    root: &Path,
    tasks: Vec<Task>,
    hashes: BTreeMap<TaskId, String>,
    opts: RunOptions,
) -> Result<Vec<TaskReport>> {
    let cache = Cache::new(root);

    if opts.dry_run {
        for task in &tasks {
            let hash = &hashes[&task.id];
            let hit = if task.resolved.cache && !opts.skip_cache && cache.lookup(hash).is_some() {
                "  [cache hit]"
            } else {
                ""
            };
            println!("{}  ({}){hit}", task.id, task.resolved.executor);
            if !task.deps.is_empty() {
                let deps: Vec<String> = task.deps.iter().map(ToString::to_string).collect();
                println!("    after: {}", deps.join(", "));
            }
            if task.resolved.plan.steps.is_empty() {
                println!("    (nothing to run)");
            }
            let mode = if task.resolved.plan.parallel && task.resolved.plan.steps.len() > 1 {
                "|"
            } else {
                "$"
            };
            for step in &task.resolved.plan.steps {
                println!("    {mode} {step}");
            }
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
    let mut remaining = Vec::with_capacity(tasks.len());
    for task in tasks {
        let hash = &hashes[&task.id];
        if task.resolved.cache
            && !opts.skip_cache
            && let Some(meta) = cache.lookup(hash)
        {
            cache.restore(root, &meta)?;
            print_task(&task.id, "cache", &cache.stdout(hash));
            done.insert(task.id.clone());
            reports.push(TaskReport {
                id: task.id,
                outcome: Outcome::Cached,
            });
            continue;
        }
        remaining.push(task);
    }

    // Ready-set scheduling over the remaining tasks.
    let mut pending: BTreeMap<TaskId, Task> =
        remaining.into_iter().map(|t| (t.id.clone(), t)).collect();
    let semaphore = Arc::new(Semaphore::new(opts.parallel.max(1)));
    let cache = Arc::new(cache);
    let root = root.to_path_buf();
    let mut in_flight: JoinSet<Result<UnitResult>> = JoinSet::new();
    let mut scheduled: BTreeSet<TaskId> = BTreeSet::new();
    let mut failed = false;

    loop {
        if !failed {
            let ready: Vec<TaskId> = pending
                .values()
                .filter(|t| !scheduled.contains(&t.id))
                .filter(|t| t.deps.iter().all(|d| done.contains(d)))
                .map(|t| t.id.clone())
                .collect();

            // Group ready cargo tasks by batch key; the rest run alone.
            let mut batches: BTreeMap<String, Batch> = BTreeMap::new();
            let mut singles: Vec<Task> = Vec::new();
            for id in ready {
                let task = pending[&id].clone();
                match cargo_batchable(&task) {
                    Some((key, template, package, env)) => {
                        let hash = hashes[&id].clone();
                        batches
                            .entry(key)
                            .or_insert_with(|| Batch {
                                template,
                                env,
                                members: Vec::new(),
                            })
                            .members
                            .push((task, hash, package));
                    }
                    None => singles.push(task),
                }
            }

            for batch in batches.into_values() {
                let Batch {
                    template,
                    env,
                    members,
                } = batch;
                for (t, _, _) in &members {
                    scheduled.insert(t.id.clone());
                }
                let unit = if members.len() == 1 {
                    let (task, hash, _) = members.into_iter().next().expect("len checked");
                    Unit::Single(Box::new(task), hash)
                } else {
                    let packages: Vec<String> = members
                        .iter()
                        .map(|(_, _, p)| format!("--package {p}"))
                        .collect();
                    let command = template.replace(PACKAGES, &packages.join(" "));
                    Unit::CargoBatch {
                        members,
                        command,
                        env,
                    }
                };
                spawn_unit(&mut in_flight, unit, &root, &semaphore, &cache);
            }
            for task in singles {
                scheduled.insert(task.id.clone());
                let hash = hashes[&task.id].clone();
                spawn_unit(
                    &mut in_flight,
                    Unit::Single(Box::new(task), hash),
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
        for (id, outcome, output, duration_ms) in result.tasks {
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
                let out = run_task(&root, &task, &hash).await?;
                let duration = started.elapsed().as_millis();
                let outcome = finish(&cache, &root, &task, &hash, &out, duration)?;
                Ok(UnitResult {
                    tasks: vec![(task.id, outcome, out.output, duration)],
                })
            }
            Unit::CargoBatch {
                members,
                command,
                env,
            } => {
                let started = Instant::now();
                let step = Step::Shell {
                    script: command.clone(),
                    cwd: ".".into(),
                    env: BTreeMap::new(),
                };
                let out = run_step(&root, &step, Arc::new(env)).await?;
                let duration = started.elapsed().as_millis();
                if out.success {
                    let n = members.len() as u128;
                    let mut tasks = Vec::with_capacity(members.len());
                    for (task, hash, _) in members {
                        let outcome = finish(&cache, &root, &task, &hash, &out, duration)?;
                        tasks.push((
                            task.id,
                            outcome,
                            format!("[batched: {command}]\n{}", out.output),
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
                    let out = run_task(&root, &task, &hash).await?;
                    let duration = started.elapsed().as_millis();
                    let outcome = finish(&cache, &root, &task, &hash, &out, duration)?;
                    tasks.push((task.id.clone(), outcome, out.output, duration));
                }
                Ok(UnitResult { tasks })
            }
        }
    });
}

/// Record the result; cache only successful runs of cacheable targets.
fn finish(
    cache: &Cache,
    root: &Path,
    task: &Task,
    hash: &str,
    out: &StepOutput,
    duration: u128,
) -> Result<Outcome> {
    if !out.success {
        return Ok(Outcome::Failed);
    }
    if task.resolved.cache {
        cache.store(
            root,
            &task.id.to_string(),
            hash,
            &out.output,
            duration,
            &task.resolved.outputs,
        )?;
    }
    Ok(Outcome::Success)
}

/// Run a task's plan: steps concurrently (all must pass; the first failure
/// stops the rest, as run-commands does) or in order, stopping at the first
/// failure.
async fn run_task(root: &Path, task: &Task, hash: &str) -> Result<StepOutput> {
    let mut env = (*task.env).clone();
    env.insert("NX_TASK_HASH".into(), hash.to_string());
    let env = Arc::new(env);
    let steps = &task.resolved.plan.steps;
    if steps.len() == 1 || !task.resolved.plan.parallel {
        let mut output = String::new();
        for step in steps {
            let out = run_step(root, step, Arc::clone(&env)).await?;
            output.push_str(&out.output);
            if !out.success {
                output.push_str(&format!(
                    "\nWarning: command \"{step}\" exited with non-zero status code\n"
                ));
                return Ok(StepOutput {
                    success: false,
                    output,
                });
            }
        }
        return Ok(StepOutput {
            success: true,
            output,
        });
    }
    let mut set = JoinSet::new();
    for (i, step) in steps.iter().enumerate() {
        let root = root.to_path_buf();
        let step = step.clone();
        let env = Arc::clone(&env);
        set.spawn(async move { (i, run_step(&root, &step, env).await) });
    }
    let mut parts: Vec<(usize, String)> = Vec::new();
    let mut success = true;
    while let Some(joined) = set.join_next().await {
        let (i, result) = match joined {
            Ok(v) => v,
            // A sibling aborted after the first failure.
            Err(e) if e.is_cancelled() => continue,
            Err(e) => return Err(eyre!("command panicked: {e}")),
        };
        let out = result?;
        parts.push((i, out.output));
        if !out.success && success {
            success = false;
            let step = &steps[i];
            parts.push((
                usize::MAX,
                format!("\nWarning: command \"{step}\" exited with non-zero status code\n"),
            ));
            // Dropping the remaining handles kills their processes.
            set.abort_all();
        }
    }
    parts.sort_by_key(|(i, _)| *i);
    Ok(StepOutput {
        success,
        output: parts
            .into_iter()
            .map(|(_, o)| o)
            .collect::<Vec<_>>()
            .join(""),
    })
}

async fn run_step(
    root: &Path,
    step: &Step,
    env: Arc<BTreeMap<String, String>>,
) -> Result<StepOutput> {
    match step {
        Step::Shell {
            script,
            cwd,
            env: extra,
        } => {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(script);
            spawn(cmd, root, cwd, &env, extra, script).await
        }
        Step::Exec {
            argv,
            cwd,
            env: extra,
        } => {
            let (program, args) = argv
                .split_first()
                .ok_or_else(|| eyre!("empty argv in plan step"))?;
            let mut cmd = Command::new(program);
            cmd.args(args);
            spawn(cmd, root, cwd, &env, extra, &step.to_string()).await
        }
        Step::Native { label, run } => {
            let run = Arc::clone(run);
            let root: PathBuf = root.to_path_buf();
            let label = label.clone();
            tokio::task::spawn_blocking(move || {
                run(&NativeCtx {
                    workspace_root: &root,
                    env: &env,
                })
            })
            .await
            .map_err(|e| eyre!("`{label}` panicked: {e}"))?
        }
    }
}

async fn spawn(
    mut cmd: Command,
    root: &Path,
    cwd: &str,
    env: &BTreeMap<String, String>,
    extra: &BTreeMap<String, String>,
    what: &str,
) -> Result<StepOutput> {
    let out = cmd
        .current_dir(root.join(cwd))
        .env_clear()
        .envs(env)
        .envs(extra)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| eyre!("spawning `{what}`: {e}"))?;
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(StepOutput {
        success: out.status.success(),
        output,
    })
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
    use crate::executor::Plan;
    use crate::task::{Overrides, ResolvedTarget};

    fn task(project: &str, command: &str, cwd: &str) -> Task {
        Task {
            id: TaskId {
                project: project.into(),
                target: "build".into(),
                configuration: None,
            },
            resolved: ResolvedTarget {
                executor: "nx:run-commands".into(),
                plan: Plan {
                    steps: vec![Step::Shell {
                        script: command.into(),
                        cwd: cwd.into(),
                        env: BTreeMap::from([(
                            "PATH".into(),
                            "/w/node_modules/.bin:/usr/bin".into(),
                        )]),
                    }],
                    parallel: false,
                },
                cache: false,
                inputs: None,
                outputs: vec![],
            },
            deps: BTreeSet::new(),
            env: Arc::new(BTreeMap::from([
                ("NX_TASK_TARGET_PROJECT".into(), project.into()),
                ("HOME".into(), "/h".into()),
            ])),
            overrides: Overrides::default(),
        }
    }

    #[test]
    fn cargo_commands_canonicalize_for_batching() {
        let (key_a, canonical, package, env) = cargo_batchable(&task(
            "zerg_api",
            "cargo build --package zerg_api --release",
            ".",
        ))
        .unwrap();
        assert_eq!(canonical, "cargo build {packages} --release");
        // Packages stay in front of `--`, or clippy would receive them.
        let (_, template, ..) = cargo_batchable(&task(
            "a",
            "cargo clippy --package a --all-targets -- -D warnings",
            ".",
        ))
        .unwrap();
        assert_eq!(
            template,
            "cargo clippy {packages} --all-targets -- -D warnings"
        );
        assert!(cargo_batchable(&task("b", "cargo run -- -p b", ".")).is_none());
        assert!(cargo_batchable(&task("c", "cargo test --package c && git diff", ".")).is_none());
        assert_eq!(package, "zerg_api");
        assert!(!env.contains_key("NX_TASK_TARGET_PROJECT"));
        // Same command shape and environment -> same batch, whatever the
        // per-task nx variables say.
        let (key_b, ..) = cargo_batchable(&task(
            "todo_api",
            "cargo build --package todo_api --release",
            ".",
        ))
        .unwrap();
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn non_batchable_commands_are_left_alone() {
        assert!(cargo_batchable(&task("w", "bun run build", "apps/w")).is_none());
        assert!(cargo_batchable(&task("x", "cargo build --package x", "apps/x")).is_none());
        assert!(cargo_batchable(&task("y", "cargo check --workspace", ".")).is_none());
        assert!(cargo_batchable(&task("z", "cargo build -p a -p b", ".")).is_none());
    }

    #[tokio::test]
    async fn parallel_plan_fails_fast_and_stops_siblings() {
        let shell = |s: &str| Step::Shell {
            script: s.into(),
            cwd: ".".into(),
            env: BTreeMap::new(),
        };
        let mut t = task("p", "true", ".");
        t.resolved.plan.steps = vec![shell("sleep 30; echo late"), shell("exit 2")];
        t.resolved.plan.parallel = true;
        t.env = Arc::new(BTreeMap::from([(
            "PATH".into(),
            std::env::var("PATH").unwrap_or_default(),
        )]));
        let started = Instant::now();
        let out = run_task(Path::new("/"), &t, "h").await.unwrap();
        assert!(!out.success);
        assert!(!out.output.contains("late"));
        assert!(started.elapsed().as_secs() < 20);
    }

    #[tokio::test]
    async fn serial_plan_stops_at_first_failure() {
        let mut t = task("p", "true", ".");
        t.resolved.plan.steps = vec![
            Step::Shell {
                script: "echo one".into(),
                cwd: ".".into(),
                env: BTreeMap::new(),
            },
            Step::Shell {
                script: "exit 3".into(),
                cwd: ".".into(),
                env: BTreeMap::new(),
            },
            Step::Shell {
                script: "echo never".into(),
                cwd: ".".into(),
                env: BTreeMap::new(),
            },
        ];
        t.env = Arc::new(BTreeMap::from([(
            "PATH".into(),
            std::env::var("PATH").unwrap_or_default(),
        )]));
        let out = run_task(Path::new("/"), &t, "h").await.unwrap();
        assert!(!out.success);
        assert!(out.output.contains("one"));
        assert!(!out.output.contains("never"));
    }
}
