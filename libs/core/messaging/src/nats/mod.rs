//! NATS JetStream worker framework for background job processing.
//!
//! This module provides a production-ready worker framework using NATS JetStream
//! for durable message queues. It implements the `Job` and `Processor` traits
//! from the core messaging abstractions.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────┐     ┌─────────────────────┐     ┌────────────────┐
//! │   Producer     │────▶│   NATS JetStream    │────▶│     Worker     │
//! │ (NatsProducer) │     │   (Durable Stream)  │     │ (NatsWorker)   │
//! └────────────────┘     └─────────────────────┘     └────────────────┘
//!                                  │                         │
//!                                  ▼                         ▼
//!                        ┌─────────────────┐        ┌────────────────┐
//!                        │   DLQ Stream    │        │   Processor    │
//!                        │ (Dead Letters)  │        │ (Your Logic)   │
//!                        └─────────────────┘        └────────────────┘
//! ```
//!
//! # Key Features
//!
//! - **JetStream Consumers**: Pull-based consumers with ack/nak semantics
//! - **Dead Letter Queue**: Failed messages moved to DLQ after max retries
//! - **Health Endpoints**: K8s-ready liveness/readiness probes
//! - **Prometheus Metrics**: Jobs processed, failed, latency histograms
//! - **Graceful Shutdown**: Drain in-flight messages before exit
//! - **Concurrent Processing**: Process multiple messages in parallel (configurable)
//!
//! # Example
//!
//! ```rust,ignore
//! use messaging::{Job, Processor};
//! use messaging::nats::{NatsWorker, NatsProducer, StreamConfig};
//!
//! // Define your stream
//! struct EmailStream;
//! impl StreamConfig for EmailStream {
//!     const STREAM_NAME: &'static str = "EMAILS";
//!     const CONSUMER_NAME: &'static str = "email-worker";
//!     const DLQ_STREAM: &'static str = "EMAILS_DLQ";
//! }
//!
//! // Create worker
//! let worker = NatsWorker::<EmailJob, EmailProcessor>::new(
//!     nats_client,
//!     processor,
//!     WorkerConfig::from_stream::<EmailStream>(),
//! ).await?;
//!
//! // Run with graceful shutdown
//! worker.run(shutdown_rx).await?;
//! ```

mod config;
mod connection;
mod consumer;
mod dlq;
mod error;
mod health;
pub mod metrics;
mod producer;
mod worker;

pub use config::{StreamConfig, WorkerConfig};
pub use connection::{connect, connect_with_retry, jetstream, jetstream_with_retry, RetryConfig};
pub use consumer::{NatsConsumer, NatsMessage, StreamInfo};
pub use dlq::{DlqEntry, DlqManager, DlqStats};
pub use error::NatsError;
pub use health::{HealthServer, HealthState, HealthStatus};
pub use metrics::{init_metrics, NatsMetrics};
pub use producer::NatsProducer;
pub use worker::NatsWorker;
