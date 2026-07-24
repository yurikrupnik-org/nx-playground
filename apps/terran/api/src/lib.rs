//! terran API — B2B multi-tenant observability platform backend.
//!
//! Auth is Keycloak OIDC via the `oidc-auth` crate: mandatory `auth_required` on the
//! business routes, token-handler/BFF `/api/auth/*` endpoints, and JIT tenant
//! provisioning. See `docs/terran-apps-plan.md`.

pub mod assets;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod health;
pub mod openapi;
pub mod provisioning;
pub mod state;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing;
use axum_helpers::server::{create_production_app, create_router, health_router};
use core_config::app_info;
use core_config::tracing::init_tracing;
use oidc_auth::{
    AuthLayerState, KeycloakProvider, LoginFlowStore, OidcVerifier, RedisSessionStore,
    VerifierConfig,
};
use tracing::info;

use crate::config::Config;
use crate::state::AppState;

/// Build application state: DB pool, Redis, OIDC provider + verifier, session store.
pub async fn build_state(config: Config) -> eyre::Result<AppState> {
    let db = db::connect(&config.database_url).await?;
    let manager = database::redis::connect(&config.redis_url).await?;
    let sessions = Arc::new(RedisSessionStore::from_manager(manager.clone(), "terran"));
    let flows = LoginFlowStore::new(manager.clone(), "terran", auth::FLOW_TTL_SECS);
    let provider = Arc::new(
        KeycloakProvider::new(
            &config.oidc_issuer,
            &config.oidc_client_id,
            &config.oidc_client_secret,
            config.callback_url(),
        )
        .with_scopes(&config.oidc_scopes),
    );
    let mut verifier_config = VerifierConfig::keycloak(&config.oidc_issuer);
    // Lock the Bearer path to tokens issued for this client (reject other realm clients).
    verifier_config.authorized_parties = vec![config.oidc_client_id.clone()];
    verifier_config.audience = config.oidc_audience.clone();
    let verifier = Arc::new(OidcVerifier::new(verifier_config));

    Ok(AppState {
        config: Arc::new(config),
        db,
        redis: manager,
        flows,
        sessions,
        provider,
        verifier,
    })
}

/// Business + auth routes **without** the `/api` prefix (added by `create_router`).
/// Public auth-flow routes are unguarded; everything else requires `auth_required`.
pub fn api_routes(state: AppState) -> Router {
    let auth_layer = AuthLayerState::new(
        state.verifier.clone(),
        state.sessions.clone(),
        state.provider.clone(),
        state.config.cookie_name.clone(),
    );

    let public = Router::new()
        .route("/auth/login", routing::get(auth::login))
        .route("/auth/callback", routing::get(auth::callback))
        .route("/auth/logout", routing::post(auth::logout));

    let protected = Router::new()
        .route("/auth/me", routing::get(auth::me))
        .route(
            "/assets",
            routing::get(assets::list_assets).post(assets::create_asset),
        )
        .route("/assets/{id}", routing::get(assets::get_asset))
        .route(
            "/assets/by-user/{user_id}",
            routing::get(assets::list_assets_by_user),
        )
        .route_layer(from_fn_with_state(auth_layer, oidc_auth::auth_required));

    // CSRF: double-submit guard on cookie-authed state-changing requests (logout,
    // asset writes). Safe methods and Bearer/machine requests pass through.
    let csrf = axum_helpers::CsrfConfig::new(auth::CSRF_COOKIE);
    let guarded = public
        .merge(protected)
        .route_layer(from_fn_with_state(csrf, axum_helpers::csrf_protect));

    // Native email+password login (Keycloak Direct Access Grant). Public and CSRF-exempt:
    // it runs before any session/CSRF cookie exists, and its JSON content-type blocks
    // cross-site forgery (a cross-origin JSON POST needs a CORS-denied preflight).
    let password_login =
        Router::new().route("/auth/login/password", routing::post(auth::password_login));

    guarded.merge(password_login).with_state(state)
}

/// Assemble the full app: API routes under `/api` with shared middleware + OpenAPI
/// docs (`create_router`), plus root `/health` (liveness) and `/ready` (deps).
pub async fn build_app(state: AppState) -> eyre::Result<Router> {
    let routes = api_routes(state.clone());
    let app = create_router::<openapi::ApiDoc>(routes)
        .await
        .map_err(|e| eyre::eyre!("failed to build router: {e}"))?;
    Ok(app
        .merge(health_router(app_info!()))
        .merge(health::ready_router(state))
        // RED metrics for every route; no-op until a recorder is installed (see `run`).
        .layer(from_fn(axum_helpers::track_metrics)))
}

/// Bootstrap and serve the API with OTEL tracing and graceful shutdown + cleanup.
pub async fn run() -> eyre::Result<()> {
    let config = Config::from_env()?;
    // Guard flushes OTEL spans on drop; hold it for the server's lifetime.
    let _tracing_guard = init_tracing(&config.environment, app_info!());

    // Install the global Prometheus recorder before any metric is emitted.
    let metrics_handle = axum_helpers::init_metrics()?;

    let state = build_state(config).await?;
    // `/metrics` merged after `build_app`'s `track_metrics` layer, so it is not tracked.
    let app = build_app(state.clone())
        .await?
        .merge(axum_helpers::metrics_router(metrics_handle));
    let server = state.config.server.clone();

    // Sample the Postgres pool into gauges every 15s (runs for the process lifetime).
    axum_helpers::spawn_pool_metrics(state.db.clone(), Duration::from_secs(15));

    info!(
        "terran_api starting on {} (graceful shutdown: 30s)",
        server.addr()
    );
    create_production_app(app, &server, Duration::from_secs(30), async move {
        info!("shutting down: closing Postgres + Redis");
        state.db.close().await;
        drop(state.redis);
    })
    .await
    .map_err(|e| eyre::eyre!("server error: {e}"))?;

    info!("terran_api shutdown complete");
    Ok(())
}
