//! Standalone Todo API configuration.
//!
//! Single load point for every environment-derived setting, composed from the
//! shared `core_config` / `database` component configs. This is the workspace
//! `AppConfig: FromEnv` pattern: load once in `main`, thread the struct through
//! wiring, and never read the environment anywhere else.

use core_config::server::ServerConfig;
use core_config::{env_or_default, ConfigError, Environment, FromEnv};
use database::postgres::PostgresConfig;

/// Aggregate configuration for the todo API.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub database: PostgresConfig,
    pub server: ServerConfig,
    pub nats_url: String,
}

impl FromEnv for AppConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let environment = Environment::from_env();
        // Required: fail fast if DATABASE_URL is unset (no silent fallback DB).
        let database = PostgresConfig::from_env()?;
        let server = ServerConfig::from_env()?;
        let nats_url = env_or_default("NATS_URL", "nats://localhost:4222");

        Ok(Self {
            environment,
            database,
            server,
            nats_url,
        })
    }
}
