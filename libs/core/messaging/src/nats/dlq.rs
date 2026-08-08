//! Dead Letter Queue management for NATS.

use crate::nats::consumer::StreamInfo;
use crate::nats::error::NatsError;
use crate::Job;
use async_nats::jetstream::consumer::pull::Config as PullConsumerConfig;
use async_nats::jetstream::consumer::DeliverPolicy;
use async_nats::jetstream::stream::{Config as StreamConfig, RetentionPolicy};
use async_nats::jetstream::Context;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use chrono::Utc;
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Manager for Dead Letter Queue operations.
pub struct DlqManager {
    jetstream: Arc<Context>,
    dlq_stream: String,
    dlq_subject: String,
}

impl DlqManager {
    /// Create a new DLQ manager.
    pub fn new(jetstream: Arc<Context>, dlq_stream: &str) -> Self {
        let dlq_subject = format!("{}.>", dlq_stream.to_lowercase());
        Self {
            jetstream,
            dlq_stream: dlq_stream.to_string(),
            dlq_subject,
        }
    }

    /// Ensure the DLQ stream exists.
    ///
    /// Retention is **explicitly** `Limits`, and must stay that way. A DLQ is read
    /// repeatedly — inspect, diagnose, redrive — and `WorkQueue` retention deletes a
    /// message on first ack, destroying the evidence the queue exists to preserve.
    /// This is why the DLQ deliberately does NOT go through
    /// [`crate::nats::consumer::stream_config_for`]: that derives retention from the
    /// parent stream's [`StreamKind`](crate::nats::config::StreamKind), so an
    /// `EMAILS_DLQ` would inherit `WorkQueue` from `EMAILS`.
    pub async fn ensure_stream(&self) -> Result<(), NatsError> {
        match self.jetstream.get_stream(&self.dlq_stream).await {
            Ok(_) => {
                debug!(stream = %self.dlq_stream, "DLQ stream already exists");
                Ok(())
            }
            Err(_) => {
                info!(stream = %self.dlq_stream, "Creating DLQ stream");

                self.jetstream
                    .create_stream(StreamConfig {
                        name: self.dlq_stream.clone(),
                        subjects: vec![self.dlq_subject.clone()],
                        max_messages: 10_000,
                        max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
                        retention: RetentionPolicy::Limits,
                        ..Default::default()
                    })
                    .await?;

                info!(stream = %self.dlq_stream, "DLQ stream created");
                Ok(())
            }
        }
    }

    /// The subject failed entries are published to.
    fn failed_subject(&self) -> String {
        format!("{}.failed", self.dlq_stream.to_lowercase())
    }

    /// Move a failed job to the DLQ.
    ///
    /// `original_subject` is what makes [`redrive`](Self::redrive) possible: a stream
    /// like `EMAILS` fans across `emails.welcome`, `emails.password_reset` and five
    /// others, and the entry is unroutable without knowing which one it came from.
    pub async fn move_to_dlq<J: Job>(
        &self,
        job: &J,
        original_subject: &str,
        error: &str,
        original_sequence: u64,
        delivery_count: u32,
    ) -> Result<u64, NatsError> {
        let entry = DlqEntry {
            job_id: Some(job.job_id()),
            job_type: job.job_type().to_string(),
            payload: DlqPayload::Job(serde_json::to_value(job)?),
            original_subject: original_subject.to_string(),
            error: error.to_string(),
            original_sequence,
            delivery_count,
            failed_at: Utc::now(),
        };

        self.publish(entry).await
    }

    /// Move an undeserializable ("poison") message to the DLQ.
    ///
    /// The payload could not be decoded, so there is no job id and no typed value —
    /// only the raw bytes, kept base64-encoded so the entry stays valid JSON. This is
    /// the schema-evolution case: a producer shipped a field the consumer cannot read.
    /// Terminating without capturing the bytes would delete the only evidence.
    pub async fn move_poison_to_dlq(
        &self,
        raw: &[u8],
        original_subject: &str,
        error: &str,
        original_sequence: u64,
        delivery_count: u32,
    ) -> Result<u64, NatsError> {
        let entry = DlqEntry {
            job_id: None,
            job_type: "<undeserializable>".to_string(),
            payload: DlqPayload::Raw {
                base64: BASE64.encode(raw),
            },
            original_subject: original_subject.to_string(),
            error: error.to_string(),
            original_sequence,
            delivery_count,
            failed_at: Utc::now(),
        };

        self.publish(entry).await
    }

    async fn publish(&self, entry: DlqEntry) -> Result<u64, NatsError> {
        let job_id = entry.job_id;
        let payload = serde_json::to_vec(&entry)?;

        let ack = self
            .jetstream
            .publish(self.failed_subject(), payload.into())
            .await?
            .await?;

        debug!(
            job_id = ?job_id,
            sequence = ack.sequence,
            "Moved job to DLQ"
        );

        Ok(ack.sequence)
    }

