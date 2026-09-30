//! `taskgraph run`: execute through the real go-task binary and turn what it
//! reports into events, spans and a summary.
//!
//! go-task stays the executor — templating, `status:`/`sources:` checks,
//! `run: once`, prompts and exit codes are all its own. This process adds
//! `--verbose`, reads go-task's stderr line by line (stdout and stdin stay
//! attached to the terminal), forwards what the user would normally see, and
//! feeds every line to a [`Tracker`]. Each fact becomes:
//!
//! - an event on the `TASKGRAPH` stream and/or a line in the
//!   `$TASKGRAPH_EVENTS_OUT` JSONL file (published in order by one background
//!   task, so a slow broker never stalls the forwarding of output);
//! - a tracing span per execution, parented on the execution that pulled it in,
//!   exported over OTLP when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use contract_taskgraph::{EventBody, TaskOutcome, TaskgraphEvent, TraceRef};
use domain_taskgraph::observe::{self, Tracker};
use domain_taskgraph::origin::detect_origin;
use domain_taskgraph::{EventPublisher, FanOut, FilePublisher, GraphIndex, NatsPublisher};
use eyre::{Result, WrapErr, bail, eyre};
use opentelemetry::trace::TraceContextExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{Span, field, info, info_span, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::context::{Ctx, EVENTS_OUT_ENV, history};
use crate::render;

/// Upper bound on draining queued events after go-task exits.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct RunOptions {
    pub target: String,
    pub args: Vec<String>,
    pub verbose: bool,
    pub task_bin: String,
    pub commands: bool,
    pub trace_url: Option<String>,
    /// Set on go-task's environment when running for `taskgraph shim`.
    pub shim_depth: Option<u32>,
}

/// Runs `opts.target`; returns go-task's exit code (1 when it has none).
pub async fn run(ctx: &Ctx, opts: RunOptions) -> Result<u8> {
    let graph = ctx.parse()?;
    let index = GraphIndex::new(&graph);
    if index.resolve(&opts.target).is_none() {
        // Only an include we could not read can hide a task from the parse;
        // with none, go-task would just fail — say so without recording a run.
        let opaque: Vec<&str> = graph
            .includes
            .iter()
            .filter(|i| !i.resolved)
            .map(|i| i.taskfile.as_str())
            .collect();
        if opaque.is_empty() {
            bail!(
                "task {:?} is not in {} (see `taskgraph list --all`)",
                opts.target,
                graph.taskfile
            );
        }
        warn!(
            target = %opts.target,
            "not in the parsed graph; it can only come from an unresolved include ({}), so go-task decides",
            opaque.join(", ")
        );
    }

    let js = ctx.jetstream().await;
    let publisher = sinks(ctx, js.as_ref()).await;
    let history = history(js.as_ref(), &graph).await;
    // An estimate with no history behind any of its tasks is a 0 that means
    // "unknown"; publishing it would read as "expected to be instant".
    let estimate = history
        .estimate(&graph.id, &opts.target)
        .filter(|e| e.unknown.len() < e.tasks);
    if let Some(est) = &estimate {
        eprintln!(
            "taskgraph: {} expected ~{} (critical path {})",
            est.target,
            render::ms(est.expected_ms),
            est.critical_path.join(" → ")
        );
    }

    let run_id = Uuid::now_v7();
    let run_span = info_span!(
        "run",
        otel.name = %format!("task {}", opts.target),
        otel.status_code = field::Empty,
        run_id = %run_id,
        target = %opts.target,
        graph_id = %graph.id,
        exit_code = field::Empty,
    );

    let (tx, rx) = mpsc::unbounded_channel::<TaskgraphEvent>();
    let pump = tokio::spawn(pump(publisher, rx));
    // The local fold renders the post-run summary from exactly what was published.
    let mut local = history;
    let mut emit = |body: EventBody, span: &Span| {
        let event = TaskgraphEvent::new(body, Utc::now(), trace_ref(span));
        local.apply(&event);
        // The receiver lives until `tx` is dropped below.
        let _ = tx.send(event);
    };

    emit(
        EventBody::GraphPublished {
            graph: graph.clone(),
        },
        &run_span,
    );
    emit(
        EventBody::RunStarted {
            run_id,
            graph_id: graph.id.clone(),
            target: opts.target.clone(),
            args: opts.args.clone(),
            host: ctx.host.clone(),
            user: ctx.user.clone(),
            cwd: ctx.cwd.display().to_string(),
            estimate_ms: estimate.as_ref().map(|e| e.expected_ms),
            origin: detect_origin(|name| std::env::var(name).ok(), &ctx.cwd),
        },
        &run_span,
    );

    let mut command = Command::new(&opts.task_bin);
    command.arg("--verbose");
    if ctx.explicit_taskfile {
        command.arg("--taskfile").arg(&ctx.taskfile);
    }
    command.arg(&opts.target);
    if !opts.args.is_empty() {
        command.arg("--").args(&opts.args);
    }
    // Nested `task` calls inside this run append to the same file.
    if let Some(path) = &ctx.events_out {
        command.env(EVENTS_OUT_ENV, path);
    }
    if let Some(depth) = opts.shim_depth {
        command.env(crate::shim::DEPTH_ENV, depth.to_string());
    }
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped());
    let started_at = Utc::now();
    let mut child = command.spawn().wrap_err_with(|| {
        format!(
            "starting go-task ({}); is it installed and on PATH?",
            opts.task_bin
        )
    })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| eyre!("go-task stderr was not captured"))?;

    let mut tracker = Tracker::new(run_id, &index, &opts.target);
    let mut spans: HashMap<u32, Span> = HashMap::new();
    let mut lines = BufReader::new(stderr).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(raw) = line.wrap_err("reading go-task stderr")? else { break };
                let clean = observe::strip_ansi(&raw);
                let classified = observe::classify(&clean);
                if opts.verbose || classified.shown_without_verbose() {
                    eprintln!("{raw}");
                }
                for body in tracker.observe(&classified, Utc::now()) {
                    let span = span_for(&body, &run_span, &mut spans);
                    emit(body, &span);
                }
            }
            // The terminal delivers Ctrl-C to go-task too (same process
            // group); keep reading so its final lines and exit are recorded.
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    let status = child.wait().await.wrap_err("waiting for go-task")?;
    let exit_code = status.code();

    for body in tracker.finish(exit_code, started_at, Utc::now()) {
        let span = span_for(&body, &run_span, &mut spans);
        emit(body, &span);
    }
    match exit_code {
        Some(0) => {}
        _ => {
            run_span.record("otel.status_code", "error");
        }
    }
    if let Some(code) = exit_code {
        run_span.record("exit_code", code);
    }
    let trace_id = trace_ref(&run_span).map(|t| t.trace_id);
    drop(run_span);
    drop(tx);

    match tokio::time::timeout(FLUSH_TIMEOUT, pump).await {
        Ok(Ok(failures)) if failures > 0 => {
            warn!(failures, "some events were not published");
        }
        Ok(_) => {}
        Err(_) => warn!("timed out publishing queued events"),
    }

    if let Some(run) = local.run(run_id) {
        eprintln!();
        eprint!(
            "{}",
            render::run_detail(run, opts.commands, opts.trace_url.as_deref())
        );
    }
    if let Some(trace_id) = trace_id {
        info!(%trace_id, "trace exported");
    }

    Ok(match exit_code {
        Some(code) => u8::try_from(code).unwrap_or(1),
        None => 1,
    })
}

