//! NATS JetStream consumer for receiving jobs.

use crate::nats::config::WorkerConfig;
use crate::nats::error::NatsError;
use crate::Job;
use async_nats::jetstream::consumer::pull::Config as ConsumerConfig;
use async_nats::jetstream::consumer::AckPolicy;
use async_nats::jetstream::stream::Config as StreamConfig;
use async_nats::jetstream::Context;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

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
                    "Creating stream"
                );

                self.jetstream
                    .create_stream(StreamConfig {
                        name: self.config.stream_name.clone(),
                        subjects: vec![self.config.subject.clone()],
                        max_messages: 100_000,
                        max_age: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
                        ..Default::default()
                    })
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
            .get_consumer::<ConsumerConfig>(&self.config.durable_name)
            .await
        {
            Ok(consumer) => {
                debug!(
                    consumer = %self.config.durable_name,
                    "Consumer already exists"
                );
                Ok(consumer)
            }
            Err(_) => {
                // Create the consumer
                info!(
                    consumer = %self.config.durable_name,
                    stream = %self.config.stream_name,
                    "Creating consumer"
                );

                let consumer = stream
                    .create_consumer(ConsumerConfig {
                        durable_name: Some(self.config.durable_name.clone()),
                        name: Some(self.config.durable_name.clone()),
                        ack_policy: AckPolicy::Explicit,
                        ack_wait: self.config.ack_wait,
                        max_deliver: self.config.max_deliver,
                        filter_subject: self.config.subject.clone(),
                        ..Default::default()
                    })
                    .await?;

                info!(
                    consumer = %self.config.durable_name,
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
    pub async fn fetch<J: Job>(&self, batch_size: usize) -> Result<Vec<NatsMessage<J>>, NatsError> {
        let consumer = self.ensure_consumer().await?;

        let mut messages = consumer
            .fetch()
            .max_messages(batch_size)
            .expires(self.config.fetch_timeout)
            .messages()
            .await?;

        let mut result = Vec::new();

        while let Some(msg) = messages.next().await {
            match msg {
                Ok(message) => {
                    let payload = message.payload.to_vec();
                    match serde_json::from_slice::<J>(&payload) {
                        Ok(job) => {
                            // Get info before consuming message
                            let (sequence, delivery_count) = match message.info() {
                                Ok(info) => (info.stream_sequence, info.delivered as u32),
                                Err(e) => {
                                    warn!(error = %e, "Failed to get message info, using defaults");
                                    (0, 1) // Default values
                                }
                            };
                            result.push(NatsMessage {
                                job,
                                message,
                                sequence,
                                delivery_count,
                            });
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                "Failed to deserialize message, ack-ing"
                            );
                            if let Err(nak_err) = message.ack().await {
                                warn!(error = %nak_err, "Failed to ack bad message");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Error receiving message");
                }
            }
        }

        Ok(result)
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

/// A message received from NATS with metadata.
pub struct NatsMessage<J: Job> {
    /// The deserialized job.
    pub job: J,
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