    /// Get DLQ stream info.
    pub async fn stream_info(&self) -> Result<StreamInfo, NatsError> {
        let mut stream = self.jetstream.get_stream(&self.dlq_stream).await?;

        let info = stream.info().await?;

        Ok(StreamInfo {
            stream_name: self.dlq_stream.clone(),
            messages: info.state.messages,
            bytes: info.state.bytes,
            first_sequence: info.state.first_sequence,
            last_sequence: info.state.last_sequence,
            consumer_count: info.state.consumer_count as i64,
        })
    }

    /// Get DLQ statistics.
    pub async fn stats(&self) -> Result<DlqStats, NatsError> {
        let info = self.stream_info().await?;

        Ok(DlqStats {
            stream_name: self.dlq_stream.clone(),
            total_messages: info.messages,
            total_bytes: info.bytes,
        })
    }

    /// Republish DLQ entries back onto their original subjects.
    ///
    /// **Operator-triggered, never a running consumer.** A message reaches the DLQ
    /// only after automatic retry is exhausted, so a daemon that reprocesses it is a
    /// slower retry loop with the same outcome — and for a poison pill, an infinite
    /// one across two streams. Redrive is what you run *after* deploying the fix.
    ///
    /// Reads up to `limit` entries starting at `start_sequence` and republishes each
    /// [`DlqPayload::Job`] to its recorded `original_subject`.
    /// [`DlqPayload::Raw`] entries are skipped and counted: nothing can decode them,
    /// so replaying the bytes would only re-poison the source stream.
    ///
    /// Entries are **not** deleted — `Limits` retention keeps them for the audit
    /// trail, and a redrive that half-succeeds must stay diagnosable. Acking below is
    /// only what settles the throwaway consumer; under `Limits` it removes nothing.
    ///
    /// Reading happens through an **ephemeral** consumer rather than sequence-by-
    /// sequence `direct_get`: sequences are sparse once `max_age` expires entries, and
    /// `direct_get` additionally requires `allow_direct` on the stream, which a DLQ
    /// created before this method existed will not have.
    pub async fn redrive(&self, start_sequence: u64, limit: usize) -> Result<Redriven, NatsError> {
        let stream = self.jetstream.get_stream(&self.dlq_stream).await?;
        let mut report = Redriven::default();

        let consumer = stream
            .create_consumer(PullConsumerConfig {
                deliver_policy: DeliverPolicy::ByStartSequence { start_sequence },
                // Ephemeral: no durable_name, and the server reaps it shortly after
                // this call returns. A redrive must not leave a cursor behind.
                inactive_threshold: Duration::from_secs(30),
                ..Default::default()
            })
            .await?;

        let mut batch = consumer
            .fetch()
            .max_messages(limit)
            .expires(Duration::from_secs(5))
            .messages()
            .await?;

        while let Some(msg) = batch.next().await {
            let msg = match msg {
                Ok(msg) => msg,
                Err(e) => {
                    warn!(error = %e, "Error reading DLQ entry");
                    continue;
                }
            };

            match serde_json::from_slice::<DlqEntry>(&msg.payload) {
                Ok(entry) => match entry.payload {
                    DlqPayload::Job(job_data) => {
                        let payload = serde_json::to_vec(&job_data)?;
                        self.jetstream
                            .publish(entry.original_subject.clone(), payload.into())
                            .await?
                            .await?;

                        info!(
                            job_id = ?entry.job_id,
                            subject = %entry.original_subject,
                            "Redriven from DLQ"
                        );
                        report.republished += 1;
                    }
                    DlqPayload::Raw { .. } => {
                        warn!(
                            subject = %entry.original_subject,
                            error = %entry.error,
                            "Skipping poison entry: replaying it would re-poison the stream"
                        );
                        report.skipped += 1;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "DLQ entry is not a DlqEntry, skipping");
                    report.skipped += 1;
                }
            }

            msg.ack().await.map_err(NatsError::Ack)?;
        }

        Ok(report)
    }
}

/// What a DLQ entry carries. A job that failed processing keeps its decoded form;
/// a message that could not be deserialized at all keeps only its bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DlqPayload {
    /// The job, decoded. Redrivable.
    Job(serde_json::Value),
    /// Raw bytes that failed to deserialize. Not redrivable.
    Raw { base64: String },
}

/// Entry stored in the Dead Letter Queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqEntry {
    /// Job ID. `None` for a poison message, whose payload never decoded.
    pub job_id: Option<uuid::Uuid>,
    /// Concrete job type, from `Job::job_type()`.
    pub job_type: String,
    /// The failed message, decoded or raw.
    pub payload: DlqPayload,
    /// Subject the message was originally published to. Required for redrive.
    pub original_subject: String,
    /// Error message that caused the failure
    pub error: String,
    /// Original stream sequence number
    pub original_sequence: u64,
    /// Server-side delivery attempt count when the job was given up on.
    pub delivery_count: u32,
    /// When the job failed
    pub failed_at: chrono::DateTime<Utc>,
}

/// Outcome of a [`DlqManager::redrive`] run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redriven {
    /// Entries republished to their original subject.
    pub republished: u64,
    /// Entries left in place — poison, or not a `DlqEntry` at all.
    pub skipped: u64,
}

/// DLQ statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqStats {
    pub stream_name: String,
    pub total_messages: u64,
    pub total_bytes: u64,
}
