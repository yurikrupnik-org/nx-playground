//! Wire contract for Taskfile graph and execution events.
//!
//! `taskgraph` (the CLI) parses a go-task `Taskfile.yml`, runs tasks through
//! the real `task` binary and publishes what it saw as facts on the
//! `TASKGRAPH` stream. `taskgraph_api` folds those facts into a read model and
//! serves the drill-down. Two independently deployed processes agreeing on a
//! JSON payload is the serialization boundary that justifies a contract crate
//! (`docs/architecture-backlog.md` 5.1).
//!
//! # Stream shape
//!
//! [`StreamKind::EventLog`]: these are facts (a run started, a task finished),
//! so every reader sees everything and a reader added tomorrow replays today.
//! The consumer name is deliberately not here — it belongs to the reader.
//!
//! Every event is one [`TaskgraphEvent`] envelope; the `type` field selects the
//! [`EventBody`] variant and [`EventBody::subject`] the concrete subject, so a
//! reader can filter server-side (`taskgraph.task_finished`) without decoding.
//!
//! # Execution identity
//!
//! go-task runs a task once per *reference* unless it declares `run: once`, and
//! runs `deps` concurrently — so a name is not an identity. Each execution gets
//! a run-scoped `instance` number, and `parent` + [`Via`] record which
//! execution pulled it in. Durations are inclusive: go-task reports a task as
//! started before its deps run and finished after its last command.
//!
//! # Origin
//!
//! [`RunOrigin`] says where a run happened (a CI job or a workstation), who
//! started it (a person, an AI coding agent, CI automation) and at which
//! commit. It is optional on the wire: facts recorded before it existed
//! decode with an empty origin.
//!
//! [`shell`] is the second, file-based contract: the static shell scan the
//! CLI writes and the insights service ingests.

pub mod shell;

use chrono::{DateTime, Utc};
use messaging::nats::StreamKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JetStream stream carrying `taskgraph.>` facts.
pub const TASKGRAPH_STREAM: &str = "TASKGRAPH";

/// Subject space owned by the stream. Concrete events use `taskgraph.<fact>`.
pub const TASKGRAPH_SUBJECT: &str = "taskgraph.>";

/// Dead-letter stream for `TASKGRAPH`.
pub const TASKGRAPH_DLQ: &str = "TASKGRAPH_DLQ";

/// Facts, not jobs: retained by age/count so every reader replays history.
pub const TASKGRAPH_KIND: StreamKind = StreamKind::EventLog;

/// A parsed Taskfile: every task reachable from the root file, includes
/// flattened into go-task's `namespace:task` names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    /// Stable identity of this Taskfile on this host (hash of host + path), so
    /// re-publishing an edited file replaces the previous graph.
    pub id: String,
    /// Absolute path of the root Taskfile.
    pub taskfile: String,
    pub host: String,
    /// Hash of the task/include model: equal digests mean an identical graph.
    pub digest: String,
    /// Sorted by `name`.
    pub tasks: Vec<TaskNode>,
    pub includes: Vec<IncludeNode>,
    /// Things the parser could not resolve (remote or templated includes,
    /// references to unknown tasks). The graph is still usable.
    pub warnings: Vec<String>,
}

/// One task, as declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNode {
    /// Fully-qualified name (`rust:lint` for a task in a namespaced include).
    pub name: String,
    pub desc: Option<String>,
    pub summary: Option<String>,
    pub aliases: Vec<String>,
    /// Declaring file, relative to the root Taskfile's directory.
    pub taskfile: String,
    pub internal: bool,
    /// `deps:` — run concurrently before `cmds`.
    pub deps: Vec<TaskRef>,
    /// `cmds: - task: x` — run sequentially, in order, as part of `cmds`.
    pub calls: Vec<TaskRef>,
    /// Shell commands in declaration order (`defer:` ones prefixed `defer: `).
    pub cmds: Vec<String>,
    pub sources: Vec<String>,
    pub generates: Vec<String>,
    /// `status:` checks; any present means the task can be skipped as up to date.
    pub status: Vec<String>,
    pub preconditions: Vec<String>,
    /// `requires: vars:` names.
    pub requires: Vec<String>,
    /// `run:` policy (`always` | `once` | `when_changed`), when declared.
    pub run: Option<String>,
    pub dir: Option<String>,
}

/// A reference from one task to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRef {
    /// The canonical target name when `resolved`, else the reference as written.
    pub name: String,
    /// `false` for templated (`{{.X}}`) or unknown targets.
    pub resolved: bool,
}

/// One `includes:` entry, after namespace flattening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncludeNode {
    /// Fully-qualified namespace (empty for a flattened include).
    pub namespace: String,
    /// Path relative to the root Taskfile's directory, or the URL for a remote one.
    pub taskfile: String,
    pub resolved: bool,
    pub remote: bool,
    pub optional: bool,
    pub flatten: bool,
    pub internal: bool,
}

/// W3C trace identity of the span that produced an event, hex-encoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRef {
    pub trace_id: String,
    pub span_id: String,
}

