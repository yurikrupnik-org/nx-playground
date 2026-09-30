//! gRPC server initialization for the vector service.

use std::sync::Arc;

use core_config::Environment;
use domain_vector::{OpenAIProvider, QdrantConfig, QdrantRepository, VectorService};
use eyre::{Result, WrapErr};
use grpc_client::server::{GrpcServer, ServerConfig, create_health_service};
use rpc::vector::v1::vector_service_server::{SERVICE_NAME as VECTOR_SERVICE, VectorServiceServer};
use tonic::codec::CompressionEncoding;
use tonic::transport::Server;
use tracing::info;

use crate::vector_service::VectorServiceImpl;

/// Run the vector gRPC server.
pub async fn run() -> Result<()> {
    let environment = Environment::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&environment, core_config::app_info!());

    let server_config = ServerConfig::from_env().wrap_err("Failed to load server configuration")?;

    info!("Connecting to Qdrant...");
    let qdrant_config = QdrantConfig::from_env().wrap_err("Failed to load Qdrant configuration")?;
    let qdrant_repository = QdrantRepository::new(qdrant_config)
        .await
        .wrap_err("Failed to connect to Qdrant")?;
    info!("Connected to Qdrant");

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

    let (health_reporter, health_service) = create_health_service();
    let services = [VECTOR_SERVICE];
    GrpcServer::setup_health_multiple(&health_reporter, &services).await;
    GrpcServer::log_startup_multiple(&server_config, &services);

    GrpcServer::serve_with_shutdown(
        server_config.socket_addr(),
        Server::builder()
            .add_service(health_service)
            .add_service(vector_grpc),
    )
    .await
    .wrap_err("gRPC server failed")?;

    Ok(())
}
