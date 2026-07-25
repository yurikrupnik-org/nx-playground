//! Error types for NATS worker.

use crate::ErrorCategory;
use async_nats::jetstream::consumer::pull::BatchError;
use async_nats::jetstream::context::{
    CreateStreamError, CreateStreamErrorKind, GetStreamError, GetStreamErrorKind, PublishError,
    PublishErrorKind, RequestError,
};
use async_nats::jetstream::stream::{ConsumerError, ConsumerErrorKind};
use thiserror::Error;

/// Error that can occur in NATS worker operations.
#[derive(Debug, Error)]
pub enum NatsError {
    /// NATS connection error
    #[error("NATS connection error: {0}")]
    Connection(#[from] async_nats::ConnectError),

    /// Stream lookup failed
    #[error("stream lookup failed: {0}")]
    GetStream(#[from] GetStreamError),

    /// Stream creation failed
    #[error("stream creation failed: {0}")]
    CreateStream(#[from] CreateStreamError),

    /// Stream info request failed
    #[error("stream info request failed: {0}")]
    StreamInfo(#[from] RequestError),

    /// Consumer lookup/creation failed
    #[error("consumer error: {0}")]
    Consumer(#[from] ConsumerError),

    /// Batch fetch failed
    #[error("batch fetch failed: {0}")]
    Batch(#[from] BatchError),

    /// Receiving a message from an open batch failed
    #[error("message receive failed: {0}")]
    Receive(#[source] async_nats::Error),

    /// Publish failed (or the server did not ack)
    #[error("publish failed: {0}")]
    Publish(#[from] PublishError),

    /// Message acknowledgement (ack/nak/term) failed
    #[error("message acknowledgement failed: {0}")]
    Ack(#[source] async_nats::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Processing error
    #[error("processing error: {0}")]
    Processing(#[from] crate::ProcessingError),

    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),
}

impl NatsError {
    /// Get the error category for retry decisions.
    ///
    /// Retryability is decided from the typed error variants (and their
    /// kinds), never from message-string sniffing.
    pub fn category(&self) -> ErrorCategory {
        match self {
            // Connectivity problems are always worth retrying.
            NatsError::Connection(_)
            | NatsError::Batch(_)
            | NatsError::Receive(_)
            | NatsError::Ack(_)
            | NatsError::StreamInfo(_) => ErrorCategory::Transient,

            NatsError::GetStream(e) => match e.kind() {
                GetStreamErrorKind::EmptyName | GetStreamErrorKind::InvalidStreamName => {
                    ErrorCategory::Permanent
                }
                GetStreamErrorKind::Request | GetStreamErrorKind::JetStream(_) => {
                    ErrorCategory::Transient
                }
            },

            NatsError::CreateStream(e) => match e.kind() {
                CreateStreamErrorKind::EmptyStreamName
                | CreateStreamErrorKind::InvalidStreamName
                | CreateStreamErrorKind::DomainAndExternalSet
                | CreateStreamErrorKind::NotFound => ErrorCategory::Permanent,
                CreateStreamErrorKind::JetStreamUnavailable
                | CreateStreamErrorKind::JetStream(_)
                | CreateStreamErrorKind::TimedOut
                | CreateStreamErrorKind::Response
                | CreateStreamErrorKind::ResponseParse => ErrorCategory::Transient,
            },

            NatsError::Consumer(e) => match e.kind() {
                ConsumerErrorKind::InvalidConsumerType | ConsumerErrorKind::InvalidName => {
                    ErrorCategory::Permanent
                }
                _ => ErrorCategory::Transient,
            },

            NatsError::Publish(e) => match e.kind() {
                PublishErrorKind::StreamNotFound
                | PublishErrorKind::WrongLastMessageId
                | PublishErrorKind::WrongLastSequence => ErrorCategory::Permanent,
                _ => ErrorCategory::Transient,
            },

            // Bad payloads and bad configuration never fix themselves.
            NatsError::Serialization(_) | NatsError::Config(_) => ErrorCategory::Permanent,

            // Processing errors carry their own category.
            NatsError::Processing(e) => e.category(),
        }
    }

    /// Check if this error should be retried.
    pub fn should_retry(&self, retry_count: u32) -> bool {
        self.category().should_retry(retry_count)
    }

    /// Calculate backoff delay for retry.
    pub fn backoff_delay_ms(&self, retry_count: u32) -> u64 {
        self.category().backoff_delay_ms(retry_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_category() {
        let batch_err = NatsError::Batch(BatchError::new(
            async_nats::jetstream::consumer::pull::BatchErrorKind::Pull,
        ));
        assert_eq!(batch_err.category(), ErrorCategory::Transient);

        let serialization_err =
            NatsError::Serialization(serde_json::from_str::<String>("invalid").unwrap_err());
        assert_eq!(serialization_err.category(), ErrorCategory::Permanent);

        let config_err = NatsError::Config("bad config".to_string());
        assert_eq!(config_err.category(), ErrorCategory::Permanent);

        let publish_not_found =
            NatsError::Publish(PublishError::new(PublishErrorKind::StreamNotFound));
        assert_eq!(publish_not_found.category(), ErrorCategory::Permanent);

        let publish_timeout = NatsError::Publish(PublishError::new(PublishErrorKind::TimedOut));
        assert_eq!(publish_timeout.category(), ErrorCategory::Transient);
    }

    #[test]
    fn test_should_retry() {
        let transient = NatsError::Publish(PublishError::new(PublishErrorKind::TimedOut));
        assert!(transient.should_retry(0));
        assert!(transient.should_retry(2));
        assert!(!transient.should_retry(3));

        let permanent = NatsError::Config("invalid".to_string());
        assert!(!permanent.should_retry(0));
    }
}
