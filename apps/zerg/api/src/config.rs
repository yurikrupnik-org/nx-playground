use axum_helpers::{JwtConfig, RateLimitConfig};
use core_config::{AppInfo, FromEnv, app_info, server::ServerConfig};

// Import database configs from the database library
use database::postgres::PostgresConfig;
use database::redis::RedisConfig;

// Re-export Environment for use in other modules
pub use core_config::Environment;

/// Application-specific configuration
/// Composes shared config components from the `config` library
#[derive(Clone, Debug)]
pub struct Config {
    pub app: AppInfo,
    pub database: PostgresConfig,
    pub redis: RedisConfig,
    pub server: ServerConfig,
    pub environment: Environment,
    // Auth configuration (using library config structs)
    pub jwt: JwtConfig,
    #[allow(dead_code)] // Will be used for CORS configuration
    pub cors_allowed_origin: String,
    pub frontend_url: String,
    pub redirect_base_url: String,
    // OAuth configuration
    pub google_client_id: String,
    pub google_client_secret: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    // NATS configuration
    pub nats_url: String,
    // Rate limiting configuration
    pub rate_limit: RateLimitConfig,
    // Vector tier rate limit (stricter limit for expensive search operations)
    pub rate_limit_vector_requests: u64,
    pub rate_limit_vector_window_secs: u64,
    // Auth tier rate limit (strict limit to prevent brute-force/credential stuffing)
    pub rate_limit_auth_requests: u64,
    pub rate_limit_auth_window_secs: u64,
}

impl Config {
    pub fn from_env() -> eyre::Result<Self> {
        let environment = Environment::from_env();
        let database = PostgresConfig::from_env()?; // Required - will fail if not set
        let server = ServerConfig::from_env()?; // Uses defaults: HOST=0.0.0.0, PORT=8080
        let redis = RedisConfig::from_env()?; // Required - will fail if not set
        let jwt = JwtConfig::from_env()?; // Required - validates min 32 chars

        // Other auth configuration
        let cors_allowed_origin =
            core_config::env_or_default("CORS_ALLOWED_ORIGIN", "http://localhost:3000");
        let frontend_url = core_config::env_or_default("FRONTEND_URL", "http://localhost:3000");
        let redirect_base_url =
            core_config::env_or_default("REDIRECT_BASE_URL", "http://localhost:8080");

        // OAuth configuration
        let google_client_id = core_config::env_required("GOOGLE_CLIENT_ID")?;
        let google_client_secret = core_config::env_required("GOOGLE_CLIENT_SECRET")?;
        let github_client_id = core_config::env_required("GITHUB_CLIENT_ID")?;
        let github_client_secret = core_config::env_required("GITHUB_CLIENT_SECRET")?;

        // NATS configuration
        let nats_url = core_config::env_or_default("NATS_URL", "nats://localhost:4222");

        // Rate limiting configuration (all optional with defaults)
        let rate_limit = RateLimitConfig::from_env();

        let rate_limit_vector_requests = std::env::var("RATE_LIMIT_VECTOR_REQUESTS_PER_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let rate_limit_vector_window_secs = std::env::var("RATE_LIMIT_VECTOR_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let rate_limit_auth_requests = std::env::var("RATE_LIMIT_AUTH_REQUESTS_PER_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let rate_limit_auth_window_secs = std::env::var("RATE_LIMIT_AUTH_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        Ok(Self {
            app: app_info!(),
            database,
            redis,
            server,
            environment,
            jwt,
            cors_allowed_origin,
            frontend_url,
            redirect_base_url,
            google_client_id,
            google_client_secret,
            github_client_id,
            github_client_secret,
            nats_url,
            rate_limit,
            rate_limit_vector_requests,
            rate_limit_vector_window_secs,
            rate_limit_auth_requests,
            rate_limit_auth_window_secs,
        })
    }
}