/// NATS when connected, the events file when configured, both when both —
/// and a no-op when neither. A sink that cannot be opened is warned about:
/// the run itself never fails over telemetry.
async fn sinks(ctx: &Ctx, js: Option<&async_nats::jetstream::Context>) -> Arc<dyn EventPublisher> {
    let mut sinks: Vec<Arc<dyn EventPublisher>> = Vec::new();
    if let Some(js) = js {
        match NatsPublisher::new(js.clone()).await {
            Ok(p) => sinks.push(Arc::new(p)),
            Err(e) => warn!(error = %e, "cannot publish to TASKGRAPH; NATS events disabled"),
        }
    }
    if let Some(path) = &ctx.events_out {
        match FilePublisher::open(path) {
            Ok(f) => sinks.push(Arc::new(f)),
            Err(e) => warn!(error = %e, "cannot open {EVENTS_OUT_ENV}; file events disabled"),
        }
    }
    Arc::new(FanOut::new(sinks))
}

/// Publish queued events in order; returns how many failed. The first failure
/// is warned about, the rest counted: the run itself must not fail over it.
async fn pump(
    publisher: Arc<dyn EventPublisher>,
    mut rx: mpsc::UnboundedReceiver<TaskgraphEvent>,
) -> usize {
    let mut failures = 0;
    while let Some(event) = rx.recv().await {
        if let Err(e) = publisher.publish(&event).await {
            if failures == 0 {
                warn!(error = %e, "publishing a TASKGRAPH event failed");
            }
            failures += 1;
        }
    }
    failures
}

/// The span an event belongs to: a new child span on start, the execution's
/// span for its commands, and the closing of that span on finish.
fn span_for(body: &EventBody, run_span: &Span, spans: &mut HashMap<u32, Span>) -> Span {
    match body {
        EventBody::TaskStarted {
            instance,
            task,
            parent,
            via,
            ..
        } => {
            let parent_span = parent
                .and_then(|p| spans.get(&p))
                .unwrap_or(run_span)
                .clone();
            let span = info_span!(
                parent: &parent_span,
                "task",
                otel.name = %task,
                otel.status_code = field::Empty,
                task = %task,
                instance = instance,
                via = ?via,
                outcome = field::Empty,
                duration_ms = field::Empty,
            );
            spans.insert(*instance, span.clone());
            span
        }
        EventBody::CommandStarted {
            instance, command, ..
        } => {
            let span = spans.get(instance).unwrap_or(run_span).clone();
            info!(parent: &span, command = %command, "command");
            span
        }
        EventBody::TaskFinished {
            instance,
            outcome,
            duration_ms,
            error,
            ..
        } => match spans.remove(instance) {
            Some(span) => {
                span.record("outcome", field::debug(outcome));
                span.record("duration_ms", duration_ms);
                if *outcome == TaskOutcome::Failed {
                    span.record("otel.status_code", "error");
                    if let Some(error) = error {
                        // info: exported as a span event, but go-task already
                        // told the terminal.
                        info!(parent: &span, error = %error, "task failed");
                    }
                }
                span
            }
            None => run_span.clone(),
        },
        _ => run_span.clone(),
    }
}

/// Hex trace/span ids of `span`, when an OTLP layer is recording it.
fn trace_ref(span: &Span) -> Option<TraceRef> {
    let context = span.context();
    let otel_span = context.span();
    let span_context = otel_span.span_context();
    span_context.is_valid().then(|| TraceRef {
        trace_id: span_context.trace_id().to_string(),
        span_id: span_context.span_id().to_string(),
    })
}
