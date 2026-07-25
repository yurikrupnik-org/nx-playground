//! Email provider implementations
//!
//! Available providers (feature-gated):
//!
//! | Provider | Feature | Use Case | Auth Method |
//! |----------|---------|----------|-------------|
//! | [`SmtpProvider`] | `smtp` | Generic SMTP | Username/Password |
//! | [`SendGridProvider`] | `sendgrid` | General purpose | API Key |
//! | [`MockSmtpProvider`] | (always) | Testing | None |
//!
//! ## Gmail via SMTP
//!
//! For Gmail integration, use [`SmtpProvider`] (requires `smtp` feature) with these helpers:
//! - [`SmtpProvider::gmail_app_password()`] - Personal Gmail with app password
//! - [`SmtpProvider::gmail_relay()`] - Workspace SMTP relay (IP allowlist)
//! - [`SmtpProvider::gmail_relay_with_auth()`] - Workspace SMTP relay with auth

use crate::models::Email;
use async_trait::async_trait;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Error returned by an email provider, classified by retryability.
///
/// Providers map their transport-specific failures onto these variants so the
/// processor can decide retry behavior from the *type*, never from message text:
/// - HTTP: 4xx is [`Permanent`](Self::Permanent) except 408/429; 5xx and
///   transport failures are [`Transient`](Self::Transient); 429 is
///   [`RateLimited`](Self::RateLimited).
/// - SMTP: lettre's permanent/transient response classification is used directly.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Permanent failure - retrying will not help (invalid request, auth,
    /// rejected recipient).
    #[error("permanent provider error: {message}")]
    Permanent {
        message: String,
        #[source]
        source: Option<BoxError>,
    },

    /// Transient failure - retrying may succeed (network error, timeout, 5xx).
    #[error("transient provider error: {message}")]
    Transient {
        message: String,
        #[source]
        source: Option<BoxError>,
    },

    /// Rate limited by the provider, with an optional retry-after hint.
    #[error("provider rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after_ms: Option<u64>,
    },
}

impl ProviderError {
    /// Create a permanent error.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent {
            message: message.into(),
            source: None,
        }
    }

    /// Create a permanent error preserving its source.
    pub fn permanent_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Permanent {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create a transient error.
    pub fn transient(message: impl Into<String>) -> Self {
        Self::Transient {
            message: message.into(),
            source: None,
        }
    }

    /// Create a transient error preserving its source.
    pub fn transient_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Transient {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create a rate-limited error.
    pub fn rate_limited(message: impl Into<String>, retry_after_ms: Option<u64>) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after_ms,
        }
    }
}

impl From<ProviderError> for messaging::ProcessingError {
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::Permanent { message, source } => {
                messaging::ProcessingError::Permanent { message, source }
            }
            ProviderError::Transient { message, source } => {
                messaging::ProcessingError::Transient { message, source }
            }
            ProviderError::RateLimited {
                message,
                retry_after_ms,
            } => messaging::ProcessingError::RateLimited {
                message,
                retry_after_ms,
            },
        }
    }
}

/// Result of sending an email
#[derive(Debug)]
pub struct SendResult {
    /// Provider-specific message ID
    pub message_id: String,
}

/// Trait for email providers.
///
/// `async_trait` is kept deliberately: `messaging::Processor::process` must
/// return a `Send` future, so a generic `P: EmailProvider` must guarantee
/// `Send` futures too - native async-fn-in-trait cannot express that without
/// return-type-notation bounds. It also keeps the trait dyn-compatible.
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// Send an email
    async fn send(&self, email: &Email) -> Result<SendResult, ProviderError>;

    /// Check if the provider is healthy
    async fn health_check(&self) -> Result<(), ProviderError>;

    /// Get provider name
    fn name(&self) -> &'static str;
}

// Mock provider (always available for testing)
pub mod mock;
pub use mock::MockSmtpProvider;

// SMTP provider (feature-gated)
#[cfg(feature = "smtp")]
pub mod smtp;
#[cfg(feature = "smtp")]
pub use smtp::{SmtpConfig, SmtpProvider};

// SendGrid provider (feature-gated)
#[cfg(feature = "sendgrid")]
pub mod sendgrid;
#[cfg(feature = "sendgrid")]
pub use sendgrid::SendGridProvider;
