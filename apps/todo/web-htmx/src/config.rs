//! Todo htmx frontend configuration.
//!
//! Workspace `AppConfig: FromEnv` pattern: load once in `main`, thread the
//! struct through wiring, never read the environment anywhere else.

use core_config::server::ServerConfig;
use core_config::{env_or_default, ConfigError, Environment, FromEnv};

/// Aggregate configuration for the htmx frontend.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub server: ServerConfig,
    /// Upstream todo-api origin (same default as the Astro variant's api.ts).
    pub todo_api_url: String,
}

impl FromEnv for AppConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            environment: Environment::from_env()?,
            server: ServerConfig::from_env()?,
            todo_api_url: env_or_default("TODO_API_URL", "http://127.0.0.1:8080"),
        })
    }
}
