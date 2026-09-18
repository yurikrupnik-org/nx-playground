use axum::routing::{get, post};
use axum::{Extension, Router, middleware};
use axum_helpers::RateLimitTier;

pub mod auth;
pub mod cloud_resources;
pub mod health;
pub mod org;
pub mod projects;
pub mod tasks;
pub mod users;
pub mod vector;

/// Creates the API routes without the `/api` prefix.
/// The `/api` prefix will be added by the `create_router` helper.
///
/// This function takes a reference to AppState and initializes all services.
/// Returns a stateless Router (all sub-routers have state already applied).
/// Only Arc pointer clones remain when domains extract db connections (cheap).
///
/// Uses generated constants from SeaOrmResource proc macro to avoid hardcoded paths.
pub fn routes(state: &crate::state::AppState) -> Router {
    // Import ApiResource trait to access URL constants
    use domain_projects::ApiResource;

    let rl = &state.config.rate_limit;
    let standard = RateLimitTier::new("standard", rl.requests_per_window, rl.window_secs);
    let vector_tier = RateLimitTier::new(
        "vector",
        state.config.rate_limit_vector_requests,
        state.config.rate_limit_vector_window_secs,
    );
    let auth_tier = RateLimitTier::new(
        "auth",
        state.config.rate_limit_auth_requests,
        state.config.rate_limit_auth_window_secs,
    );

    // Closure to build per-route rate limit layers.
    // Axum onion: last .layer() is outermost (runs first).
    // Extension(tier) runs first → sets RateLimitTier,
    // then rate_limit_middleware reads it and checks Redis.
    let rl_layer = || {
        middleware::from_fn_with_state(
            state.rate_limiter.clone(),
            axum_helpers::rate_limit_middleware,
        )
    };

    // Mandatory auth on every business router: unauthenticated/forged requests are
    // rejected before the handler and an `AuthIdentity` lands in request extensions
    // (the rate limiter keys on it). Public auth-flow routes (/auth) are excluded.
    let auth_layer = oidc_auth::AuthLayerState::new(
        state.verifier.clone(),
        state.sessions.clone(),
        state.provider.clone(),
        state.config.cookie_name.clone(),
    );
    let auth_mw = || middleware::from_fn_with_state(auth_layer.clone(), oidc_auth::auth_required);
    // Tenant context: resolves org/user scope from the verified identity; MUST sit
    // inside auth_mw (innermost) so AuthIdentity is already in extensions.
    let tenant_mw =
        || middleware::from_fn_with_state(state.clone(), crate::orgs::tenant_context_mw);
    // CSRF double-submit on cookie-authed mutations; safe methods and Bearer are exempt.
    let csrf_cfg = axum_helpers::CsrfConfig::new("csrf_token");
    let csrf_mw = || middleware::from_fn_with_state(csrf_cfg.clone(), axum_helpers::csrf_protect);

    // BFF auth routes (terran pattern): flow routes are public; /me requires auth;
    // logout is a cookie-authed mutation so it sits behind CSRF; the native
    // password login is CSRF-exempt (no cookie exists yet; JSON body blocks
    // cross-site form posts).
    let auth_routes = {
        let public = Router::new()
            .route("/login", get(auth::login))
            .route("/callback", get(auth::callback))
            .route("/logout", post(auth::logout));
        let protected = Router::new()
            .route("/me", get(auth::me))
            .route_layer(auth_mw());
        public
            .merge(protected)
            .route_layer(csrf_mw())
            .route("/login/password", post(auth::password_login))
            .with_state(state.clone())
    };

    let router = Router::new()
        .nest(
            "/auth",
            auth_routes.layer(rl_layer()).layer(Extension(auth_tier)),
        )
        .nest(
            "/tasks",
            tasks::router(state.clone())
                .layer(tenant_mw())
                .layer(rl_layer())
                .layer(Extension(standard.clone()))
                .layer(auth_mw())
                .layer(csrf_mw()),
        )
        .nest(
            "/org",
            org::router(state)
                .layer(tenant_mw())
                .layer(rl_layer())
                .layer(Extension(standard.clone()))
                .layer(auth_mw())
                .layer(csrf_mw()),
        )
        .nest(
            domain_projects::entity::Model::URL,
            projects::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone()))
                .layer(auth_mw())
                .layer(csrf_mw()),
        )
        .nest(
            domain_cloud_resources::entity::Model::URL,
            cloud_resources::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone()))
                .layer(auth_mw())
                .layer(csrf_mw()),
        )
        .nest(
            "/users",
            users::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone()))
                .layer(auth_mw())
                .layer(csrf_mw()),
        );

    // Add vector routes with stricter tier if Qdrant is configured
    if let Some(vector_router) = vector::router(state) {
        router.nest(
            "/vector",
            vector_router
                .layer(tenant_mw())
                .layer(rl_layer())
                .layer(Extension(vector_tier))
                .layer(auth_mw())
                .layer(csrf_mw()),
        )
    } else {
        router
    }
}

/// Creates a router with the /ready and /upstreams endpoints.
///
/// This router has state applied and can be merged with the stateless app router
/// from `create_router`. `/ready` checks only what this process owns (database,
/// redis); `/upstreams` reports downstream reachability without gating on it.
pub fn ready_router(state: crate::state::AppState) -> Router {
    use axum::routing::get;

    Router::new()
        .route("/ready", get(health::ready_handler))
        .route("/upstreams", get(health::upstreams_handler))
        .with_state(state)
}
