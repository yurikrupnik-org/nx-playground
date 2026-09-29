//! Everything a command needs from the environment: which Taskfile, who and
//! where we are, and (optionally) a JetStream connection plus the history
//! replayed from it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_nats::jetstream::Context;
use chrono::Utc;
use contract_taskgraph::{EventBody, Graph, TaskgraphEvent};
use domain_taskgraph::{Limits, Projection, nats, taskfile};
use eyre::{Result, WrapErr, eyre};
use tracing::warn;

/// A connect attempt against a down broker must not stall an interactive command.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Replay bound: history is a nicety for `show`/`run`, never worth a hang.
const REPLAY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Ctx {
    /// Root Taskfile as found (not canonicalized — go-task gets the same path).
    pub taskfile: PathBuf,
    /// Whether the user named the file; if not, go-task discovers it itself.
    pub explicit_taskfile: bool,
    pub host: String,
    pub user: String,
    pub cwd: PathBuf,
    pub nats_url: String,
    pub offline: bool,
    /// `$TASKGRAPH_EVENTS_OUT`, made absolute: every published event is also
    /// appended here as one JSON line.
    pub events_out: Option<PathBuf>,
}

impl Ctx {
    pub fn new(taskfile: Option<PathBuf>, nats_url: String, offline: bool) -> Result<Self> {
        let cwd = std::env::current_dir().wrap_err("current directory")?;
        let explicit_taskfile = taskfile.is_some();
        let taskfile = match taskfile {
            Some(path) => path,
            None => taskfile::find_taskfile(&cwd).ok_or_else(|| {
                eyre!(
                    "no Taskfile found in {} or any parent (looked for {})",
                    cwd.display(),
                    taskfile::DEFAULT_TASKFILES.join(", ")
                )
            })?,
        };
        let events_out = std::env::var_os(EVENTS_OUT_ENV)
            .filter(|v| !v.is_empty())
            .map(|v| cwd.join(v));
        Ok(Self {
            taskfile,
            explicit_taskfile,
            host: hostname(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".into()),
            cwd,
            nats_url,
            offline,
            events_out,
        })
    }

    pub fn parse(&self) -> Result<Graph> {
        taskfile::parse(&self.taskfile, &self.host)
            .wrap_err_with(|| format!("parsing {}", self.taskfile.display()))
    }

    /// `None` when offline or the broker is unreachable (warned, not fatal).
    pub async fn jetstream(&self) -> Option<Context> {
        if self.offline {
            return None;
        }
        match connect(&self.nats_url).await {
            Ok(js) => Some(js),
            Err(e) => {
                warn!(nats_url = %self.nats_url, error = %e, "NATS unreachable; continuing without events/history");
                None
            }
        }
    }

    /// Like [`Self::jetstream`], but for commands that are pointless without it.
    pub async fn require_jetstream(&self) -> Result<Context> {
        if self.offline {
            return Err(eyre!(
                "this command needs NATS; drop --offline / unset TASKGRAPH_OFFLINE"
            ));
        }
        connect(&self.nats_url).await
    }
}

/// Env var naming the JSONL file `run` appends every event to.
pub const EVENTS_OUT_ENV: &str = "TASKGRAPH_EVENTS_OUT";

/// JetStream at `nats_url`, bounded by [`CONNECT_TIMEOUT`].
pub async fn connect(nats_url: &str) -> Result<Context> {
    tokio::time::timeout(CONNECT_TIMEOUT, messaging::nats::jetstream(nats_url))
        .await
        .map_err(|_| eyre!("NATS connect to {nats_url} timed out"))?
        .wrap_err_with(|| format!("connecting to NATS at {nats_url}"))
}

/// Directory of the root Taskfile: what repository-relative paths are
/// relative to.
pub fn root_dir(graph: &Graph) -> &Path {
    Path::new(&graph.taskfile)
        .parent()
        .unwrap_or(Path::new("/"))
}

/// Replay retained history into a projection that also knows `graph` (the
/// freshly parsed file), so stats and estimates line up with what is on disk.
pub async fn history(js: Option<&Context>, graph: &Graph) -> Projection {
    let mut projection = Projection::new(Limits::default());
    if let Some(js) = js {
        match tokio::time::timeout(REPLAY_TIMEOUT, nats::replay(js, &mut projection)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!(error = %e, "history replay failed; showing the graph without it"),
            Err(_) => warn!("history replay timed out; showing partial history"),
        }
    }
    projection.apply(&TaskgraphEvent::new(
        EventBody::GraphPublished {
            graph: graph.clone(),
        },
        Utc::now(),
        None,
    ));
    projection
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown".into())
}
