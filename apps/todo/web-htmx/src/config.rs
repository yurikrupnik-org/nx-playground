//! Todo htmx frontend configuration.
//!
//! Workspace `AppConfig: FromEnv` pattern: load once in `main`, thread the
//! struct through wiring, never read the environment anywhere else.

use core_config::server::ServerConfig;
use core_config::{ConfigError, Environment, FromEnv, env_or_default, env_parse_or};

/// Default listen port. Deliberately *not* `ServerConfig::DEFAULT_PORT` (8080):
/// that is todo-api's port and also this app's default upstream, so sharing it
/// makes the frontend fetch itself and 404 on `/api/todos`. Sits alongside the
/// other frontends' dev ports (todo-web 3100, todo-web-astro 3200).
const DEFAULT_PORT: u16 = 3300;

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
            server: ServerConfig {
                host: env_parse_or("HOST", ServerConfig::DEFAULT_HOST)?,
                // `TODO_WEB_HTMX_PORT`, not `PORT`: the root `.env` exports
                // `PORT=8080` (todo-api) into every `just` recipe, which would
                // reintroduce the self-fetch collision.
                port: env_parse_or("TODO_WEB_HTMX_PORT", DEFAULT_PORT)?,
            },
            todo_api_url: env_or_default("TODO_API_URL", "http://127.0.0.1:8080"),
        })
    }
}
