//! Finished GitHub Actions run attempts → OTLP traces with their ORIGINAL
//! timestamps: workflow run → job → step, and the taskgraph runs recorded in
//! a job (→ task executions, nested by `parent`) under that job's span.
//!
//! Spans are built as `SpanData` and handed straight to an OTLP exporter
//! rather than through a tracer: the SDK tracer always draws fresh ids, and
//! these ids must be deterministic (trace id = first 16 bytes of
//! sha256(`github:<run_id>:<attempt>`)) so a re-export after a crash lands
//! on the same trace, and the export result is what marks an attempt done.
//! The resource's `service.name` is [`SERVICE_NAME`]: the spans describe CI
//! work, not this service.

use std::borrow::Cow;
use std::collections::HashSet;
use std::time::SystemTime;

use chrono::{DateTime, Duration, Utc};
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter as _, SpanLinks};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{InsightsError, InsightsResult};
use crate::store::{Store, TraceAttempt, TraceExecution, TraceJob, TraceStep, TraceTaskRun};

/// `service.name` of the exported spans.
pub const SERVICE_NAME: &str = "github-actions";
/// Run attempts per export call.
const BATCH: i64 = 20;

pub struct TraceExporter {
    exporter: opentelemetry_otlp::SpanExporter,
}

impl TraceExporter {
    /// OTLP/gRPC exporter for `endpoint` (`OTEL_EXPORTER_OTLP_ENDPOINT`).
    pub fn new(endpoint: &str) -> InsightsResult<Self> {
        let mut exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
            .map_err(|e| InsightsError::Otlp(format!("building exporter for {endpoint}: {e}")))?;
        opentelemetry_sdk::trace::SpanExporter::set_resource(
            &mut exporter,
            &Resource::builder().with_service_name(SERVICE_NAME).build(),
        );
        Ok(Self { exporter })
    }

    async fn export(&self, spans: Vec<SpanData>) -> InsightsResult<()> {
        self.exporter
            .export(spans)
            .await
            .map_err(|e| InsightsError::Otlp(e.to_string()))
    }
}

/// Everything one run attempt's trace is built from.
#[derive(Debug, Clone)]
pub struct RunTrace {
    pub attempt: TraceAttempt,
    pub jobs: Vec<TraceJob>,
    pub steps: Vec<TraceStep>,
    pub task_runs: Vec<TraceTaskRun>,
    pub executions: Vec<TraceExecution>,
}

fn sha256(key: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(key.as_bytes()));
    out
}

fn trace_key(run_id: i64, attempt: i32) -> String {
    format!("github:{run_id}:{attempt}")
}

/// First 16 bytes of sha256(`github:<run_id>:<attempt>`).
pub fn trace_id(run_id: i64, attempt: i32) -> TraceId {
    let digest = sha256(&trace_key(run_id, attempt));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    TraceId::from_bytes(bytes)
}

/// First 8 bytes of sha256(`<trace key>:<part>`), never the invalid zero id.
fn span_id(trace_key: &str, part: &str) -> SpanId {
    let digest = sha256(&format!("{trace_key}:{part}"));
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    if bytes == [0; 8] {
        bytes[7] = 1;
    }
    SpanId::from_bytes(bytes)
}

fn ci_status(conclusion: Option<&str>) -> Status {
    match conclusion {
        Some("success") => Status::Ok,
        Some(c @ ("failure" | "timed_out" | "startup_failure")) => Status::error(c.to_string()),
        _ => Status::Unset,
    }
}

fn task_status(outcome: Option<&str>, error: Option<&str>) -> Status {
    match outcome {
        Some("failed") => Status::error(error.unwrap_or("failed").to_string()),
        Some("succeeded" | "up_to_date") => Status::Ok,
        _ => Status::Unset,
    }
}

/// A span's interval from what is known: start, end, inclusive duration.
fn interval(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    duration_ms: Option<i64>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let duration = duration_ms.map(Duration::milliseconds);
    let (start, end) = match (start, end, duration) {
        (Some(s), Some(e), _) => (s, e),
        (Some(s), None, Some(d)) => (s, s + d),
        (None, Some(e), Some(d)) => (e - d, e),
        (Some(s), None, None) => (s, s),
        (None, Some(e), None) => (e, e),
        (None, None, _) => return None,
    };
    Some((start, end.max(start)))
}

struct SpanBuilder<'a> {
    trace_id: TraceId,
    scope: &'a InstrumentationScope,
    spans: Vec<SpanData>,
}

