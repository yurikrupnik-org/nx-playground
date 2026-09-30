//! `taskgraph` — drill into a go-task Taskfile and run it observably.
//!
//! Reads the Taskfile go-task would use (or `--taskfile`), resolves includes
//! and namespaces, and shows tasks, dependencies both ways, history and
//! estimates. `run` executes through the real `task` binary and publishes each
//! execution fact to the `TASKGRAPH` JetStream stream (consumed by
//! `taskgraph_api`) plus OTLP spans when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//!
//! NATS resolution: `--nats-url` > `$NATS_URL` > `nats://localhost:4222`.
//! Unreachable NATS degrades to an offline graph browser; `--offline` (or a
//! truthy `$TASKGRAPH_OFFLINE`) skips it. `$TASKGRAPH_EVENTS_OUT` makes `run`
//! append every event to a JSONL file as well, with or without NATS.
//!
//! `taskgraph shim ARGS…` is a drop-in for `task`; when `shim` is the first
//! argument everything after it is go-task's, never parsed as our flags.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use contract_taskgraph::{EventBody, TaskgraphEvent};
use core_authorship::is_truthy;
use core_config::{app_info, env_or_default};
use domain_taskgraph::{EventPublisher, GraphIndex, NatsPublisher};
use eyre::{Result, WrapErr, bail, eyre};
use uuid::Uuid;

mod context;
mod ingest;
mod render;
mod run;
mod shell_scan;
mod shim;

use context::{Ctx, history};

#[derive(Parser)]
#[command(
    name = "taskgraph",
    version,
    about = "Drill into a go-task Taskfile: tasks, dependencies, estimates, runs, events and traces"
)]
struct Cli {
    /// Taskfile to read (default: the one go-task would find from here)
    #[arg(long, short = 't', global = true)]
    taskfile: Option<PathBuf>,

    /// NATS server URL (default: $NATS_URL, else nats://localhost:4222)
    #[arg(long, global = true)]
    nats_url: Option<String>,

    /// Never connect to NATS: no events published, no history shown
    /// (also: a truthy $TASKGRAPH_OFFLINE)
    #[arg(long, global = true)]
    offline: bool,

