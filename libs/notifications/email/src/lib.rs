//! Email notification library with NATS JetStream support
//!
//! This library provides a complete email notification system that works with
//! NATS JetStream via the `messaging` crate.
//!
//! ## Features
//!
//! - `smtp` (default) - Enable SMTP provider via lettre
//! - `sendgrid` - Enable SendGrid HTTP API provider
//!
//! ## Components
//!
//! - **Stream Processing**: `EmailJob`, `EmailNatsStream`, `EmailProcessor`
//! - **Email Models**: `Email`, `EmailEvent`, `EmailPriority` for email data
//! - **Providers**: SMTP (feature-gated), SendGrid (feature-gated), and Mock (always available)
//! - **Templates**: Handlebars-based `TemplateEngine` for email templating
//!
//! ## Usage with NATS JetStream
//!
//! ```ignore
//! use email::{EmailJob, EmailNatsStream, EmailProcessor};
//! use messaging::{NatsWorker, WorkerConfig};
//!
//! let processor = EmailProcessor::new(provider, templates);
//! let config = WorkerConfig::from_stream::<EmailNatsStream>();
//! let worker = NatsWorker::new(jetstream, processor, config).await?;
//! worker.run(shutdown_rx).await?;
//! ```

// Core modules
pub mod error;
pub mod job;
pub mod models;
pub mod processor;
pub mod provider;
pub mod service;
pub mod streams;
pub mod templates;

// Re-export main types
pub use error::{NotificationError, NotificationResult};
pub use job::{EmailJob, EmailType};
pub use models::{Email, EmailEvent, EmailPriority, EmailStatus};
pub use processor::EmailProcessor;
pub use streams::EmailNatsStream;
pub use templates::{InMemoryTemplateStore, TemplateEngine, TemplateStore};

// Service exports (for API integration)
pub use service::{NotificationService, NotificationServiceConfig, WelcomeEmailData};

// Provider re-exports with feature gates
pub use provider::{EmailProvider, MockSmtpProvider, SendResult};

#[cfg(feature = "smtp")]
pub use provider::{SmtpConfig, SmtpProvider};

#[cfg(feature = "sendgrid")]
pub use provider::SendGridProvider;

#[cfg(feature = "ses")]
pub use provider::SesProvider;
