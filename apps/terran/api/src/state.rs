use std::sync::Arc;

use domain_cloud_resources::observed::K8sInventory;
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
    /// Read-only cloud inventory observed from the cluster (Crossplane
    /// `CloudInventory`). `None` when the API has no cluster access — the
    /// inventory endpoints then answer 503 and the rest of the API is unaffected.
    pub inventory: Option<Arc<K8sInventory>>,
}
