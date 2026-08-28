//! butler — monorepo task compiler.
//!
//! Reads the workspace's nx-compatible configuration (nx.json, project.json,
//! package.json workspaces, Cargo workspace) but owns the graph, hashing,
//! caching, and execution itself. Coexists with nx: targets butler cannot run
//! (e.g. container executors) fail loudly and stay on nx.

mod cache;
mod config;
mod discovery;
mod git;
mod graph;
mod hash;
mod runner;

use std::collections::{BTreeMap, BTreeSet};

use clap::{Parser, Subcommand};
use eyre::{bail, Result};

use crate::graph::TaskId;

#[derive(Parser)]
#[command(
    name = "butler",
    version,
    about = "Monorepo task compiler (nx-config compatible)"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List discovered projects.
    Projects {
        #[arg(long)]
        json: bool,
    },
    /// Print the project dependency graph.
    Graph {
        #[arg(long)]
        json: bool,
    },
    /// List projects affected relative to a base ref.
    Affected {
        /// Base ref for merge-base comparison.
        #[arg(long, default_value = "origin/main")]
        base: String,
        #[arg(long)]
        json: bool,
    },
    /// Compile and execute a target across projects.
    Run {
        /// Target name (build, test, lint, ...).
        target: String,
        /// Comma-separated project names.
        #[arg(short, long, value_delimiter = ',')]
        projects: Vec<String>,
        /// Run for every project that has the target.
        #[arg(long)]
        all: bool,
        /// Run for projects affected since --base.
        #[arg(long)]
        affected: bool,
        /// Base ref for --affected.
        #[arg(long, default_value = "origin/main")]
        base: String,
        /// Named configuration (e.g. production, ci).
        #[arg(short, long)]
        configuration: Option<String>,
        /// Max concurrent tasks (cargo batches count as one).
        #[arg(long)]
        parallel: Option<usize>,
        /// Bypass the cache for this run.
        #[arg(long)]
        no_cache: bool,
        /// Print the compiled plan without executing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = discovery::find_workspace_root()?;
    let nx = config::NxJson::load(&root)?;
    let graph = discover_with_defaults(&root, &nx)?;

    match cli.command {
        Cmd::Projects { json } => {
            if json {
                let list: Vec<_> = graph
                    .projects
                    .values()
                    .map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "root": p.root,
                            "projectType": p.project_type,
                            "tags": p.tags,
                            "targets": p.targets.keys().collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                for p in graph.projects.values() {
                    println!("{}", p.name);
                }
            }
        }
        Cmd::Graph { json } => {
            let adjacency: BTreeMap<&str, &BTreeSet<String>> = graph
                .projects
                .values()
                .map(|p| (p.name.as_str(), &p.deps))
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&adjacency)?);
            } else {
                for (name, deps) in adjacency {
                    if deps.is_empty() {
                        println!("{name}");
                    } else {
                        println!(
                            "{name} -> {}",
                            deps.iter().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                }
            }
        }
        Cmd::Affected { base, json } => {
            let affected = affected_projects(&root, &graph, &base)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&affected)?);
            } else {
                for name in affected {
                    println!("{name}");
                }
            }
        }
        Cmd::Run {
            target,
            projects,
            all,
            affected,
            base,
            configuration,
            parallel,
            no_cache,
            dry_run,
        } => {
            let selected: Vec<String> = if !projects.is_empty() {
                for p in &projects {
                    graph.get(p)?;
                }
                projects
            } else if affected {
                affected_projects(&root, &graph, &base)?
                    .into_iter()
                    .collect()
            } else if all {
                graph.projects.keys().cloned().collect()
            } else {
                bail!("select projects with --projects, --affected, or --all");
            };

            let seeds: Vec<(String, String)> = selected
                .iter()
                .filter(|p| {
                    graph
                        .projects
                        .get(*p)
                        .is_some_and(|proj| proj.targets.contains_key(&target))
                })
                .map(|p| (p.clone(), target.clone()))
                .collect();
            if seeds.is_empty() {
                println!("no selected project has target `{target}`; nothing to do");
                return Ok(());
            }

            let tasks = graph::build_task_graph(&graph, &seeds, configuration.as_deref())?;

            let named_inputs = nx.named_inputs()?;
            let files = git::ls_files(&root)?;
            let mut hasher = hash::Hasher::new(&root, &graph, &named_inputs, files);
            let mut hashes: BTreeMap<TaskId, String> = BTreeMap::new();
            for task in &tasks {
                hashes.insert(
                    task.id.clone(),
                    hasher.task_hash(task, configuration.as_deref())?,
                );
            }

            let opts = runner::RunOptions {
                parallel: parallel.unwrap_or_else(|| {
                    std::thread::available_parallelism().map_or(4, |n| n.get().min(8))
                }),
                no_cache,
                dry_run,
            };
            let n_tasks = tasks.len();
            let runtime = tokio::runtime::Runtime::new()?;
            let reports = runtime.block_on(runner::execute(&root, tasks, hashes, opts))?;

            if dry_run {
                return Ok(());
            }
            let count = |o: runner::Outcome| reports.iter().filter(|r| r.outcome == o).count();
            let failed = count(runner::Outcome::Failed);
            let skipped = count(runner::Outcome::Skipped);
            println!(
                "\n{} tasks: {} ok, {} from cache, {} failed, {} skipped",
                n_tasks,
                count(runner::Outcome::Success),
                count(runner::Outcome::Cached),
                failed,
                skipped,
            );
            for r in &reports {
                if r.outcome == runner::Outcome::Failed {
                    println!("  failed: {}", r.id);
                }
            }
            if failed > 0 {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Discover the graph, then merge nx.json targetDefaults into every target.
fn discover_with_defaults(
    root: &std::path::Path,
    nx: &config::NxJson,
) -> Result<graph::ProjectGraph> {
    let mut g = discovery::discover(root)?;
    for project in g.projects.values_mut() {
        for (name, target) in project.targets.iter_mut() {
            *target = target.merged_over(nx.target_defaults.get(name));
        }
    }
    Ok(g)
}

fn affected_projects(
    root: &std::path::Path,
    graph: &graph::ProjectGraph,
    base: &str,
) -> Result<BTreeSet<String>> {
    let changed = git::changed_files(root, base)?;
    let mut seeds: BTreeSet<String> = BTreeSet::new();
    let mut global_change = false;
    for file in &changed {
        match graph.project_for_file(file) {
            Some(p) => {
                seeds.insert(p.name.clone());
            }
            // Workspace-level config affects everything.
            None => {
                if matches!(
                    file.as_str(),
                    "nx.json" | "package.json" | "Cargo.toml" | "bun.lock" | "Cargo.lock"
                ) {
                    global_change = true;
                }
            }
        }
    }
    if global_change {
        return Ok(graph.projects.keys().cloned().collect());
    }
    Ok(graph.with_dependents(&seeds))
}
