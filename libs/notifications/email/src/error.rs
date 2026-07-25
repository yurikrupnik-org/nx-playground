//! Error types for the notification service.

use crate::provider::ProviderError;
use crate::templates::TemplateError;

/// Result type for notification operations.
pub type NotificationResult<T> = Result<T, NotificationError>;

/// Errors that can occur in the notification service.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// NATS queue operation failed
    #[error("queue error")]
    Queue(#[from] messaging::nats::NatsError),

    /// Serialization/deserialization error
    #[error("serialization error")]
    Serialization(#[from] serde_json::Error),

    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),

    /// Invalid input
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Template registration or rendering failed
    #[error("template error")]
    Template(#[from] TemplateError),

    /// Provider error (SMTP, SendGrid, etc.)
    #[error("provider error")]
    Provider(#[from] ProviderError),
}
