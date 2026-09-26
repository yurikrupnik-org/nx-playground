//! Todo worker configuration.
//!
//! Single load point for every environment-derived setting, following the
//! workspace `AppConfig: FromEnv` pattern: load once in `run`, thread the struct
//! through, and never read the environment anywhere else.

use core_config::{ConfigError, Environment, FromEnv, env_or_default};

/// Default health-probe port (overridable via `TODO_WORKER_HEALTH_PORT`, then `HEALTH_PORT`).
const DEFAULT_HEALTH_PORT: u16 = 8091;

/// Aggregate configuration for the todo worker.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub nats_url: String,
    pub health_port: u16,
}

impl FromEnv for AppConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let environment = Environment::from_env()?;
        let nats_url = env_or_default("NATS_URL", "nats://localhost:4222");
        let health_port = std::env::var("TODO_WORKER_HEALTH_PORT")
            .or_else(|_| std::env::var("HEALTH_PORT"))
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_HEALTH_PORT);
        Ok(Self {
            environment,
            nats_url,
            health_port,
        })
    }
}
