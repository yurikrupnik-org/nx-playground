//! # gRPC Client Library
//!
//! Production-ready gRPC client creation with HTTP/2 tuning, compression,
//! and interceptors for auth/tracing/metrics.
//!
//! This library provides reusable utilities for creating optimized gRPC clients
//! across all services in the monorepo (Tasks, Users, Terran, CodeGraph).
//!
//! ## Features
//!
//! - **Optimized Channel Creation**: Production-ready HTTP/2 tuning validated
//!   through benchmarking (15K+ req/s throughput, sub-4ms P99 latency)
//! - **Compression Support**: Zstd compression with helper functions
//! - **Interceptors**: Auth (Bearer tokens), tracing (request IDs), metrics (counters)
//! - **Retry Logic**: Exponential backoff with jitter for resilient connections
//!
//! ## Quick Start
//!
//! ### Basic Usage
//! ```ignore
//! use grpc_client::{create_channel, configure_client};
//! use rpc::tasks::tasks_service_client::TasksServiceClient;
//!
//! let channel = create_channel("http://[::1]:50051").await?;
//! let client = configure_client(TasksServiceClient::new(channel));
//! ```
//!
//! ### With Interceptors
//! ```ignore
//! use grpc_client::{create_channel, interceptors::*};
//! use rpc::tasks::tasks_service_client::TasksServiceClient;
//!
//! let channel = create_channel("http://[::1]:50051").await?;
//! let auth = AuthInterceptor::bearer("my-token");
//! let tracing = TracingInterceptor::new();
//! let interceptor = compose_interceptors(auth, tracing);
//!
//! let client = TasksServiceClient::with_interceptor(channel, interceptor)
//!     .accept_compressed(tonic::codec::CompressionEncoding::Zstd)
//!     .send_compressed(tonic::codec::CompressionEncoding::Zstd)
//!     .max_decoding_message_size(8 * 1024 * 1024)
//!     .max_encoding_message_size(8 * 1024 * 1024);
//! ```
//!
//! ### With Custom Configuration
//! ```ignore
//! use grpc_client::{create_channel_with_config, ChannelConfig};
//! use std::time::Duration;
//!
//! let config = ChannelConfig::default()
//!     .with_connect_timeout(Duration::from_secs(10))
//!     .with_request_timeout(Duration::from_secs(120))
//!     .with_max_concurrent_streams(200);
//!
//! let channel = create_channel_with_config("http://[::1]:50051", config).await?;
//! ```
//!
//! ### With Retry
//! ```ignore
//! use grpc_client::{create_channel_with_retry, RetryConfig};
//!
//! let retry = RetryConfig::new().with_max_retries(5);
//! let channel = create_channel_with_retry("http://[::1]:50051", Some(retry)).await?;
//! ```

pub mod channel;
pub mod client;
pub mod conversions;
pub mod error;
pub mod interceptors;
pub mod retry;

#[cfg(feature = "server")]
pub mod server;

// Re-export main types and functions for convenience
pub use channel::{
    ChannelConfig, create_channel, create_channel_lazy, create_channel_lazy_with_config,
    create_channel_with_config, create_channel_with_retry,
};
pub use client::{
    ConfigurableClient, configure_client, with_compression, with_limits, with_standard_limits,
    with_zstd_compression,
};
pub use error::{GrpcError, GrpcResult, ToTonicOption, ToTonicResult};
pub use retry::{RetryConfig, retry, retry_with_backoff};

// Re-export interceptors for convenience
pub use interceptors::{
    AuthInterceptor, ComposedInterceptor, MetricsInterceptor, TracingInterceptor,
    compose_interceptors,
};

// Re-export server types (when feature enabled)
#[cfg(feature = "server")]
pub use server::{GrpcServer, ServerConfig, create_health_service};
