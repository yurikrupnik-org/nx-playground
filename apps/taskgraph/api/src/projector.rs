//! Keeps the in-memory [`Projection`] in step with the `TASKGRAPH` stream.
//!
//! On (re)start it replays every retained fact into a fresh projection off to
//! the side and swaps it in once caught up, so readers never see a half-built
//! model and `/readyz` turns green only when the drill-down is complete. After
//! that, live facts are applied in place, fanned out to SSE subscribers and
//! counted in Prometheus metrics — replayed history is deliberately *not*
//! counted, or every restart would re-add a week of executions to the counters.
//!
//! If the subscription breaks, the whole model is rebuilt from the stream
//! rather than patched: the stream is the source of truth, and a rebuild is
//! the one path that is correct whatever was missed.

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_nats::jetstream::Context;
use contract_taskgraph::{EventBody, RunOutcome, TaskOutcome, TaskgraphEvent};
use domain_taskgraph::{Limits, Projection, TaskgraphResult, nats};
use futures::StreamExt;
use metrics::{counter, gauge, histogram};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Live events buffered per SSE subscriber before it starts lagging.
const CHANNEL_CAPACITY: usize = 1024;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub struct Shared {
    pub projection: RwLock<Projection>,
    pub ready: AtomicBool,
    pub events: broadcast::Sender<TaskgraphEvent>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            projection: RwLock::new(Projection::new(Limits::default())),
            ready: AtomicBool::new(false),
            events: broadcast::channel(CHANNEL_CAPACITY).0,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
}

/// Runs for the process lifetime.
pub async fn run(jetstream: Context, shared: Arc<Shared>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match follow(&jetstream, &shared).await {
            Ok(()) => warn!("TASKGRAPH subscription ended; rebuilding from the stream"),
            Err(e) => {
                error!(error = %e, "TASKGRAPH subscription failed; rebuilding from the stream")
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn follow(jetstream: &Context, shared: &Shared) -> TaskgraphResult<()> {
    let (backlog, stream) = nats::subscribe(jetstream).await?;
    let mut stream = pin!(stream);
    let mut building = Some(Projection::new(Limits::default()));
    if backlog == 0 {
        swap_in(shared, building.take(), 0);
    }
    while let Some(delivered) = stream.next().await {
        let delivered = delivered?;
        gauge!("taskgraph_projection_pending").set(delivered.pending as f64);
        let event = match delivered.event {
            Ok(event) => event,
            Err(e) => {
                counter!("taskgraph_events_undecodable_total").increment(1);
                warn!(seq = delivered.seq, error = %e, "skipping undecodable TASKGRAPH message");
                continue;
            }
        };
        match building.as_mut() {
            Some(projection) => {
                projection.apply(&event);
                if delivered.pending == 0 {
                    swap_in(shared, building.take(), delivered.seq);
                }
            }
            None => {
                let changed = shared.projection.write().apply(&event);
                if changed {
                    record(&event);
                    // No subscribers is not an error.
                    let _ = shared.events.send(event);
                }
            }
        }
        gauge!("taskgraph_projection_applied").set(shared.projection.read().applied() as f64);
    }
    Ok(())
}

fn swap_in(shared: &Shared, projection: Option<Projection>, seq: u64) {
    if let Some(projection) = projection {
        info!(
            seq,
            applied = projection.applied(),
            "TASKGRAPH replay caught up; serving"
        );
        *shared.projection.write() = projection;
        shared.ready.store(true, Ordering::Release);
    }
}

fn task_outcome(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Succeeded => "succeeded",
        TaskOutcome::Failed => "failed",
        TaskOutcome::UpToDate => "up_to_date",
        TaskOutcome::Cancelled => "cancelled",
    }
}

fn run_outcome(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Succeeded => "succeeded",
        RunOutcome::Failed => "failed",
    }
}

/// Prometheus view of live facts. `task` is a label: bounded by the number of
/// tasks in the Taskfiles publishing here, which is small by construction.
fn record(event: &TaskgraphEvent) {
    counter!("taskgraph_events_total", "type" => event.body.kind()).increment(1);
    match &event.body {
        EventBody::TaskFinished {
            task,
            outcome,
            duration_ms,
            ..
        } => {
            let outcome = task_outcome(*outcome);
            counter!("taskgraph_task_executions_total", "task" => task.clone(), "outcome" => outcome)
                .increment(1);
            histogram!("taskgraph_task_duration_seconds", "task" => task.clone(), "outcome" => outcome)
                .record(*duration_ms as f64 / 1000.0);
        }
        EventBody::RunFinished {
            outcome,
            duration_ms,
            ..
        } => {
            let outcome = run_outcome(*outcome);
            counter!("taskgraph_runs_total", "outcome" => outcome).increment(1);
            histogram!("taskgraph_run_duration_seconds", "outcome" => outcome)
                .record(*duration_ms as f64 / 1000.0);
        }
        _ => {}
    }
}
