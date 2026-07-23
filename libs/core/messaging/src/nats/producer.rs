//! NATS JetStream producer for publishing jobs.

use crate::nats::config::StreamConfig;
use crate::nats::error::NatsError;
use crate::Job;
use async_nats::jetstream::Context;
use std::sync::Arc;
use tracing::debug;

/// Producer for publishing jobs to NATS JetStream.
#[derive(Clone)]
pub struct NatsProducer {
    jetstream: Arc<Context>,
    stream_name: String,
    subject: String,
}

impl NatsProducer {
    /// Create a new NATS producer.
    pub fn new(
        jetstream: Context,
        stream_name: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Self {
            jetstream: Arc::new(jetstream),
            stream_name: stream_name.into(),
            subject: subject.into(),
        }
    }

    /// Create a producer from a StreamConfig.
    pub fn from_stream_config<S: StreamConfig>(jetstream: Context) -> Self {
        Self {
            jetstream: Arc::new(jetstream),
            stream_name: S::STREAM_NAME.to_string(),
            subject: S::SUBJECT.to_string(),
        }
    }

    /// Create from an Arc (for sharing).
    pub fn from_arc(
        jetstream: Arc<Context>,
        stream_name: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Self {
            jetstream,
            stream_name: stream_name.into(),
            subject: subject.into(),
        }
    }

    /// Get the stream name.
    pub fn stream_name(&self) -> &str {
        &self.stream_name
    }

    /// Get the subject.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Publish a job to the stream.
    ///
    /// Returns the sequence number of the published message.
    pub async fn send<J: Job>(&self, job: &J) -> Result<u64, NatsError> {
        let job_json = serde_json::to_vec(job)?;

        let ack = self
            .jetstream
            .publish(self.subject.clone(), job_json.into())
            .await?
            .await?;

        debug!(
            stream = %self.stream_name,
            subject = %self.subject,
            sequence = ack.sequence,
            job_id = %job.job_id(),
            "Published job"
        );

        Ok(ack.sequence)
    }

    /// Publish a job to a specific subject (within the stream's subject space).
    pub async fn send_to<J: Job>(&self, subject: &str, job: &J) -> Result<u64, NatsError> {
        let job_json = serde_json::to_vec(job)?;

        let ack = self
            .jetstream
            .publish(subject.to_string(), job_json.into())
            .await?
            .await?;

        debug!(
            stream = %self.stream_name,
            subject = %subject,
            sequence = ack.sequence,
            job_id = %job.job_id(),
            "Published job to subject"
        );

        Ok(ack.sequence)
    }

    /// Publish multiple jobs in batch.
    ///
    /// Returns the sequence numbers of all published messages.
    pub async fn send_batch<J: Job>(&self, jobs: &[J]) -> Result<Vec<u64>, NatsError> {
        let mut sequences = Vec::with_capacity(jobs.len());

        for job in jobs {
            let seq = self.send(job).await?;
            sequences.push(seq);
        }

        debug!(
            stream = %self.stream_name,
            count = sequences.len(),
            "Published batch of jobs"
        );

        Ok(sequences)
    }

    /// Ensure the stream exists, creating it if necessary.
    pub async fn ensure_stream(&self) -> Result<(), NatsError> {
        let mut stream = self
            .jetstream
            .get_stream(&self.stream_name)
            .await?;

        let _info = stream
            .info()
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    struct TestStream;

    impl StreamConfig for TestStream {
        const STREAM_NAME: &'static str = "TEST";
        const CONSUMER_NAME: &'static str = "test-consumer";
        const DLQ_STREAM: &'static str = "TEST_DLQ";
        const SUBJECT: &'static str = "test.>";
    }

    // Note: Real tests require a NATS server
    // Integration tests should be in a separate test file
}