impl SpanBuilder<'_> {
    fn push(
        &mut self,
        id: SpanId,
        parent: SpanId,
        name: String,
        (start, end): (DateTime<Utc>, DateTime<Utc>),
        attributes: Vec<KeyValue>,
        status: Status,
    ) {
        self.spans.push(SpanData {
            span_context: SpanContext::new(
                self.trace_id,
                id,
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: parent,
            parent_span_is_remote: false,
            span_kind: SpanKind::Internal,
            name: Cow::Owned(name),
            start_time: SystemTime::from(start),
            end_time: SystemTime::from(end),
            attributes,
            dropped_attributes_count: 0,
            events: SpanEvents::default(),
            links: SpanLinks::default(),
            status,
            instrumentation_scope: self.scope.clone(),
        });
    }
}

fn opt(key: &'static str, value: Option<&str>) -> Option<KeyValue> {
    value.map(|v| KeyValue::new(key, v.to_string()))
}

/// Does GitHub job `name` belong to the workflow job key `ci_job`
/// (`GITHUB_JOB`)? Matrix jobs display as `<name> (<values>)`.
fn job_matches(name: &str, ci_job: &str) -> bool {
    name == ci_job
        || name
            .strip_prefix(ci_job)
            .is_some_and(|rest| rest.starts_with(" ("))
}

/// The spans of one run attempt, deterministic in ids and content.
pub fn build_spans(trace: &RunTrace) -> Vec<SpanData> {
    let a = &trace.attempt;
    let key = trace_key(a.run_id, a.attempt);
    let scope = InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();
    let mut out = SpanBuilder {
        trace_id: trace_id(a.run_id, a.attempt),
        scope: &scope,
        spans: Vec::new(),
    };

    let root = span_id(&key, "run");
    let run_start = a.started_at.unwrap_or(a.created_at);
    let run_end = a.completed_at.unwrap_or(run_start).max(run_start);
    let mut attributes: Vec<KeyValue> = vec![
        KeyValue::new("ci.provider", "github_actions"),
        KeyValue::new("ci.run_id", a.run_id),
        KeyValue::new("ci.run_attempt", i64::from(a.attempt)),
        KeyValue::new("ci.workflow", a.workflow.clone()),
        KeyValue::new("ci.workflow_path", a.workflow_path.clone()),
        KeyValue::new("ci.event", a.event.clone()),
        KeyValue::new("head_sha", a.head_sha.clone()),
        KeyValue::new("url", a.html_url.clone()),
    ];
    attributes.extend(
        [
            opt("ci.branch", a.branch.as_deref()),
            opt("conclusion", a.conclusion.as_deref()),
            opt("ci.actor", a.actor.as_deref()),
            opt("author", a.author.as_deref()),
            opt("author_kind", a.author_kind.as_deref()),
        ]
        .into_iter()
        .flatten(),
    );
    out.push(
        root,
        SpanId::INVALID,
        a.workflow.clone(),
        (run_start, run_end),
        attributes,
        ci_status(a.conclusion.as_deref()),
    );

    for job in &trace.jobs {
        let Some(bounds) = interval(job.started_at.or(job.created_at), job.completed_at, None)
        else {
            continue;
        };
        let id = span_id(&key, &format!("job:{}", job.job_id));
        let mut attributes = vec![KeyValue::new("ci.job_id", job.job_id)];
        attributes.extend(
            [
                opt("conclusion", job.conclusion.as_deref()),
                opt("ci.runner", job.runner.as_deref()),
                opt("url", job.html_url.as_deref()),
            ]
            .into_iter()
            .flatten(),
        );
        out.push(
            id,
            root,
            job.name.clone(),
            bounds,
            attributes,
            ci_status(job.conclusion.as_deref()),
        );
        for step in trace.steps.iter().filter(|s| s.job_id == job.job_id) {
            let Some(bounds) = interval(step.started_at, step.completed_at, None) else {
                continue;
            };
            let mut attributes = vec![KeyValue::new("ci.step", i64::from(step.number))];
            attributes.extend(opt("conclusion", step.conclusion.as_deref()));
            out.push(
                span_id(&key, &format!("step:{}:{}", job.job_id, step.number)),
                id,
                step.name.clone(),
                bounds,
                attributes,
                ci_status(step.conclusion.as_deref()),
            );
        }
    }

    for run in &trace.task_runs {
        let Some(bounds) = interval(run.started_at, run.finished_at, run.duration_ms) else {
            continue;
        };
        let parent = run
            .ci_job
            .as_deref()
            .and_then(|ci_job| trace.jobs.iter().find(|j| job_matches(&j.name, ci_job)))
            .map_or(root, |j| span_id(&key, &format!("job:{}", j.job_id)));
        let run_span = span_id(&key, &format!("taskrun:{}", run.run_id));
        let mut attributes = vec![KeyValue::new("taskgraph.run_id", run.run_id.to_string())];
        attributes.extend(
            [
                opt("target", run.target.as_deref()),
                opt("outcome", run.outcome.as_deref()),
                opt("invoker", run.invoker_kind.as_deref()),
                opt("agent", run.agent.as_deref()),
            ]
            .into_iter()
            .flatten(),
        );
        attributes.extend(
            run.exit_code
                .map(|c| KeyValue::new("exit_code", i64::from(c))),
        );
        out.push(
            run_span,
            parent,
            format!("task {}", run.target.as_deref().unwrap_or("?")),
            bounds,
            attributes,
            task_status(run.outcome.as_deref(), None),
        );

        let executions: Vec<&TraceExecution> = trace
            .executions
            .iter()
            .filter(|e| e.run_id == run.run_id)
            .collect();
        let instances: HashSet<i32> = executions.iter().map(|e| e.instance).collect();
        let execution_span =
            |run_id: Uuid, instance: i32| span_id(&key, &format!("task:{run_id}:{instance}"));
        for e in executions {
            let Some(bounds) = interval(e.started_at, e.finished_at, e.duration_ms) else {
                continue;
            };
            let parent = e
                .parent
                .filter(|p| instances.contains(p))
                .map_or(run_span, |p| execution_span(e.run_id, p));
            let mut attributes = vec![
                KeyValue::new("task", e.task.clone()),
                KeyValue::new("instance", i64::from(e.instance)),
            ];
            attributes.extend(
                [
                    opt("via", e.via.as_deref()),
                    opt("outcome", e.outcome.as_deref()),
                    opt("error", e.error.as_deref()),
                ]
                .into_iter()
                .flatten(),
            );
            out.push(
                execution_span(e.run_id, e.instance),
                parent,
                e.task.clone(),
                bounds,
                attributes,
                task_status(e.outcome.as_deref(), e.error.as_deref()),
            );
        }
    }
    out.spans
}

