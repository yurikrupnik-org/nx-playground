//! NATS JetStream consumer for receiving jobs.

use crate::nats::config::{StreamKind, WorkerConfig};
use crate::nats::error::NatsError;
use crate::Job;
use async_nats::jetstream::consumer::pull::Config as ConsumerConfig;
use async_nats::jetstream::consumer::AckPolicy;
use async_nats::jetstream::stream::Config as StreamConfig;
use async_nats::jetstream::stream::RetentionPolicy;
use async_nats::jetstream::Context;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Map the declared stream kind onto a JetStream retention policy.
///
/// `WorkQueue` is the load-bearing one: the server deletes a message once acked and
/// **refuses** a second consumer with an overlapping subject filter, so a job stream
/// cannot be accidentally fanned out.
///
/// `EventLog` maps to `Limits`, **not** `Interest`. Under `Interest` a message is
/// dropped unless a consumer already exists when it is published, and a consumer
/// registered later receives nothing published before it existed — which destroys the
/// one property an event log is for. `Limits` retains by age/count regardless of who
/// is listening, so a new consumer group can be added at any time and replay history.
pub fn retention_for(kind: StreamKind) -> RetentionPolicy {
    match kind {
        StreamKind::JobQueue => RetentionPolicy::WorkQueue,
        StreamKind::EventLog => RetentionPolicy::Limits,
    }
}

/// Build the JetStream stream configuration for a stream this workspace owns.
///
/// **Use this everywhere a stream is created** — consumer startup, domain publishers,
/// and examples alike. `get_or_create_stream` does not reconcile an existing stream, so
/// whichever side runs first wins and any disagreement is silent: a publisher that
/// hardcodes `Limits` against a `JobQueue` consumer quietly restores fan-out. One
/// builder makes that disagreement unrepresentable.
pub fn stream_config_for(
    name: impl Into<String>,
    subject: impl Into<String>,
    kind: StreamKind,
) -> StreamConfig {
    StreamConfig {
        name: name.into(),
        subjects: vec![subject.into()],
        max_messages: 100_000,
        max_age: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
        retention: retention_for(kind),
        ..Default::default()
    }
}

/// Consumer for receiving jobs from NATS JetStream.
pub struct NatsConsumer {
    jetstream: Arc<Context>,
    config: WorkerConfig,
}

impl NatsConsumer {
    /// Create a new NATS consumer.
    pub fn new(jetstream: Arc<Context>, config: WorkerConfig) -> Self {
        Self { jetstream, config }
    }

    /// Get the JetStream context.
    pub fn jetstream(&self) -> Arc<Context> {
        self.jetstream.clone()
    }

    /// Get the stream name.
    pub fn stream_name(&self) -> &str {
        &self.config.stream_name
    }

    /// Get the consumer name.
    pub fn consumer_name(&self) -> &str {
        &self.config.consumer_name
    }

    /// Ensure the stream exists, creating it if necessary.
    pub async fn ensure_stream(&self) -> Result<(), NatsError> {
        // Try to get the stream first
        match self.jetstream.get_stream(&self.config.stream_name).await {
            Ok(mut stream) => {
                debug!(
                    stream = %self.config.stream_name,
                    "Stream already exists"
                );
                let info = stream.info().await?;
                debug!(
                    stream = %self.config.stream_name,
                    messages = info.state.messages,
                    "Stream info"
                );
                Ok(())
            }
            Err(_) => {
                // Create the stream
                info!(
                    stream = %self.config.stream_name,
                    subject = %self.config.subject,
                    kind = ?self.config.kind,
                    "Creating stream"
                );

                self.jetstream
                    .create_stream(stream_config_for(
                        self.config.stream_name.clone(),
                        self.config.subject.clone(),
                        self.config.kind,
                    ))
                    .await?;

                info!(
                    stream = %self.config.stream_name,
                    "Stream created"
                );

                Ok(())
            }
        }
    }

    /// Ensure the consumer exists, creating it if necessary.
    pub async fn ensure_consumer(
        &self,
    ) -> Result<async_nats::jetstream::consumer::Consumer<ConsumerConfig>, NatsError> {
        let stream = self.jetstream.get_stream(&self.config.stream_name).await?;

        // Try to get existing consumer
        match stream
            .get_consumer::<ConsumerConfig>(&self.config.consumer_name)
            .await
        {
            Ok(consumer) => {
                debug!(
                    consumer = %self.config.consumer_name,
                    "Consumer already exists"
                );
                Ok(consumer)
            }
            Err(_) => {
                // Create the consumer
                info!(
                    consumer = %self.config.consumer_name,
                    stream = %self.config.stream_name,
                    "Creating consumer"
                );

                let consumer = stream
                    .create_consumer(ConsumerConfig {
                        durable_name: Some(self.config.consumer_name.clone()),
                        name: Some(self.config.consumer_name.clone()),
                        ack_policy: AckPolicy::Explicit,
                        ack_wait: self.config.ack_wait,
                        max_deliver: self.config.max_deliver,
                        filter_subject: self.config.subject.clone(),
                        ..Default::default()
                    })
                    .await?;

                info!(
                    consumer = %self.config.consumer_name,
                    "Consumer created"
                );

                Ok(consumer)
            }
        }
    }

    /// Initialize stream and consumer.
    pub async fn init(&self) -> Result<(), NatsError> {
        self.ensure_stream().await?;
        self.ensure_consumer().await?;
        Ok(())
    }

