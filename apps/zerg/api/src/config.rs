use axum_helpers::RateLimitConfig;
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
    // WorkOS AuthKit (BFF auth via the oidc-auth crate)
    /// WorkOS environment client id (`client_...`).
    pub workos_client_id: String,
    /// WorkOS API key (`sk_...`) — sent as `client_secret` on authenticate calls.
    pub workos_api_key: String,
    /// Token issuer; `https://api.workos.com` unless a custom auth domain is set.
    pub oidc_issuer: String,
    /// Browser session cookie name.
    pub cookie_name: String,
    /// Set the `Secure` attribute on auth cookies (false for local http dev).
    pub cookie_secure: bool,
    /// Session lifetime in seconds.
    pub session_ttl_secs: u64,
    pub frontend_url: String,
    pub redirect_base_url: String,
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
        let environment = Environment::from_env()?;
        let database = PostgresConfig::from_env()?; // Required - will fail if not set
        let server = ServerConfig::from_env()?; // Uses defaults: HOST=0.0.0.0, PORT=8080
        let redis = RedisConfig::from_env()?; // Required - will fail if not set

        let frontend_url = core_config::env_or_default("FRONTEND_URL", "http://localhost:3000");
        let redirect_base_url =
            core_config::env_or_default("REDIRECT_BASE_URL", "http://localhost:8080");

        // WorkOS AuthKit configuration
        let workos_client_id = core_config::env_required("WORKOS_CLIENT_ID")?;
        let workos_api_key = core_config::env_required("WORKOS_API_KEY")?;
        // App-prefixed keys: terran shares the same shell env and reads the generic
        // OIDC_ISSUER / SESSION_COOKIE_* names for its Keycloak config; on localhost
        // cookies ignore ports, so the two apps MUST NOT share these values.
        // Token issuer, per WorkOS OIDC discovery
        // (`{api}/user_management/{client_id}/.well-known/openid-configuration`):
        // `https://api.workos.com/user_management/{client_id}`. Override with
        // WORKOS_ISSUER only for a custom auth domain.
        let oidc_issuer = std::env::var("WORKOS_ISSUER")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!("https://api.workos.com/user_management/{workos_client_id}")
            });
        let cookie_name = core_config::env_or_default("ZERG_SESSION_COOKIE_NAME", "zerg_session");
        let cookie_secure =
            core_config::env_or_default("ZERG_SESSION_COOKIE_SECURE", "false") == "true";
        let session_ttl_secs: u64 =
            core_config::env_or_default("ZERG_SESSION_TTL_SECS", "28800").parse()?;

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
            workos_client_id,
            workos_api_key,
            oidc_issuer,
            cookie_name,
            cookie_secure,
            session_ttl_secs,
            frontend_url,
            redirect_base_url,
            nats_url,
            rate_limit,
            rate_limit_vector_requests,
            rate_limit_vector_window_secs,
            rate_limit_auth_requests,
            rate_limit_auth_window_secs,
        })
    }
}

impl Config {
    /// OAuth redirect URI registered with WorkOS.
    pub fn callback_url(&self) -> String {
        format!("{}/api/auth/callback", self.redirect_base_url)
    }
}