async fn load(store: &Store, attempt: TraceAttempt) -> InsightsResult<RunTrace> {
    let jobs = store.trace_jobs(attempt.run_id, attempt.attempt).await?;
    let job_ids: Vec<i64> = jobs.iter().map(|j| j.job_id).collect();
    let steps = store.trace_steps(&job_ids).await?;
    let task_runs = store
        .trace_task_runs(attempt.run_id, attempt.attempt)
        .await?;
    let run_ids: Vec<Uuid> = task_runs.iter().map(|r| r.run_id).collect();
    let executions = store.trace_executions(&run_ids).await?;
    Ok(RunTrace {
        attempt,
        jobs,
        steps,
        task_runs,
        executions,
    })
}

/// Export every pending attempt; an attempt is marked exported (with its
/// trace id) only after the exporter accepted its batch. Returns the number
/// of attempts exported.
pub async fn export_pending(store: &Store, exporter: &TraceExporter) -> InsightsResult<u64> {
    let mut exported = 0;
    loop {
        let attempts = store.attempts_pending_trace(BATCH).await?;
        let full_batch = attempts.len() as i64 == BATCH;
        if attempts.is_empty() {
            break;
        }
        let mut spans = Vec::new();
        let mut done = Vec::with_capacity(attempts.len());
        for attempt in attempts {
            let (run_id, n) = (attempt.run_id, attempt.attempt);
            spans.extend(build_spans(&load(store, attempt).await?));
            done.push((run_id, n, trace_id(run_id, n).to_string()));
        }
        exporter.export(spans).await?;
        store.mark_traces_exported(&done).await?;
        exported += done.len() as u64;
        if !full_batch {
            break;
        }
    }
    Ok(exported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().expect("timestamp")
    }

    fn fixture() -> RunTrace {
        let task_run = Uuid::from_u128(7);
        RunTrace {
            attempt: TraceAttempt {
                run_id: 42,
                attempt: 1,
                workflow: "CI".into(),
                workflow_path: ".github/workflows/ci-optimized.yml".into(),
                event: "push".into(),
                branch: Some("main".into()),
                head_sha: "abc".into(),
                conclusion: Some("failure".into()),
                created_at: at("2026-09-01T10:00:00Z"),
                started_at: Some(at("2026-09-01T10:00:05Z")),
                completed_at: Some(at("2026-09-01T10:10:00Z")),
                actor: Some("dev".into()),
                html_url: "https://github.com/o/r/actions/runs/42".into(),
                author: Some("dev".into()),
                author_kind: Some("human".into()),
            },
            jobs: vec![TraceJob {
                job_id: 9,
                name: "rust (stable)".into(),
                conclusion: Some("failure".into()),
                created_at: Some(at("2026-09-01T10:00:06Z")),
                started_at: Some(at("2026-09-01T10:00:30Z")),
                completed_at: Some(at("2026-09-01T10:09:00Z")),
                runner: Some("GitHub Actions 1".into()),
                html_url: None,
            }],
            steps: vec![TraceStep {
                job_id: 9,
                number: 1,
                name: "checkout".into(),
                conclusion: Some("success".into()),
                started_at: Some(at("2026-09-01T10:00:31Z")),
                completed_at: Some(at("2026-09-01T10:00:40Z")),
            }],
            task_runs: vec![TraceTaskRun {
                run_id: task_run,
                target: Some("check".into()),
                ci_job: Some("rust".into()),
                outcome: Some("failed".into()),
                exit_code: Some(1),
                invoker_kind: Some("ci".into()),
                agent: None,
                started_at: Some(at("2026-09-01T10:01:00Z")),
                finished_at: Some(at("2026-09-01T10:08:00Z")),
                duration_ms: Some(420_000),
            }],
            executions: vec![
                TraceExecution {
                    run_id: task_run,
                    instance: 1,
                    task: "check".into(),
                    parent: None,
                    via: Some("root".into()),
                    outcome: Some("failed".into()),
                    started_at: Some(at("2026-09-01T10:01:00Z")),
                    finished_at: Some(at("2026-09-01T10:08:00Z")),
                    duration_ms: Some(420_000),
                    error: Some("exit status 1".into()),
                },
                TraceExecution {
                    run_id: task_run,
                    instance: 2,
                    task: "lint".into(),
                    parent: Some(1),
                    via: Some("dep".into()),
                    outcome: Some("failed".into()),
                    // Only the finish was observed: start comes from the duration.
                    started_at: None,
                    finished_at: Some(at("2026-09-01T10:07:00Z")),
                    duration_ms: Some(60_000),
                    error: Some("exit status 1".into()),
                },
            ],
        }
    }

    /// Re-exporting after a crash must land on the same trace and spans,
    /// with the tree and the original timestamps intact.
    #[test]
    fn spans_are_deterministic_and_nested_with_original_times() {
        let trace = fixture();
        let spans = build_spans(&trace);
        let again = build_spans(&trace);
        let ids = |s: &[SpanData]| -> Vec<(SpanId, SpanId)> {
            s.iter()
                .map(|d| (d.span_context.span_id(), d.parent_span_id))
                .collect()
        };
        assert_eq!(ids(&spans), ids(&again));
        assert!(
            spans
                .iter()
                .all(|s| s.span_context.trace_id() == trace_id(42, 1))
        );
        assert_ne!(
            trace_id(42, 1),
            trace_id(42, 2),
            "attempts are separate traces"
        );

        let by_name = |name: &str| spans.iter().find(|s| s.name == name).expect(name);
        let (run, job, step) = (by_name("CI"), by_name("rust (stable)"), by_name("checkout"));
        let (task_run, check, lint) = (by_name("task check"), by_name("check"), by_name("lint"));
        assert_eq!(run.parent_span_id, SpanId::INVALID);
        assert_eq!(job.parent_span_id, run.span_context.span_id());
        assert_eq!(step.parent_span_id, job.span_context.span_id());
        // The task run is matched to its matrix job by GITHUB_JOB.
        assert_eq!(task_run.parent_span_id, job.span_context.span_id());
        assert_eq!(check.parent_span_id, task_run.span_context.span_id());
        assert_eq!(lint.parent_span_id, check.span_context.span_id());

        assert_eq!(run.start_time, SystemTime::from(at("2026-09-01T10:00:05Z")));
        assert_eq!(run.end_time, SystemTime::from(at("2026-09-01T10:10:00Z")));
        assert_eq!(
            lint.start_time,
            SystemTime::from(at("2026-09-01T10:06:00Z"))
        );
        assert!(matches!(run.status, Status::Error { .. }));
        assert!(matches!(step.status, Status::Ok));
    }
}