/// How an execution was pulled into a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// The task named on the command line (or one whose caller is unknown).
    Root,
    /// Listed in the parent's `deps:`.
    Dep,
    /// A `task:` entry in the parent's `cmds:`.
    Call,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Succeeded,
    Failed,
    /// `status:`/`sources:` said there was nothing to do.
    UpToDate,
    /// Still running when go-task exited (a sibling failed, or an interrupt).
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Succeeded,
    Failed,
}

/// Which CI system a run happened on, and where to find that run there.
///
/// `provider` is an open set on purpose (`github_actions`, `tekton`,
/// `gitlab_ci`, `buildkite`, `circleci`, `jenkins`, or whatever
/// `TASKGRAPH_CI_PROVIDER` says): a reader groups by it, never matches it
/// exhaustively.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiContext {
    pub provider: String,
    /// The provider's id of the pipeline run (GitHub: `GITHUB_RUN_ID`).
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_url: Option<String>,
    /// Workflow / pipeline name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<String>,
    /// Job / task-run name within the pipeline run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// `owner/name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Commit the pipeline was triggered for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// Trigger (`push`, `pull_request`, `schedule`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Account that triggered the pipeline run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokerKind {
    /// Nothing marked the environment as automated.
    Human,
    /// An AI coding agent's shell (`tools/authorship/agents.json` markers),
    /// including an agent running inside a CI job.
    Agent,
    /// CI automation with no agent marker.
    Ci,
}

/// Who started go-task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoker {
    pub kind: InvokerKind,
    /// Agent id from `tools/authorship/agents.json` when `kind` is `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Where and by whom a run was started. Every field is optional so facts
/// published before the origin existed still decode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<CiContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoker: Option<Invoker>,
    /// `git rev-parse HEAD` of the working directory at start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

/// The fact itself; the variant decides the subject.
// RunStarted (origin) dwarfs the other variants, but there is one per run and
// events are serialized and dropped, never held in bulk; boxing would add an
// allocation and churn every constructor and pattern.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventBody {
    /// A Taskfile was parsed. Published before every run and by `taskgraph publish`.
    GraphPublished { graph: Graph },
    RunStarted {
        run_id: Uuid,
        graph_id: String,
        /// Task named on the command line.
        target: String,
        /// Arguments after `--`, forwarded to go-task as `CLI_ARGS`.
        args: Vec<String>,
        host: String,
        user: String,
        cwd: String,
        /// Expected duration from history at start time, if any history existed.
        estimate_ms: Option<u64>,
        #[serde(default)]
        origin: RunOrigin,
    },
    TaskStarted {
        run_id: Uuid,
        instance: u32,
        task: String,
        parent: Option<u32>,
        via: Via,
    },
    /// go-task echoed a command it is about to run for this execution.
    CommandStarted {
        run_id: Uuid,
        instance: u32,
        task: String,
        command: String,
    },
    TaskFinished {
        run_id: Uuid,
        instance: u32,
        task: String,
        outcome: TaskOutcome,
        duration_ms: u64,
        error: Option<String>,
    },
    RunFinished {
        run_id: Uuid,
        outcome: RunOutcome,
        /// `None` when go-task was killed by a signal.
        exit_code: Option<i32>,
        duration_ms: u64,
        error: Option<String>,
    },
}

impl EventBody {
    pub const GRAPH_PUBLISHED: &'static str = "taskgraph.graph_published";
    pub const RUN_STARTED: &'static str = "taskgraph.run_started";
    pub const TASK_STARTED: &'static str = "taskgraph.task_started";
    pub const COMMAND_STARTED: &'static str = "taskgraph.command_started";
    pub const TASK_FINISHED: &'static str = "taskgraph.task_finished";
    pub const RUN_FINISHED: &'static str = "taskgraph.run_finished";

    /// Concrete subject within [`TASKGRAPH_SUBJECT`].
    pub fn subject(&self) -> &'static str {
        match self {
            Self::GraphPublished { .. } => Self::GRAPH_PUBLISHED,
            Self::RunStarted { .. } => Self::RUN_STARTED,
            Self::TaskStarted { .. } => Self::TASK_STARTED,
            Self::CommandStarted { .. } => Self::COMMAND_STARTED,
            Self::TaskFinished { .. } => Self::TASK_FINISHED,
            Self::RunFinished { .. } => Self::RUN_FINISHED,
        }
    }

    /// The `type` tag value (also the subject's last token).
    pub fn kind(&self) -> &'static str {
        self.subject().trim_start_matches("taskgraph.")
    }

    /// The run this fact belongs to; `None` for graph publications.
    pub fn run_id(&self) -> Option<Uuid> {
        match self {
            Self::GraphPublished { .. } => None,
            Self::RunStarted { run_id, .. }
            | Self::TaskStarted { run_id, .. }
            | Self::CommandStarted { run_id, .. }
            | Self::TaskFinished { run_id, .. }
            | Self::RunFinished { run_id, .. } => Some(*run_id),
        }
    }
}

