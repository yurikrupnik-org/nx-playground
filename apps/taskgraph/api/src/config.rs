//! Single load point for every environment-derived setting (`AppConfig: FromEnv`).

use core_config::server::ServerConfig;
use core_config::{ConfigError, Environment, FromEnv, env_or_default};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub server: ServerConfig,
    pub nats_url: String,
    /// Link template for a run's trace, `{trace_id}` substituted
    /// (e.g. `http://localhost:16686/trace/{trace_id}`); unset shows the bare id.
    pub trace_url: Option<String>,
}

impl FromEnv for AppConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            environment: Environment::from_env()?,
            server: ServerConfig::from_env()?,
            nats_url: env_or_default("NATS_URL", "nats://localhost:4222"),
            trace_url: std::env::var("TASKGRAPH_TRACE_URL")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}
