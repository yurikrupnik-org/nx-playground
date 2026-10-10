//! `TASKGRAPH` → Postgres: the `insights-warehouse` durable consumer.
//!
//! `TASKGRAPH` is an event log (Limits retention), not a work queue: acking
//! only advances this consumer's cursor, every other reader still sees the
//! message. Each batch is written in one transaction and acked only after it
//! commits; a crash in between redelivers the batch and the `event_id` ledger
//! turns the replay into a no-op. An undecodable payload is recorded as a
//! sync error and terminated (never redelivered to this consumer).

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::Context;
use contract_taskgraph::{
    TASKGRAPH_DLQ, TASKGRAPH_KIND, TASKGRAPH_STREAM, TASKGRAPH_SUBJECT, TaskgraphEvent,
};
use messaging::nats::{NatsConsumer, StreamConfig, StreamKind, WorkerConfig};
use tracing::warn;

use crate::error::{InsightsError, InsightsResult};
use crate::store::Store;

/// This service's consumer group on `TASKGRAPH`.
pub const CONSUMER_NAME: &str = "insights-warehouse";
const BATCH: usize = 256;

/// The warehouse's view of the `TASKGRAPH` stream.
pub struct WarehouseStream;

impl StreamConfig for WarehouseStream {
    const STREAM_NAME: &'static str = TASKGRAPH_STREAM;
    const CONSUMER_NAME: &'static str = CONSUMER_NAME;
    const DLQ_STREAM: &'static str = TASKGRAPH_DLQ;
    const SUBJECT: &'static str = TASKGRAPH_SUBJECT;
    const KIND: StreamKind = TASKGRAPH_KIND;
    /// Unlimited: a database outage must delay facts, never drop them.
    const MAX_DELIVER: i64 = -1;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub applied: u64,
    pub duplicates: u64,
    pub undecodable: u64,
}

/// Apply everything the consumer has not seen yet; returns once the
/// consumer has nothing pending.
pub async fn drain(jetstream: &Context, store: &Store) -> InsightsResult<DrainReport> {
    let consumer = NatsConsumer::new(
        Arc::new(jetstream.clone()),
        WorkerConfig::from_stream::<WarehouseStream>()
            .with_batch_size(BATCH)
            .with_fetch_timeout(Duration::from_secs(2)),
    );
    consumer.init().await.map_err(nats)?;
    let mut report = DrainReport::default();
    // Asking first keeps an idle drain to one round trip: a fetch with
    // nothing to deliver waits out its expiry.
    while pending(&consumer).await? > 0 {
        let fetched = consumer
            .fetch::<TaskgraphEvent>(BATCH)
            .await
            .map_err(nats)?;
        if fetched.is_empty() {
            // Pending but not deliverable now (e.g. ack-pending limit): next cycle.
            break;
        }
        let batch: Vec<(&TaskgraphEvent, Option<u64>)> = fetched
            .jobs
            .iter()
            .map(|m| (&m.job, Some(m.sequence)))
            .collect();
        let applied = store.apply_events(&batch).await?;
        report.applied += applied.applied;
        report.duplicates += applied.duplicates;
        for message in fetched.jobs {
            message.ack().await.map_err(nats)?;
        }
        for poison in fetched.poison {
            warn!(seq = poison.sequence, subject = %poison.subject, error = %poison.error, "undecodable TASKGRAPH message");
            store
                .record_sync_error(
                    "warehouse",
                    Some(&format!("{} seq {}", poison.subject, poison.sequence)),
                    &poison.error,
                )
                .await?;
            report.undecodable += 1;
            poison.term().await.map_err(nats)?;
        }
    }
    Ok(report)
}

/// Messages this consumer has not been delivered yet.
async fn pending(consumer: &NatsConsumer) -> InsightsResult<u64> {
    Ok(consumer
        .ensure_consumer()
        .await
        .map_err(nats)?
        .info()
        .await
        .map_err(|e| InsightsError::Nats(e.to_string()))?
        .num_pending)
}

fn nats(e: messaging::nats::NatsError) -> InsightsError {
    InsightsError::Nats(e.to_string())
}