/// Envelope published for every fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskgraphEvent {
    pub event_id: Uuid,
    /// When the publisher observed the fact (publisher clock).
    pub at: DateTime<Utc>,
    /// Span that was current when the fact was observed; `None` without OTLP export.
    pub trace: Option<TraceRef>,
    #[serde(flatten)]
    pub body: EventBody,
}

impl TaskgraphEvent {
    pub fn new(body: EventBody, at: DateTime<Utc>, trace: Option<TraceRef>) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            at,
            trace,
            body,
        }
    }
}

impl messaging::Job for TaskgraphEvent {
    fn job_id(&self) -> Uuid {
        self.event_id
    }

    fn job_type(&self) -> &'static str {
        self.body.kind()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<EventBody> {
        let run_id = Uuid::now_v7();
        vec![
            EventBody::GraphPublished {
                graph: Graph {
                    id: "g".into(),
                    taskfile: "/repo/Taskfile.yml".into(),
                    host: "h".into(),
                    digest: "d".into(),
                    tasks: vec![TaskNode {
                        name: "rust:lint".into(),
                        desc: Some("lint".into()),
                        summary: None,
                        aliases: vec![],
                        taskfile: "scripts/tasks/rust.yml".into(),
                        internal: false,
                        deps: vec![TaskRef {
                            name: "rust:fmt".into(),
                            resolved: true,
                        }],
                        calls: vec![],
                        cmds: vec!["cargo clippy".into()],
                        sources: vec![],
                        generates: vec![],
                        status: vec![],
                        preconditions: vec![],
                        requires: vec![],
                        run: None,
                        dir: None,
                    }],
                    includes: vec![],
                    warnings: vec![],
                },
            },
            EventBody::RunStarted {
                run_id,
                graph_id: "g".into(),
                target: "check".into(),
                args: vec!["-v".into()],
                host: "h".into(),
                user: "u".into(),
                cwd: "/repo".into(),
                estimate_ms: Some(1200),
                origin: RunOrigin {
                    ci: Some(CiContext {
                        provider: "github_actions".into(),
                        run_id: "36338703831".into(),
                        run_attempt: Some(2),
                        job: Some("rust".into()),
                        sha: Some("5f17449".into()),
                        ..CiContext::default()
                    }),
                    invoker: Some(Invoker {
                        kind: InvokerKind::Agent,
                        agent: Some("claude-code".into()),
                    }),
                    sha: Some("5f17449".into()),
                },
            },
            EventBody::TaskStarted {
                run_id,
                instance: 2,
                task: "rust:lint".into(),
                parent: Some(1),
                via: Via::Dep,
            },
            EventBody::CommandStarted {
                run_id,
                instance: 2,
                task: "rust:lint".into(),
                command: "cargo clippy".into(),
            },
            EventBody::TaskFinished {
                run_id,
                instance: 2,
                task: "rust:lint".into(),
                outcome: TaskOutcome::UpToDate,
                duration_ms: 7,
                error: None,
            },
            EventBody::RunFinished {
                run_id,
                outcome: RunOutcome::Failed,
                exit_code: Some(201),
                duration_ms: 99,
                error: Some("exit status 1".into()),
            },
        ]
    }

    /// A subject outside the stream's space publishes to no stream: the event
    /// is silently dropped.
    #[test]
    fn every_subject_is_within_the_stream_subject_space() {
        let prefix = TASKGRAPH_SUBJECT.trim_end_matches('>');
        for body in samples() {
            assert!(body.subject().starts_with(prefix), "{}", body.subject());
            assert_eq!(format!("taskgraph.{}", body.kind()), body.subject());
        }
    }

    /// `#[serde(flatten)]` over an internally tagged enum is where serde
    /// round-trips quietly break (numbers buffered as the wrong type); every
    /// variant must survive the wire, and the tag must equal the subject token.
    #[test]
    fn every_variant_round_trips_with_its_type_tag() {
        for body in samples() {
            let event = TaskgraphEvent::new(
                body,
                Utc::now(),
                Some(TraceRef {
                    trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
                    span_id: "b7ad6b7169203331".into(),
                }),
            );
            let json = serde_json::to_value(&event).expect("serialize");
            assert_eq!(json["type"], event.body.kind());
            let back: TaskgraphEvent = serde_json::from_value(json).expect("deserialize");
            assert_eq!(back, event);
        }
    }

    /// Facts recorded before `origin` existed must still decode, with an
    /// empty origin — the stream and every CI artifact keep old payloads.
    #[test]
    fn run_started_without_origin_decodes_with_an_empty_origin() {
        let json = serde_json::json!({
            "event_id": Uuid::now_v7(),
            "at": Utc::now(),
            "trace": null,
            "type": "run_started",
            "run_id": Uuid::now_v7(),
            "graph_id": "g",
            "target": "check",
            "args": [],
            "host": "h",
            "user": "u",
            "cwd": "/repo",
            "estimate_ms": null
        });
        let event: TaskgraphEvent = serde_json::from_value(json).expect("decode legacy fact");
        match event.body {
            EventBody::RunStarted { origin, .. } => assert_eq!(origin, RunOrigin::default()),
            other => panic!("unexpected {other:?}"),
        }
    }
}
