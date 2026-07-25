use core_config::{Environment, FromEnv, env_or_default, env_required, server::ServerConfig};

/// terran API configuration, read from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    /// HTTP server bind config (HOST/PORT); shared `core_config` type.
    pub server: ServerConfig,
    /// Deployment environment (drives tracing defaults, etc.).
    pub environment: Environment,
    pub database_url: String,
    pub redis_url: String,
    /// OIDC issuer, e.g. `http://localhost:8088/realms/terran`.
    pub oidc_issuer: String,
    pub oidc_client_id: String,
    pub oidc_client_secret: String,
    /// OAuth scopes to request (include `organization` for KC Organizations claims).
    pub oidc_scopes: String,
    /// Expected access-token audience. Leave unset unless a Keycloak audience
    /// mapper adds the API to `aud`; `azp` is validated regardless (see verifier).
    pub oidc_audience: Option<String>,
    /// Public base URL of this API (for the OAuth redirect URI).
    pub redirect_base_url: String,
    /// SPA base URL to return to after login/logout.
    pub frontend_url: String,
    /// Browser session cookie name.
    pub cookie_name: String,
    /// Set the `Secure` attribute on auth cookies (false for local http dev).
    pub cookie_secure: bool,
    /// Session lifetime in seconds.
    pub session_ttl_secs: u64,
}

impl Config {
    pub fn from_env() -> eyre::Result<Self> {
        Ok(Self {
            server: ServerConfig::from_env()?,
            environment: Environment::from_env()?,
            database_url: env_required("DATABASE_URL")?,
            redis_url: env_or_default("REDIS_URL", "redis://127.0.0.1:6379"),
            oidc_issuer: env_or_default("OIDC_ISSUER", "http://localhost:8088/realms/terran"),
            oidc_client_id: env_or_default("OIDC_CLIENT_ID", "terran-api"),
            oidc_client_secret: env_or_default("OIDC_CLIENT_SECRET", "local-dev-secret"),
            oidc_scopes: env_or_default("OIDC_SCOPES", "openid profile email organization"),
            oidc_audience: std::env::var("OIDC_AUDIENCE")
                .ok()
                .filter(|s| !s.is_empty()),
            redirect_base_url: env_or_default("REDIRECT_BASE_URL", "http://localhost:8081"),
            frontend_url: env_or_default("FRONTEND_URL", "http://localhost:3001"),
            cookie_name: env_or_default("SESSION_COOKIE_NAME", "terran_session"),
            cookie_secure: env_or_default("SESSION_COOKIE_SECURE", "false") == "true",
            session_ttl_secs: env_or_default("SESSION_TTL_SECS", "28800").parse()?,
        })
    }

    /// OAuth redirect URI registered with the IdP.
    pub fn callback_url(&self) -> String {
        format!("{}/api/auth/callback", self.redirect_base_url)
    }
}
