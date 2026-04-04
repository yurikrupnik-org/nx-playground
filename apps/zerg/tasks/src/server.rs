//! gRPC server initialization and lifecycle management
//!
//! This module handles all server setup:
//! - Tracing initialization
//! - Database connection (PostgreSQL for tasks)
//! - Qdrant connection (for vector service)
//! - Service creation
//! - gRPC server configuration and startup
//! - Health check service (grpc.health.v1.Health)

use std::sync::Arc;

use core_config::{Environment, FromEnv};
use database::postgres::PostgresConfig;
use domain_tasks::{PgTaskRepository, TaskService};
use domain_vector::{OpenAIProvider, QdrantConfig, QdrantRepository, VectorService};
use eyre::{Result, WrapErr};
use grpc_client::server::{GrpcServer, ServerConfig, create_health_service};
use rpc::tasks::tasks_service_server::{SERVICE_NAME as TASKS_SERVICE, TasksServiceServer};
use rpc::vector::v1::vector_service_server::{SERVICE_NAME as VECTOR_SERVICE, VectorServiceServer};
use tonic::codec::CompressionEncoding;
use tonic::transport::Server;
use tracing::info;

use crate::service::TasksServiceImpl;
use crate::vector_service::VectorServiceImpl;

/// Run the gRPC server
///
/// This is the main entry point for server initialization. It:
/// 1. Sets up structured logging (env-aware: JSON for prod, pretty for dev)
/// 2. Connects to PostgreSQL (for tasks)
/// 3. Connects to Qdrant (for vector service)
/// 4. Creates the repository and service layers
/// 5. Starts the gRPC server with compression enabled
///
/// # Errors
///
/// Returns an error if:
/// - Database configuration is invalid
/// - Database connection fails
/// - Qdrant connection fails
/// - Server binding fails
/// - Server runtime encounters an error
pub async fn run() -> Result<()> {
    // Initialize tracing (env-aware: JSON for prod, pretty for dev)
    let environment = Environment::from_env();
    core_config::tracing::init_tracing(&environment);

    // Load gRPC server configuration
    let server_config = ServerConfig::from_env().wrap_err("Failed to load server configuration")?;

    // Connect to PostgreSQL
    let db_config = PostgresConfig::from_env().wrap_err("Failed to load database configuration")?;
    info!("Connecting to PostgreSQL...");
    let db = database::postgres::connect_from_config_with_retry(db_config, None)
        .await
        .wrap_err("Failed to connect to database")?;
    info!("Connected to PostgreSQL");

    // Connect to Qdrant
    let qdrant_config = QdrantConfig::from_env().wrap_err("Failed to load Qdrant configuration")?;
    info!("Connecting to Qdrant...");
    let qdrant_repository = QdrantRepository::new(qdrant_config)
        .await
        .wrap_err("Failed to connect to Qdrant")?;
    info!("Connected to Qdrant");

    // Create tasks service
    let task_repository = PgTaskRepository::new(db);
    let task_service = TaskService::new(task_repository);
    let tasks_grpc = TasksServiceServer::new(TasksServiceImpl::new(task_service))
        .accept_compressed(CompressionEncoding::Zstd)
        .send_compressed(CompressionEncoding::Zstd);

    // Create a vector service (with an optional embedding provider)
    let vector_service = VectorService::new(qdrant_repository);
    let vector_service = if let Ok(provider) = OpenAIProvider::from_env() {
        info!("OpenAI embedding provider configured");
        vector_service.with_embedding_provider(Arc::new(provider))
    } else {
        info!("No embedding provider configured");
        vector_service
    };
    let vector_grpc = VectorServiceServer::new(VectorServiceImpl::new(vector_service))
        .accept_compressed(CompressionEncoding::Zstd)
        .send_compressed(CompressionEncoding::Zstd);

    // Create health service
    let (health_reporter, health_service) = create_health_service();
    let services = [TASKS_SERVICE, VECTOR_SERVICE];
    GrpcServer::setup_health_multiple(&health_reporter, &services).await;
    GrpcServer::log_startup_multiple(&server_config, &services);

    // Build and start a server
    let addr = server_config
        .socket_addr()
        .wrap_err("Invalid server address")?;

    Server::builder()
        .add_service(health_service)
        .add_service(tasks_grpc)
        .add_service(vector_grpc)
        .serve(addr)
        .await
        .wrap_err("gRPC server failed")?;

    Ok(())
}
