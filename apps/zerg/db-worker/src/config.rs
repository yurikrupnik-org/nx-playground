//! Worker configuration loaded from environment variables.

use database::DatabaseBackend;
use eyre::{Context, Result};
use std::str::FromStr;

/// Configuration for the DB worker.
pub struct WorkerConfig {
    /// Which database backend this worker instance targets.
    pub backend: DatabaseBackend,
    /// Port for Dapr subscription delivery (Dapr calls into the app on this port).
    pub app_port: u16,
    /// Port for health/readiness probes.
    pub health_port: u16,
    /// Dapr pub/sub component name.
    pub pubsub_name: String,
}

impl WorkerConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self> {
        let backend_str = std::env::var("DB_BACKEND")
            .wrap_err("DB_BACKEND env var is required (postgres, mongo, qdrant, influxdb, neo4j)")?;

        let backend = DatabaseBackend::from_str(&backend_str)
            .map_err(|_| eyre::eyre!("Invalid DB_BACKEND value: '{}'. Expected: postgres, mongo, qdrant, influxdb, neo4j", backend_str))?;

        let app_port = std::env::var("APP_PORT")
            .unwrap_or_else(|_| "8081".to_string())
            .parse()
            .unwrap_or(8081);

        let health_port = std::env::var("HEALTH_PORT")
            .unwrap_or_else(|_| "8082".to_string())
            .parse()
            .unwrap_or(8082);

        let pubsub_name = std::env::var("DAPR_PUBSUB_NAME")
            .unwrap_or_else(|_| "pubsub-nats".to_string());

        Ok(Self {
            backend,
            app_port,
            health_port,
            pubsub_name,
        })
    }
}
