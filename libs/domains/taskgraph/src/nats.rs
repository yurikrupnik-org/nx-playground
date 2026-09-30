//! JetStream I/O for the `TASKGRAPH` stream.
//!
//! Stream identity comes from `contract_taskgraph` and the stream is built by
//! `messaging::nats::stream_config_for` only, so every process that touches it
//! creates the identical config (whoever creates it first wins).
//!
//! Reading uses an **ordered ephemeral consumer**, not a durable `NatsWorker`:
//! the only reader is a projection that is rebuilt from the first retained
//! message on every start, so there is no cursor worth persisting, nothing to
//! ack, and no side effect to deduplicate. The server reaps the consumer when
//! the reader goes away.

use async_nats::jetstream::Context;
use async_nats::jetstream::consumer::DeliverPolicy;
use async_nats::jetstream::consumer::pull::OrderedConfig;
use async_nats::jetstream::stream::Stream as JsStream;
use async_trait::async_trait;
use contract_taskgraph::{TASKGRAPH_KIND, TASKGRAPH_STREAM, TASKGRAPH_SUBJECT, TaskgraphEvent};
use futures::{Stream, StreamExt};
use messaging::nats::stream_config_for;
use tracing::warn;

use crate::error::{TaskgraphError, TaskgraphResult};
use crate::projection::Projection;

/// Where facts go. Required by the CLI's run loop, so a NATS-less run has a
/// real substitute ([`NoopPublisher`]) instead of `Option` checks everywhere.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: &TaskgraphEvent) -> TaskgraphResult<()>;
}

pub struct NoopPublisher;

#[async_trait]
impl EventPublisher for NoopPublisher {
    async fn publish(&self, _event: &TaskgraphEvent) -> TaskgraphResult<()> {
        Ok(())
    }
}

/// Publishes each fact with `Nats-Msg-Id: <event_id>`, so the server drops a
/// re-publish of the same event (an artifact ingested twice, a retried
/// publish) within the stream's duplicate window. `NatsProducer` has no
/// header support, hence the direct JetStream publish.
#[derive(Clone)]
pub struct NatsPublisher {
    jetstream: Context,
}

impl NatsPublisher {
    /// Creates the `TASKGRAPH` stream if absent, so a CLI can publish before
    /// the API has ever started.
    pub async fn new(jetstream: Context) -> TaskgraphResult<Self> {
        ensure_stream(&jetstream).await?;
        Ok(Self { jetstream })
    }

    /// Publish and wait for the stream's ack; `Ok(true)` when the server
    /// recognised the event id as a duplicate and did not store it again.
    pub async fn send(&self, event: &TaskgraphEvent) -> TaskgraphResult<bool> {
        let subject = event.body.subject();
        let fail =
            |e: &dyn std::fmt::Display| TaskgraphError::Nats(format!("publish {subject}: {e}"));
        let payload = serde_json::to_vec(event).map_err(|e| fail(&e))?;
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            async_nats::header::NATS_MESSAGE_ID,
            event.event_id.to_string().as_str(),
        );
        let ack = self
            .jetstream
            .publish_with_headers(subject, headers, payload.into())
            .await
            .map_err(|e| fail(&e))?
            .await
            .map_err(|e| fail(&e))?;
        Ok(ack.duplicate)
    }
}

#[async_trait]
impl EventPublisher for NatsPublisher {
    async fn publish(&self, event: &TaskgraphEvent) -> TaskgraphResult<()> {
        self.send(event).await.map(|_| ())
    }
}

pub async fn ensure_stream(jetstream: &Context) -> TaskgraphResult<JsStream> {
    jetstream
        .get_or_create_stream(stream_config_for(
            TASKGRAPH_STREAM,
            TASKGRAPH_SUBJECT,
            TASKGRAPH_KIND,
        ))
        .await
        .map_err(|e| TaskgraphError::Nats(format!("create {TASKGRAPH_STREAM} stream: {e}")))
}

/// One message read back from the stream.
pub struct Delivered {
    pub seq: u64,
    /// Messages still queued behind this one; `0` means caught up.
    pub pending: u64,
    /// A payload that does not decode is surfaced, not dropped silently.
    pub event: Result<TaskgraphEvent, serde_json::Error>,
}

/// Every retained fact, oldest first, then live ones as they are published.
/// Also returns how many messages the stream held at subscription time, so a
/// caller can tell "empty stream" from "still replaying".
pub async fn subscribe(
    jetstream: &Context,
) -> TaskgraphResult<(u64, impl Stream<Item = TaskgraphResult<Delivered>>)> {
    let mut stream = ensure_stream(jetstream).await?;
    let backlog = stream
        .info()
        .await
        .map_err(|e| TaskgraphError::Nats(format!("{TASKGRAPH_STREAM} info: {e}")))?
        .state
        .messages;
    let consumer = stream
        .create_consumer(OrderedConfig {
            filter_subject: TASKGRAPH_SUBJECT.to_string(),
            deliver_policy: DeliverPolicy::All,
            ..Default::default()
        })
        .await
        .map_err(|e| TaskgraphError::Nats(format!("ordered consumer: {e}")))?;
    let messages = consumer
        .messages()
        .await
        .map_err(|e| TaskgraphError::Nats(format!("ordered consumer messages: {e}")))?;
    let delivered = messages.map(|message| {
        let message = message.map_err(|e| TaskgraphError::Nats(format!("receive: {e}")))?;
        let info = message
            .info()
            .map_err(|e| TaskgraphError::Nats(format!("message info: {e}")))?;
        Ok(Delivered {
            seq: info.stream_sequence,
            pending: info.pending,
            event: serde_json::from_slice(&message.payload),
        })
    });
    Ok((backlog, delivered))
}

/// Fold everything retained into `projection` and stop once caught up.
/// Returns the number of messages read.
pub async fn replay(jetstream: &Context, projection: &mut Projection) -> TaskgraphResult<u64> {
    let (backlog, stream) = subscribe(jetstream).await?;
    if backlog == 0 {
        return Ok(0);
    }
    let mut stream = std::pin::pin!(stream);
    let mut read = 0;
    while let Some(delivered) = stream.next().await {
        let delivered = delivered?;
        read += 1;
        match &delivered.event {
            Ok(event) => {
                projection.apply(event);
            }
            Err(e) => {
                warn!(seq = delivered.seq, error = %e, "skipping undecodable TASKGRAPH message")
            }
        }
        if delivered.pending == 0 {
            break;
        }
    }
    Ok(read)
}
