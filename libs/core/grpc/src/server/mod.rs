//! gRPC Server Builder
//!
//! Provides utilities for creating production-ready gRPC servers
//! with health checks, compression, and standard configurations.
//!
//! ## Quick Start - Simple (Single Service)
//!
//! ```ignore
//! use grpc_client::server::{run_grpc_server, ServerConfig};
//! use rpc::tasks::v1::tasks_service_server::{TasksServiceServer, SERVICE_NAME};
//! use tonic::codec::CompressionEncoding;
//!
//! let config = ServerConfig::from_env()?;
//! let service = TasksServiceServer::new(my_impl)
//!     .accept_compressed(CompressionEncoding::Zstd)
//!     .send_compressed(CompressionEncoding::Zstd);
//!
//! run_grpc_server(config, SERVICE_NAME, service).await?;
//! ```
//!
//! ## Manual Setup (Multiple Services or Custom Configuration)
//!
//! ```ignore
//! use grpc_client::server::{GrpcServer, ServerConfig, HealthReporterExt};
//! use rpc::tasks::v1::tasks_service_server::{TasksServiceServer, SERVICE_NAME};
//! use tonic::codec::CompressionEncoding;
//! use tonic::transport::Server;
//!
//! let config = ServerConfig::from_env()?;
//! let (health_reporter, health_service) = GrpcServer::health_service();
//!
//! health_reporter.set_serving(SERVICE_NAME).await;
//! GrpcServer::log_startup(&config, SERVICE_NAME);
//!
//! Server::builder()
//!     .add_service(health_service)
//!     .add_service(TasksServiceServer::new(my_impl))
//!     .serve(config.socket_addr()?)
//!     .await?;
//! ```

mod builder;
mod config;

pub use builder::{GrpcServer, create_health_service};
pub use config::ServerConfig;
