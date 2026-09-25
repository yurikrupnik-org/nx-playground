//! butler — monorepo task compiler.
//!
//! Reads the workspace's nx-compatible configuration (nx.json, project.json,
//! package.json workspaces) and infers the rest of the project graph itself
//! (see [`infer`]), then owns hashing, caching, and execution. Coexists with
//! nx: `butler graph verify` proves the two graphs agree, and targets butler
//! cannot run fail loudly and stay on nx.

mod affected;
mod cache;
mod config;
mod container;
mod discovery;
mod executor;
mod git;
mod graph;
mod hash;
mod infer;
mod k8s;
mod nxgraph;
mod runner;
mod settings;
mod task;
mod tilt;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use clap::{Args, CommandFactory, Parser, Subcommand};
use eyre::{Result, bail};

use crate::task::{Overrides, TaskId};

#[derive(Parser)]
#[command(
    name = "butler",
    version,
    about = "Monorepo task compiler (nx-config compatible)"
)]
struct Cli {
    /// Where the repo's project-graph inference comes from, overriding
    /// `butler.toml` `[graph] infer`: `native` for the built-in Rust port, or a
    /// command (split on whitespace) speaking nx's plugin JSON, e.g.
    /// `--infer 'bun tools/nx/infer.ts'`.
    #[arg(long, global = true, value_name = "native|COMMAND")]
    infer: Option<String>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run targets for many projects (`nx run-many`). Unknown flags and
    /// arguments are task overrides, handed to the executors as nx does
    /// (`butler run-many -t format -p tag:lang:python --check`).
    RunMany {
        /// Targets to run (space or comma separated).
        #[arg(short = 't', long = "targets", visible_alias = "target", num_args = 1.., value_delimiter = ',', required = true)]
        targets: Vec<String>,
        /// Projects: names, globs, `tag:x`, `directory:x`, `!` exclusions
        /// (space or comma separated). Default: every project with a target.
        #[arg(short = 'p', long, num_args = 1.., value_delimiter = ',')]
        projects: Vec<String>,
        /// Accepted for nx compatibility; running every project is the
        /// default.
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Run targets for the projects a change affects (`nx affected`); with
    /// no `-t`, list those projects.
    Affected {
        /// Targets to run (space or comma separated).
        #[arg(short = 't', long = "targets", visible_alias = "target", num_args = 1.., value_delimiter = ',')]
        targets: Vec<String>,
        #[command(flatten)]
        range: RangeArgs,
        /// With no `-t`: print the affected projects as a JSON array.
        #[arg(long, conflicts_with = "targets")]
        json: bool,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Show projects (`nx show projects`) or one project (`nx show project`).
    Show {
        #[command(subcommand)]
        command: ShowCmd,
    },
    /// Print the project graph (`--json`: the `nx graph --file` shape).
    Graph {
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<GraphCmd>,
    },
    /// Generate Tiltfiles from butler.toml + the project graph.
    Tilt {
        #[command(subcommand)]
        command: TiltCmd,
    },
    /// Generate the k8s values files and rendered manifests from butler.toml.
    K8s {
        #[command(subcommand)]
        command: K8sCmd,
    },
}

#[derive(Subcommand)]
enum TiltCmd {
    /// Write the root Tiltfile and one Tiltfile per app.
    Gen {
        /// Write into this directory instead of the workspace root.
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
        /// Fail instead of writing when the on-disk Tiltfiles have drifted.
        #[arg(long)]
        check: bool,
        /// Write only this app's Tiltfile (workspace-relative directory).
        /// This is what the nx plugin's per-project `tilt-gen` target runs.
        #[arg(long, value_name = "DIR", conflicts_with = "root")]
        app: Option<String>,
        /// Write only the root Tiltfile.
        #[arg(long)]
        root: bool,
        /// Authoritative app list (comma-separated workspace-relative dirs),
        /// overriding butler's own discovery. The nx plugin passes the set nx
        /// inferred, so the root Tiltfile's includes cannot disagree with nx.
        #[arg(long, value_delimiter = ',', value_name = "DIRS")]
        apps: Vec<String>,
    },
}

#[derive(Subcommand)]
enum K8sCmd {
    /// Write each app's `k8s/values*.yaml`, its rendered manifest under
    /// `manifests/k8s/apps/`, and the aggregate kustomization.
    Gen {
        /// Fail instead of writing when the on-disk artifacts have drifted.
        #[arg(long)]
        check: bool,
        /// Generate only this app (workspace-relative directory). This is what
        /// the nx plugin's per-project target runs.
        #[arg(long, value_name = "DIR", conflicts_with = "root")]
        app: Option<String>,
        /// Generate only the aggregate kustomization.
        #[arg(long)]
        root: bool,
        /// Authoritative app list (comma-separated workspace-relative dirs),
        /// overriding butler's own discovery. The nx plugin passes the set nx
        /// inferred, so the kustomization cannot disagree with nx.
        #[arg(long, value_delimiter = ',', value_name = "DIRS")]
        apps: Vec<String>,
    },
}

#[derive(Subcommand)]
enum GraphCmd {
    /// Diff butler's graph against an `nx graph --file` dump: every project,
    /// target (executor, options, inputs, outputs, dependsOn, cache) and edge
    /// must agree, or two inference implementations have drifted.
    Verify {
        /// JSON dump produced by `nx graph --file <path>`.
        #[arg(long, value_name = "FILE")]
        graph: std::path::PathBuf,
    },
}

/// Options shared by the commands that run tasks (nx `withRunOptions`).
#[derive(Args)]
struct RunArgs {
    /// Projects to leave out (same patterns as `-p`).
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    exclude: Vec<String>,
    /// Max concurrent tasks (a cargo batch counts as one): a number, a
    /// percentage of the cores, or `false` (= 1). Bare `--parallel` means
    /// `NX_PARALLEL` or 3; absent means `NX_PARALLEL`, else nx.json
    /// `parallel`, else 3 — nx's defaults.
    #[arg(long, num_args = 0..=1, default_missing_value = "true", value_name = "N")]
    parallel: Option<String>,
    /// Named configuration (applied where a target defines it).
    #[arg(short = 'c', long)]
    configuration: Option<String>,
    /// Do not read the cache (results are still stored), like nx's
    /// `--skip-nx-cache`.
    #[arg(long)]
    skip_cache: bool,
    /// Print the task graph and every task's plan without running anything.
    #[arg(long)]
    dry_run: bool,
    /// Run only the requested tasks, not what they `dependsOn`.
    #[arg(long)]
    exclude_task_dependencies: bool,
    /// Task overrides: every argument butler does not recognize, in order.
    #[arg(last = true, hide = true)]
    overrides: Vec<String>,
}

/// What changed (nx's affected options). Without `--base`, `NX_BASE`, then
/// nx.json `defaultBase`, then `main`; without `--head`, `NX_HEAD`, else the
/// working tree (committed, uncommitted and untracked changes).
#[derive(Args)]
struct RangeArgs {
    #[arg(long)]
    base: Option<String>,
    #[arg(long)]
    head: Option<String>,
    /// Use exactly these changed files.
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    files: Vec<String>,
    /// Only uncommitted changes.
    #[arg(long)]
    uncommitted: bool,
    /// Only untracked files.
    #[arg(long)]
    untracked: bool,
}

impl RangeArgs {
    fn range(&self) -> affected::Range {
        affected::Range {
            base: self.base.clone(),
            head: self.head.clone(),
            files: self.files.clone(),
            uncommitted: self.uncommitted,
            untracked: self.untracked,
        }
    }
}

#[derive(Subcommand)]
enum ShowCmd {
    /// List projects (`nx show projects`), filters applied in nx's order:
    /// affected, type, `-p`, `--with-target`, `--exclude`.
    Projects {
        /// Only projects with any of these targets.
        #[arg(long = "with-target", num_args = 1.., value_delimiter = ',')]
        with_target: Vec<String>,
        /// Only projects affected by the change range.
        #[arg(long)]
        affected: bool,
        #[command(flatten)]
        range: RangeArgs,
        /// Project patterns (names, globs, `tag:x`, `directory:x`, `!`).
        #[arg(short = 'p', long, num_args = 1.., value_delimiter = ',')]
        projects: Vec<String>,
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        exclude: Vec<String>,
        /// Only projects of this nx node type.
        #[arg(long = "type", value_parser = ["app", "lib", "e2e"])]
        project_type: Option<String>,
        /// Print a JSON array.
        #[arg(long)]
        json: bool,
        /// Print on one line, joined by this separator.
        #[arg(long)]
        sep: Option<String>,
    },
    /// Print one project's configuration as nx's graph has it.
    Project {
        name: String,
        /// Accepted for nx compatibility; the output is always JSON.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse_from(split_overrides(std::env::args().collect()));
    let root = discovery::find_workspace_root()?;
    let nx = config::NxJson::load(&root)?;
    let settings = if root.join(settings::FILE).exists() {
        Some(settings::Root::load(&root)?)
    } else {
        None
    };
    let overrides = match &settings {
        Some(s) => Some(settings::load_app_overrides(&root, s)?),
        None => None,
    };
    let inference = match cli.infer.as_deref().map(str::trim) {
        Some("native") => discovery::Inference::Native,
        Some(cmd) => {
            discovery::Inference::External(cmd.split_whitespace().map(String::from).collect())
        }
        None => match settings.as_ref().and_then(|s| s.graph.infer.clone()) {
            Some(argv) => discovery::Inference::External(argv),
            None => discovery::Inference::Native,
        },
    };
    let files = git::ls_files(&root)?;
    let graph = discovery::discover(
        &root,
        &files,
        &nx,
        settings.as_ref(),
        overrides.as_ref(),
        &inference,
    )?;
    // Only the generators need `butler.toml`; everything else works without.
    let require_settings = || -> Result<(&settings::Root, &settings::AppOverrides)> {
        match (&settings, &overrides) {
            (Some(s), Some(o)) => Ok((s, o)),
            _ => bail!(
                "this command needs a {} at {}",
                settings::FILE,
                root.display()
            ),
        }
    };

    match cli.command {
        Cmd::RunMany {
            targets,
            projects,
            all: _,
            run,
        } => {
            let selected = run_many_projects(&graph, &targets, &projects, &run.exclude)?;
            run_tasks(&root, &graph, &nx, &selected, &targets, run)?;
        }
        Cmd::Affected {
            targets,
            range,
            json,
            run,
        } => {
            let touched = affected::touched_files(&root, &range.range())?;
            let affected =
                affected::affected_projects(&root, &graph, &nx, &touched, &range.range())?;
            if targets.is_empty() {
                print_names(affected.iter(), json, None)?;
            } else {
                let excluded: BTreeSet<String> = task::find_matching(&graph, &run.exclude)?
                    .into_iter()
                    .collect();
                let selected: Vec<String> = affected
                    .into_iter()
                    .filter(|p| {
                        graph.projects[p]
                            .targets
                            .keys()
                            .any(|t| targets.contains(t))
                            && !excluded.contains(p)
                    })
                    .collect();
                run_tasks(&root, &graph, &nx, &selected, &targets, run)?;
            }
        }
        Cmd::Show {
            command:
                ShowCmd::Projects {
                    with_target,
                    affected,
                    range,
                    projects,
                    exclude,
                    project_type,
                    json,
                    sep,
                },
        } => {
            let mut names: BTreeSet<String> = if affected {
                let touched = affected::touched_files(&root, &range.range())?;
                affected::affected_projects(&root, &graph, &nx, &touched, &range.range())?
            } else {
                graph.projects.keys().cloned().collect()
            };
            if let Some(ty) = &project_type {
                names.retain(|n| node_type(&root, &graph.projects[n]) == ty);
            }
            if !projects.is_empty() {
                let matched: BTreeSet<String> = task::find_matching(&graph, &projects)?
                    .into_iter()
                    .collect();
                names.retain(|n| matched.contains(n));
            }
            if !with_target.is_empty() {
                names.retain(|n| {
                    graph.projects[n]
                        .targets
                        .keys()
                        .any(|t| with_target.contains(t))
                });
            }
            if !exclude.is_empty() {
                let excluded: BTreeSet<String> =
                    task::find_matching(&graph, &exclude)?.into_iter().collect();
                names.retain(|n| !excluded.contains(n));
            }
            print_names(names.iter(), json, sep.as_deref())?;
        }
        Cmd::Show {
            command: ShowCmd::Project { name, json: _ },
        } => {
            graph.get(&name)?;
            let dump = nxgraph::to_nx_dump(&graph)?;
            let mut data = dump["graph"]["nodes"][&name]["data"].clone();
            data["name"] = serde_json::Value::String(name);
            println!("{}", serde_json::to_string(&data)?);
        }
        Cmd::Graph { json, command } => match command {
            Some(GraphCmd::Verify { graph: dump }) => {
                let ignore = settings
                    .as_ref()
                    .map(|s| s.graph.verify_ignore_targets.clone())
                    .unwrap_or_default();
                nxgraph::verify(&graph, &dump, &ignore)?;
            }
            None if json => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&nxgraph::to_nx_dump(&graph)?)?
                );
            }
            None => {
                for p in graph.projects.values() {
                    if p.deps.is_empty() {
                        println!("{}", p.name);
                    } else {
                        println!(
                            "{} -> {}",
                            p.name,
                            p.deps.iter().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                }
            }
        },
        Cmd::Tilt {
            command:
                TiltCmd::Gen {
                    out_dir,
                    check,
                    app,
                    root: root_only,
                    apps,
                },
        } => {
            let (settings, overrides) = require_settings()?;
            let selection = tilt::Selection {
                app: app.as_deref(),
                root_only,
                apps: &apps,
            };
            let files = tilt::generate(&root, &graph, settings, overrides, &selection)?;
            let out = out_dir.unwrap_or_else(|| root.clone());
            if check {
                let drifted = tilt::check_files(&out, &files);
                if !drifted.is_empty() {
                    for d in &drifted {
                        println!("drift: {d}");
                    }
                    bail!(
                        "{} of {} Tiltfiles are stale; run `butler tilt gen`",
                        drifted.len(),
                        files.len()
                    );
                }
                println!("{} Tiltfiles up to date", files.len());
            } else {
                tilt::write_files(&out, &files)?;
            }
        }
        Cmd::K8s {
            command:
                K8sCmd::Gen {
                    check,
                    app,
                    root: root_only,
                    apps,
                },
        } => {
            let (settings, overrides) = require_settings()?;
            let selection = k8s::Selection {
                app: app.as_deref(),
                root_only,
                apps: &apps,
            };
            let files = k8s::generate(&root, &graph, settings, overrides, &selection)?;
            if check {
                // Same drift gate as the Tiltfiles: the generated artifacts are
                // committed, so a stale one is a review-time failure, not a
                // surprise at deploy time.
                let drifted = tilt::check_files(&root, &files);
                if !drifted.is_empty() {
                    for d in &drifted {
                        println!("drift: {d}");
                    }
                    bail!(
                        "{} of {} k8s artifacts are stale; run `butler k8s gen`",
                        drifted.len(),
                        files.len()
                    );
                }
                println!("{} k8s artifacts up to date", files.len());
            } else {
                tilt::write_files(&root, &files)?;
            }
        }
    }
    Ok(())
}

/// nx hands every argument it does not recognize to the tasks as overrides
/// (yargs `unknown-options-as-args`), wherever it appears. clap wants them
/// after `--`, so move them there, using clap's own view of which flags
/// `run-many` / `affected` know and how many values each takes.
fn split_overrides(args: Vec<String>) -> Vec<String> {
    let mut cmd = Cli::command();
    cmd.build();
    let mut i = 1;
    while i < args.len() && args[i].starts_with('-') {
        i += if args[i] == "--infer" { 2 } else { 1 };
    }
    let Some(name) = args.get(i) else {
        return args;
    };
    if name != "run-many" && name != "affected" {
        return args;
    }
    let Some(sub) = cmd.find_subcommand(name) else {
        return args;
    };
    let mut known = args[..=i].to_vec();
    let mut overrides = Vec::new();
    let mut j = i + 1;
    while j < args.len() {
        let tok = &args[j];
        if tok == "--" {
            overrides.extend(args[j + 1..].iter().cloned());
            break;
        }
        let found = if let Some(body) = tok.strip_prefix("--") {
            let (flag, inline) = body
                .split_once('=')
                .map_or((body, false), |(f, _)| (f, true));
            sub.get_arguments()
                .find(|a| {
                    a.get_long_and_visible_aliases()
                        .is_some_and(|ls| ls.contains(&flag))
                })
                .map(|a| (a, inline))
        } else if tok.len() > 1 && tok.starts_with('-') {
            let c = tok[1..].chars().next();
            sub.get_arguments()
                .find(|a| a.get_short().is_some() && a.get_short() == c)
                .map(|a| (a, tok.len() > 2))
        } else {
            None
        };
        j += 1;
        let Some((arg, inline)) = found else {
            overrides.push(tok.clone());
            continue;
        };
        known.push(tok.clone());
        if inline || !arg.get_action().takes_values() {
            continue;
        }
        let range = arg.get_num_args().unwrap_or_default();
        let mut n = 0;
        while n < range.max_values()
            && j < args.len()
            && args[j] != "--"
            && (n < range.min_values() || !args[j].starts_with('-'))
        {
            known.push(args[j].clone());
            j += 1;
            n += 1;
        }
    }
    if !overrides.is_empty() {
        known.push("--".into());
        known.extend(overrides);
    }
    known
}

/// nx `projectsToRun`: projects with any of the targets, narrowed by `-p`,
/// minus `--exclude`.
fn run_many_projects(
    graph: &graph::ProjectGraph,
    targets: &[String],
    patterns: &[String],
    exclude: &[String],
) -> Result<Vec<String>> {
    let runnable = |name: &str| {
        graph.projects[name]
            .targets
            .keys()
            .any(|t| targets.contains(t))
    };
    let mut selected: Vec<String> = if patterns.is_empty() {
        graph
            .projects
            .keys()
            .filter(|p| runnable(p))
            .cloned()
            .collect()
    } else {
        let (valid, invalid): (Vec<String>, Vec<String>) = task::find_matching(graph, patterns)?
            .into_iter()
            .partition(|p| runnable(p));
        if !invalid.is_empty() {
            eprintln!(
                "warning: these projects have none of the targets ({}): {}",
                targets.join(", "),
                invalid.join(", ")
            );
        }
        valid
    };
    let excluded: BTreeSet<String> = task::find_matching(graph, exclude)?.into_iter().collect();
    selected.retain(|p| !excluded.contains(p));
    Ok(selected)
}

/// Build the task graph for `projects` × `targets`, hash, and run it.
fn run_tasks(
    root: &Path,
    graph: &graph::ProjectGraph,
    nx: &config::NxJson,
    projects: &[String],
    targets: &[String],
    run: RunArgs,
) -> Result<()> {
    if projects.is_empty() {
        println!("No projects with target(s) {} to run", targets.join(", "));
        return Ok(());
    }
    let overrides = Overrides::from_args(&run.overrides);
    let base_env: BTreeMap<String, String> = std::env::vars().collect();
    let tasks = task::build(
        root,
        graph,
        &base_env,
        &task::Request {
            projects,
            targets,
            configuration: run.configuration.as_deref(),
            overrides: &overrides,
            exclude_task_dependencies: run.exclude_task_dependencies,
        },
    )?;

    let files = git::ls_files(root)?;
    let mut hasher = hash::Hasher::new(root, graph, &nx.named_inputs, files);
    let mut hashes: BTreeMap<TaskId, String> = BTreeMap::new();
    for task in &tasks {
        hashes.insert(task.id.clone(), hasher.task_hash(task)?);
    }

    let env_true = |k: &str| std::env::var(k).is_ok_and(|v| v == "true");
    let opts = runner::RunOptions {
        parallel: parallelism(root, run.parallel.as_deref())?,
        skip_cache: run.skip_cache
            || env_true("NX_SKIP_NX_CACHE")
            || env_true("NX_DISABLE_NX_CACHE"),
        dry_run: run.dry_run,
    };
    let n_tasks = tasks.len();
    let runtime = tokio::runtime::Runtime::new()?;
    let reports = runtime.block_on(runner::execute(root, tasks, hashes, opts))?;
    if run.dry_run {
        return Ok(());
    }
    let count = |o: runner::Outcome| reports.iter().filter(|r| r.outcome == o).count();
    let failed = count(runner::Outcome::Failed);
    println!(
        "\n{} tasks: {} ok, {} from cache, {} failed, {} skipped",
        n_tasks,
        count(runner::Outcome::Success),
        count(runner::Outcome::Cached),
        failed,
        count(runner::Outcome::Skipped),
    );
    for r in reports
        .iter()
        .filter(|r| r.outcome == runner::Outcome::Failed)
    {
        println!("  failed: {}", r.id);
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// nx `readParallelFromArgsAndEnv`, falling back to nx.json `parallel`, then 3.
fn parallelism(root: &Path, arg: Option<&str>) -> Result<usize> {
    let env = std::env::var("NX_PARALLEL").ok().filter(|v| !v.is_empty());
    let value = match arg {
        Some("false") => return Ok(1),
        Some("true" | "") => env.unwrap_or_else(|| "3".into()),
        Some(v) => v.to_string(),
        None => match env {
            Some(v) => v,
            None => {
                let raw = std::fs::read_to_string(root.join("nx.json")).unwrap_or_default();
                let nx: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
                return Ok(nx
                    .get("parallel")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(3, |p| p.max(1) as usize));
            }
        },
    };
    // nx `concurrency`: parseInt, or a percentage of the cores.
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    let n: usize = digits.parse().map_err(|_| {
        eyre::eyre!("--parallel: `{value}` is not a number, a percentage, or false")
    })?;
    let n = if value.ends_with('%') {
        std::thread::available_parallelism().map_or(1, |c| c.get()) * n / 100
    } else {
        n
    };
    Ok(n.max(1))
}

/// nx `getProjectType`: the graph node type `--type` filters on.
fn node_type(root: &Path, project: &graph::Project) -> &'static str {
    match project.project_type.as_deref() {
        Some("library") => return "lib",
        Some(_) if project.name.ends_with("-e2e") || project.name == "e2e" => return "e2e",
        Some(_) => return "app",
        None => {}
    }
    let dir = root.join(&project.root);
    if dir.join("tsconfig.lib.json").exists() {
        return "lib";
    }
    if dir.join("tsconfig.app.json").exists() {
        return "app";
    }
    let manifest: Option<serde_json::Value> = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    match manifest {
        Some(m)
            if ["exports", "main", "module", "bin"]
                .iter()
                .all(|k| m.get(*k).is_none_or(|v| v.is_null())) =>
        {
            "app"
        }
        _ => "lib",
    }
}

/// nx's `show projects` output: one per line, `--sep`-joined, or a JSON
/// array.
fn print_names<'a>(
    names: impl Iterator<Item = &'a String>,
    json: bool,
    sep: Option<&str>,
) -> Result<()> {
    let names: Vec<&String> = names.collect();
    if json {
        println!("{}", serde_json::to_string(&names)?);
    } else if let Some(sep) = sep {
        println!(
            "{}",
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(sep)
        );
    } else {
        for n in names {
            println!("{n}");
        }
    }
    Ok(())
}
