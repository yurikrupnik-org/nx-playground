//! Dead Letter Queue management for NATS.

use crate::nats::consumer::StreamInfo;
use crate::nats::error::NatsError;
use crate::Job;
use async_nats::jetstream::stream::Config as StreamConfig;
use async_nats::jetstream::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

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
                        ..Default::default()
                    })
                    .await?;

                info!(stream = %self.dlq_stream, "DLQ stream created");
                Ok(())
            }
        }
    }

    /// Move a failed job to the DLQ.
    pub async fn move_to_dlq<J: Job>(
        &self,
        job: &J,
        error: &str,
        original_sequence: u64,
    ) -> Result<u64, NatsError> {
        let entry = DlqEntry {
            job_id: job.job_id(),
            job_data: serde_json::to_value(job)?,
            error: error.to_string(),
            original_sequence,
            retry_count: job.retry_count(),
            failed_at: Utc::now(),
        };

        let payload = serde_json::to_vec(&entry)?;
        let subject = format!("{}.failed", self.dlq_stream.to_lowercase());

        let ack = self
            .jetstream
            .publish(subject, payload.into())
            .await?
            .await?;

        debug!(
            job_id = %job.job_id(),
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
}

/// Entry stored in the Dead Letter Queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqEntry {
    /// Job ID
    pub job_id: uuid::Uuid,
    /// Full job data as JSON
    pub job_data: serde_json::Value,
    /// Error message that caused the failure
    pub error: String,
    /// Original stream sequence number
    pub original_sequence: u64,
    /// Retry count when the job failed
    pub retry_count: u32,
    /// When the job failed
    pub failed_at: chrono::DateTime<Utc>,
}

/// DLQ statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqStats {
    pub stream_name: String,
    pub total_messages: u64,
    pub total_bytes: u64,
}