    /// Trace link template for run output, e.g. http://localhost:16686/trace/{trace_id}
    /// (default: $TASKGRAPH_TRACE_URL)
    #[arg(long, global = true)]
    trace_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Tree,
    Mermaid,
    Dot,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Every task with its direct dependencies and history
    List {
        /// Include internal tasks
        #[arg(long)]
        all: bool,
    },
    /// Dependency tree of one task, or of every entry point
    Graph {
        task: Option<String>,
        /// Levels below the root to expand
        #[arg(long, default_value_t = 8)]
        depth: usize,
        #[arg(long, value_enum, default_value_t = Format::Tree)]
        format: Format,
    },
    /// Everything about one task: definition, deps both ways, history, estimate
    Show {
        task: String,
        #[arg(long, default_value_t = 3)]
        depth: usize,
        #[arg(long)]
        json: bool,
    },
    /// Expected duration from recorded history, recombined along the graph
    Estimate {
        task: String,
        #[arg(long)]
        json: bool,
    },
    /// Run a task through go-task, publishing events and spans
    Run {
        task: String,
        /// Forward go-task's verbose lifecycle lines too
        #[arg(long, short = 'v')]
        verbose: bool,
        /// List each execution's commands in the summary
        #[arg(long)]
        commands: bool,
        /// go-task binary (default: $TASKGRAPH_TASK_BIN, else `task`)
        #[arg(long)]
        task_bin: Option<String>,
        /// Passed to go-task after `--` (CLI_ARGS)
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Recorded runs, or one run's execution tree (id or unique id prefix)
    Runs {
        run: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Only runs of this Taskfile (default: all Taskfiles)
        #[arg(long)]
        this: bool,
        #[arg(long)]
        commands: bool,
        #[arg(long)]
        json: bool,
    },
    /// Publish the parsed graph without running anything
    Publish,
    /// Drop-in for `task`: plain task names run observed (in order, stopping
    /// at the first failure); anything else is exec'd to the real go-task
    /// ($TASKGRAPH_TASK_BIN, which must not be the shim itself)
    #[command(disable_help_flag = true)]
    Shim {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// ShellCheck every tracked script and every Taskfile cmd/status/precondition
    ShellScan {
        /// Write the ShellScan JSON here (default: stdout)
        #[arg(long)]
        out: Option<PathBuf>,
        /// ShellCheck binary
        #[arg(long, default_value = "shellcheck")]
        shellcheck: String,
    },
    /// Publish recorded JSONL events ($TASKGRAPH_EVENTS_OUT files) to NATS
    Ingest {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let _tracing = core_config::tracing::init_cli_tracing(
        app_info!(),
        env!("CARGO_CRATE_NAME"),
        "TASKGRAPH_LOG",
    );
    match dispatch(parse_cli()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("taskgraph: {e:?}");
            ExitCode::FAILURE
        }
    }
}

/// `taskgraph shim …` hands every later argument to go-task verbatim, even
/// ones that look like our global flags (`task --offline x` is go-task's
/// `--offline`), so it bypasses clap when it comes first.
fn parse_cli() -> Cli {
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) != Some("shim") {
        return Cli::parse();
    }
    Cli {
        taskfile: None,
        nats_url: None,
        offline: false,
        trace_url: None,
        command: Command::Shim {
            args: argv.split_off(2),
        },
    }
}

async fn dispatch(cli: Cli) -> Result<ExitCode> {
    let nats_url = cli
        .nats_url
        .unwrap_or_else(|| env_or_default("NATS_URL", "nats://localhost:4222"));
    let offline = cli.offline || std::env::var("TASKGRAPH_OFFLINE").is_ok_and(|v| is_truthy(&v));
    let trace_url = cli
        .trace_url
        .or_else(|| std::env::var("TASKGRAPH_TRACE_URL").ok());
    let default_task_bin = || env_or_default("TASKGRAPH_TASK_BIN", "task");

    // These two work without a Taskfile.
    let command = match cli.command {
        Command::Ingest { files } => {
            if offline {
                bail!("ingest publishes to NATS; drop --offline / unset TASKGRAPH_OFFLINE");
            }
            return ingest::ingest(&nats_url, &files).await;
        }
        Command::Shim { args } => {
            let ctx = Ctx::new(cli.taskfile, nats_url, offline).ok();
            return shim::shim(
                ctx.as_ref(),
                shim::ShimOptions {
                    args,
                    task_bin: default_task_bin(),
                    trace_url,
                },
            )
            .await;
        }
        command => command,
    };
    let ctx = Ctx::new(cli.taskfile, nats_url, offline)?;

    match command {
        Command::List { all } => {
            let graph = ctx.parse()?;
            let js = ctx.jetstream().await;
            let history = history(js.as_ref(), &graph).await;
            let rows: Vec<Vec<String>> = graph
                .tasks
                .iter()
                .filter(|t| all || !t.internal)
                .map(|t| {
                    let stats = history.stats(&graph.id, &t.name);
                    vec![
                        t.name.clone(),
                        t.deps.len().to_string(),
                        t.calls.len().to_string(),
                        render::opt_ms(stats.p50_ms),
                        stats.runs.to_string(),
                        t.desc.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            print!(
                "{}",
                render::table(&["TASK", "DEPS", "CALLS", "P50", "RUNS", "DESC"], &rows)
            );
            print_warnings(&graph.warnings);
        }
        Command::Graph {
            task,
            depth,
            format,
        } => {
            let graph = ctx.parse()?;
            let index = GraphIndex::new(&graph);
            let roots: Vec<String> = match &task {
                Some(t) => vec![index.resolve(t).ok_or_else(|| unknown_task(t))?.to_string()],
                None => index.roots().into_iter().map(str::to_string).collect(),
            };
            let focus = task.as_ref().map(|_| {
                let mut set: BTreeSet<String> = roots
                    .iter()
                    .flat_map(|r| index.closure(r))
                    .map(str::to_string)
                    .collect();
                set.extend(roots.iter().cloned());
                set
            });
            match format {
                Format::Tree => {
                    let js = ctx.jetstream().await;
                    let history = history(js.as_ref(), &graph).await;
                    let note = |t: &str| render::stats_note(&history.stats(&graph.id, t));
                    for root in &roots {
                        if let Some(tree) = index.tree(root, depth) {
                            print!("{}", render::tree(&tree, &note));
                        }
                    }
                    print_warnings(&graph.warnings);
                }
                Format::Mermaid => print!("{}", render::mermaid(&graph, focus.as_ref())),
                Format::Dot => print!("{}", render::dot(&graph, focus.as_ref())),
                Format::Json => {
                    let trees: Vec<_> = roots.iter().filter_map(|r| index.tree(r, depth)).collect();
                    println!("{}", serde_json::to_string_pretty(&trees)?);
                }
            }
        }
        Command::Show { task, depth, json } => {
            let graph = ctx.parse()?;
            let js = ctx.jetstream().await;
            let history = history(js.as_ref(), &graph).await;
            let view = history
                .task(&graph.id, &task, depth)
                .ok_or_else(|| unknown_task(&task))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                let note = |t: &str| render::stats_note(&history.stats(&graph.id, t));
                print!("{}", render::task_view(&view, &note));
            }
        }
        Command::Estimate { task, json } => {
            let graph = ctx.parse()?;
            let js = ctx.require_jetstream().await?;
            let history = history(Some(&js), &graph).await;
            let est = history
                .estimate(&graph.id, &task)
                .ok_or_else(|| unknown_task(&task))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&est)?);
            } else {
                print!("{}", render::estimate_line(&est));
                let index = GraphIndex::new(&graph);
                let mut tasks = vec![est.target.clone()];
                tasks.extend(
                    index
                        .closure(&est.target)
                        .into_iter()
                        .rev()
                        .map(str::to_string),
                );
                let rows: Vec<Vec<String>> = tasks
                    .iter()
                    .map(|t| {
                        let s = history.stats(&graph.id, t);
                        vec![
                            t.clone(),
                            s.runs.to_string(),
                            render::opt_ms(s.self_p50_ms),
                            render::opt_ms(s.self_p90_ms),
                            render::opt_ms(s.p50_ms),
                        ]
                    })
                    .collect();
                print!(
                    "\n{}",
                    render::table(
                        &["TASK", "RUNS", "SELF P50", "SELF P90", "TOTAL P50"],
                        &rows
                    )
                );
            }
        }
        Command::Run {
            task,
            verbose,
            commands,
            task_bin,
            args,
        } => {
            let code = run::run(
                &ctx,
                run::RunOptions {
                    target: task,
                    args,
                    verbose,
                    task_bin: task_bin.unwrap_or_else(default_task_bin),
                    commands,
                    trace_url,
                    shim_depth: None,
                },
            )
            .await?;
            return Ok(ExitCode::from(code));
        }
        Command::Runs {
            run,
            limit,
            this,
            commands,
            json,
        } => {
            let graph = ctx.parse()?;
            let js = ctx.require_jetstream().await?;
            let history = history(Some(&js), &graph).await;
            match run {
                None => {
                    let runs = history.runs(this.then_some(graph.id.as_str()), limit);
                    if json {
                        println!("{}", serde_json::to_string_pretty(&runs)?);
                    } else {
                        print!("{}", render::runs_table(&runs));
                    }
                }
                Some(prefix) => {
                    let matches: Vec<Uuid> = history
                        .runs(None, usize::MAX)
                        .into_iter()
                        .map(|r| r.run_id)
                        .filter(|id| id.to_string().starts_with(&prefix))
                        .collect();
                    let id = match matches.as_slice() {
                        [id] => *id,
                        [] => bail!("no retained run matches {prefix:?}"),
                        _ => bail!(
                            "{prefix:?} matches {} runs; give more of the id",
                            matches.len()
                        ),
                    };
                    let run = history.run(id).ok_or_else(|| eyre!("run {id} vanished"))?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(run)?);
                    } else {
                        print!(
                            "{}",
                            render::run_detail(run, commands, trace_url.as_deref())
                        );
                    }
                }
            }
        }
        Command::Publish => {
            let graph = ctx.parse()?;
            let js = ctx.require_jetstream().await?;
            let publisher = NatsPublisher::new(js).await?;
            let event = TaskgraphEvent::new(
                EventBody::GraphPublished {
                    graph: graph.clone(),
                },
                chrono::Utc::now(),
                None,
            );
            publisher
                .publish(&event)
                .await
                .wrap_err("publishing graph")?;
            println!(
                "published {} ({} tasks, digest {}, id {})",
                graph.taskfile,
                graph.tasks.len(),
                graph.digest,
                graph.id
            );
            print_warnings(&graph.warnings);
        }
        Command::ShellScan { out, shellcheck } => {
            shell_scan::shell_scan(&ctx, &shell_scan::ScanOptions { out, shellcheck })?;
        }
        Command::Shim { .. } | Command::Ingest { .. } => unreachable!("dispatched above"),
    }
    Ok(ExitCode::SUCCESS)
}

fn unknown_task(task: &str) -> eyre::Report {
    eyre!("task {task:?} is not in the parsed graph (see `taskgraph list --all`)")
}

fn print_warnings(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}
