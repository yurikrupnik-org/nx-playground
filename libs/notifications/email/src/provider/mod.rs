//! Email provider implementations
//!
//! Available providers (feature-gated):
//!
//! | Provider | Feature | Use Case | Auth Method |
//! |----------|---------|----------|-------------|
//! | [`SmtpProvider`] | `smtp` | Generic SMTP | Username/Password |
//! | [`SendGridProvider`] | `sendgrid` | General purpose | API Key |
//! | [`SesProvider`] | `ses` | AWS environments | IAM / Instance Profile |
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
use eyre::Result;

/// Result of sending an email
#[derive(Debug)]
pub struct SendResult {
    /// Provider-specific message ID
    pub message_id: String,
}

/// Trait for email providers
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// Send an email
    async fn send(&self, email: &Email) -> Result<SendResult>;

    /// Check if the provider is healthy
    async fn health_check(&self) -> Result<()>;

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

// AWS SES provider (feature-gated)
#[cfg(feature = "ses")]
pub mod ses;
#[cfg(feature = "ses")]
pub use ses::SesProvider;
