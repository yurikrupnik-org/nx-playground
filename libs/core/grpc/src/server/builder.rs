//! gRPC Server utilities.
//!
//! Provides helpers for building production-ready gRPC servers.

use super::config::ServerConfig;
use tracing::info;

/// Helper for creating gRPC servers with health checks.
///
/// # Example
///
/// ```ignore
/// use grpc_client::server::{GrpcServer, ServerConfig};
/// use rpc::tasks::tasks_service_server::{TasksServiceServer, SERVICE_NAME};
/// use tonic::codec::CompressionEncoding;
/// use tonic::transport::Server;
///
/// let config = ServerConfig::from_env()?;
///
/// // Get health service components
/// let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
///
/// // Mark services as serving
/// health_reporter.set_service_status(SERVICE_NAME, tonic_health::ServingStatus::Serving).await;
/// health_reporter.set_service_status("", tonic_health::ServingStatus::Serving).await;
///
/// // Log startup
/// GrpcServer::log_startup(&config, SERVICE_NAME);
///
/// // Build and serve
/// Server::builder()
///     .add_service(health_service)
///     .add_service(
///         TasksServiceServer::new(my_service)
///             .accept_compressed(CompressionEncoding::Zstd)
///             .send_compressed(CompressionEncoding::Zstd)
///     )
///     .serve(config.socket_addr()?)
///     .await?;
/// ```
pub struct GrpcServer;

impl GrpcServer {
    /// Log server startup information for a single service.
    pub fn log_startup(config: &ServerConfig, service_name: &str) {
        Self::log_startup_multiple(config, &[service_name]);
    }

    /// Log server startup information for multiple services.
    ///
    /// # Example
    ///
    /// ```ignore
    /// GrpcServer::log_startup_multiple(&config, &[TASKS_SERVICE, VECTOR_SERVICE]);
    /// ```
    pub fn log_startup_multiple(config: &ServerConfig, service_names: &[&str]) {
        info!(
            addr = %config.socket_addr(),
            services = ?service_names,
            compression = config.enable_compression,
            "gRPC server starting"
        );

        if config.enable_compression {
            info!("Zstd compression enabled for optimal performance");
        }

        info!("Health check service enabled (grpc.health.v1.Health)");
    }

    /// Set up health reporting for a single service.
    ///
    /// Marks both the specific service and empty service name as serving
    /// (empty is used by k8s default health checks).
    pub async fn setup_health(
        health_reporter: &tonic_health::server::HealthReporter,
        service_name: &str,
    ) {
        Self::setup_health_multiple(health_reporter, &[service_name]).await;
    }

    /// Set up health reporting for multiple services.
    ///
    /// Marks all specified services and the empty service name as serving
    /// (empty is used by k8s default health checks).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use rpc::tasks::tasks_service_server::SERVICE_NAME as TASKS_SERVICE;
    /// use rpc::vector::vector_service_server::SERVICE_NAME as VECTOR_SERVICE;
    ///
    /// GrpcServer::setup_health_multiple(
    ///     &health_reporter,
    ///     &[TASKS_SERVICE, VECTOR_SERVICE],
    /// ).await;
    /// ```
    pub async fn setup_health_multiple(
        health_reporter: &tonic_health::server::HealthReporter,
        service_names: &[&str],
    ) {
        // Mark each service as serving
        for service_name in service_names {
            health_reporter
                .set_service_status(*service_name, tonic_health::ServingStatus::Serving)
                .await;
        }

        // Also mark empty service name for generic k8s health checks
        health_reporter
            .set_service_status("", tonic_health::ServingStatus::Serving)
            .await;

        info!(services = ?service_names, "Services marked as serving");
    }
}

// Re-export health_reporter for convenience
pub use tonic_health::server::health_reporter as create_health_service;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 50051);
    }
}
