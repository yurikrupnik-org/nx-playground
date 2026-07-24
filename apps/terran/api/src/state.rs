use std::sync::Arc;

use oidc_auth::{KeycloakProvider, LoginFlowStore, OidcVerifier, RedisSessionStore};
use redis::aio::ConnectionManager;

use crate::config::Config;
use crate::db::Db;

/// Shared application state (cheap to clone; inner handles are pooled/Arc'd).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    /// Redis connection manager (readiness checks + shared by the stores below).
    pub redis: ConnectionManager,
    /// Single-use PKCE/CSRF login-flow store (BFF authorize→callback handoff).
    pub flows: LoginFlowStore,
    pub sessions: Arc<RedisSessionStore>,
    pub provider: Arc<KeycloakProvider>,
    pub verifier: Arc<OidcVerifier>,
}
