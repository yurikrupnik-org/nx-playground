//! Common messaging abstractions for job queues and event streaming.
//!
//! This library provides backend-agnostic traits and types for:
//! - **Job Queues**: Durable background job processing
//! - **Pub/Sub**: Event streaming and fan-out patterns
//! - **Request/Reply**: Synchronous service-to-service communication
//!
//! # Features
//!
//! - `nats` - Enable NATS JetStream backend support
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────────────────────────────┐
//! │   Your Code     │     │            Backends                 │
//! │                 │     │                                     │
//! │  ┌───────────┐  │     │  ┌───────────────┐                  │
//! │  │ EmailJob  │──│─────│─▶│ messaging::nats│                 │
//! │  └───────────┘  │     │  │(NATS JetStream)│                 │
//! │  ┌───────────┐  │     │  └───────────────┘                  │
//! │  │ EmailProc │──│─────│─▶       ▲                           │
//! │  └───────────┘  │     │         │                           │
//! │                 │     │    Same traits, different backends  │
//! └─────────────────┘     └─────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use messaging::{Job, Processor, QueueConfig};
//!
//! // Define your job (works with any backend)
//! #[derive(Clone, Serialize, Deserialize)]
//! struct EmailJob {
//!     id: Uuid,
//!     to: String,
//!     subject: String,
//!     retry_count: u32,
//! }
//!
//! impl Job for EmailJob {
//!     fn job_id(&self) -> String { self.id.to_string() }
//!     fn retry_count(&self) -> u32 { self.retry_count }
//!     fn with_retry(&self) -> Self { Self { retry_count: self.retry_count + 1, ..self.clone() } }
//! }
//!
//! // Define your processor (works with any backend)
//! struct EmailProcessor { ... }
//!
//! #[async_trait]
//! impl Processor<EmailJob> for EmailProcessor {
//!     async fn process(&self, job: &EmailJob) -> Result<(), ProcessingError> { ... }
//!     fn name(&self) -> &'static str { "email_processor" }
//! }
//!
//! // Use with NATS JetStream (requires "nats" feature)
//! #[cfg(feature = "nats")]
//! {
//!     use messaging::nats::{NatsWorker, WorkerConfig};
//!     let worker = NatsWorker::new(jetstream, processor, config).await?;
//! }
//! ```

// Core modules (always available)
mod config;
mod error;
mod event;
mod job;
pub mod jobs;
mod processor;

// Core exports
pub use config::{BackoffStrategy, QueueConfig, QueueDef, RetryPolicy};
pub use error::{ErrorCategory, ProcessingError};
pub use event::{JobEvent, ProcessResult};
pub use job::{Job, JobPriority};
pub use jobs::{DbOpEvent, DbOperation};
pub use processor::{FailingProcessor, NoOpProcessor, Processor};

// NATS module (feature-gated)
#[cfg(feature = "nats")]
pub mod nats;

// Re-export common NATS types at crate root for convenience
#[cfg(feature = "nats")]
pub use nats::{
    DlqManager, HealthServer, NatsConsumer, NatsError, NatsMetrics, NatsProducer, NatsWorker,
    StreamConfig, WorkerConfig,
};