    /// Fetch a batch of messages.
    ///
    /// Messages that fail to deserialize are returned as [`Fetched::poison`] rather
    /// than dropped. The consumer has no `DlqManager`, so it cannot decide their fate;
    /// the worker owns that. Previously this path called `ack()`, which on a
    /// `WorkQueue` stream **deletes** the message — a producer/consumer schema skew
    /// silently ate the backlog with nothing but a `warn!` to show for it.
    pub async fn fetch<J: Job>(&self, batch_size: usize) -> Result<Fetched<J>, NatsError> {
        let consumer = self.ensure_consumer().await?;

        let mut messages = consumer
            .fetch()
            .max_messages(batch_size)
            .expires(self.config.fetch_timeout)
            .messages()
            .await?;

        let mut fetched = Fetched::default();

        while let Some(msg) = messages.next().await {
            match msg {
                Ok(message) => {
                    // Read info before the message is moved into either bucket.
                    let (sequence, delivery_count) = match message.info() {
                        Ok(info) => (info.stream_sequence, info.delivered as u32),
                        Err(e) => {
                            warn!(error = %e, "Failed to get message info, using defaults");
                            (0, 1) // Default values
                        }
                    };
                    let subject = message.subject.to_string();

                    match serde_json::from_slice::<J>(&message.payload) {
                        Ok(job) => fetched.jobs.push(NatsMessage {
                            job,
                            subject,
                            message,
                            sequence,
                            delivery_count,
                        }),
                        Err(e) => {
                            warn!(
                                error = %e,
                                subject = %subject,
                                sequence = sequence,
                                "Failed to deserialize message, routing to DLQ"
                            );
                            fetched.poison.push(PoisonMessage {
                                raw: message.payload.to_vec(),
                                subject,
                                error: e.to_string(),
                                message,
                                sequence,
                                delivery_count,
                            });
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Error receiving message");
                }
            }
        }

        Ok(fetched)
    }

    /// Get stream info.
    pub async fn stream_info(&self) -> Result<StreamInfo, NatsError> {
        let mut stream = self.jetstream.get_stream(&self.config.stream_name).await?;

        let info = stream.info().await?;

        Ok(StreamInfo {
            stream_name: self.config.stream_name.clone(),
            messages: info.state.messages,
            bytes: info.state.bytes,
            first_sequence: info.state.first_sequence,
            last_sequence: info.state.last_sequence,
            consumer_count: info.state.consumer_count as i64,
        })
    }
}

/// One `fetch` batch, split by whether the payload decoded.
pub struct Fetched<J: Job> {
    /// Messages that deserialized into `J`.
    pub jobs: Vec<NatsMessage<J>>,
    /// Messages that did not. Never empty in normal operation — a non-empty
    /// `poison` list means producer and consumer disagree about the payload shape.
    pub poison: Vec<PoisonMessage>,
}

// Manual impl: `#[derive(Default)]` would demand `J: Default`, which no job is.
impl<J: Job> Default for Fetched<J> {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            poison: Vec::new(),
        }
    }
}

impl<J: Job> Fetched<J> {
    /// Total messages pulled from the stream, decodable or not.
    pub fn len(&self) -> usize {
        self.jobs.len() + self.poison.len()
    }

    /// Whether the batch pulled nothing at all.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty() && self.poison.is_empty()
    }
}

/// A message whose payload could not be deserialized.
///
/// Carries the raw bytes because they are the only evidence of what the producer
/// actually sent, and the ack handle because someone must still settle it.
pub struct PoisonMessage {
    /// The undecodable payload.
    pub raw: Vec<u8>,
    /// Subject it arrived on.
    pub subject: String,
    /// The deserialization error.
    pub error: String,
    /// The raw NATS message (for term).
    message: async_nats::jetstream::Message,
    /// Stream sequence number.
    pub sequence: u64,
    /// Number of delivery attempts.
    pub delivery_count: u32,
}

impl PoisonMessage {
    /// Mark as permanently failed. Redelivery cannot help: the bytes will not
    /// deserialize on the next attempt either.
    pub async fn term(self) -> Result<(), NatsError> {
        self.message
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(NatsError::Ack)
    }
}

/// A message received from NATS with metadata.
pub struct NatsMessage<J: Job> {
    /// The deserialized job.
    pub job: J,
    /// Subject the message arrived on. Recorded in DLQ entries so a failed job
    /// can be redriven to the right place.
    pub subject: String,
    /// The raw NATS message (for ack/nak).
    message: async_nats::jetstream::Message,
    /// Stream sequence number.
    pub sequence: u64,
    /// Number of delivery attempts.
    pub delivery_count: u32,
}

impl<J: Job> NatsMessage<J> {
    /// Get the job ID.
    pub fn job_id(&self) -> uuid::Uuid {
        self.job.job_id()
    }

    /// Check if this is a redelivery.
    pub fn is_redelivery(&self) -> bool {
        self.delivery_count > 1
    }

    /// Acknowledge the message (successful processing).
    pub async fn ack(self) -> Result<(), NatsError> {
        self.message.ack().await.map_err(NatsError::Ack)
    }

    /// Negative acknowledge (request redelivery).
    pub async fn nak(self) -> Result<(), NatsError> {
        self.message
            .ack_with(async_nats::jetstream::AckKind::Nak(None))
            .await
            .map_err(NatsError::Ack)
    }

    /// Negative acknowledge with delay.
    pub async fn nak_with_delay(self, delay: Duration) -> Result<(), NatsError> {
        self.message
            .ack_with(async_nats::jetstream::AckKind::Nak(Some(delay)))
            .await
            .map_err(NatsError::Ack)
    }

    /// Mark as permanently failed (won't be redelivered).
    pub async fn term(self) -> Result<(), NatsError> {
        self.message
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(NatsError::Ack)
    }
}

/// Stream information.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub stream_name: String,
    pub messages: u64,
    pub bytes: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub consumer_count: i64,
}
