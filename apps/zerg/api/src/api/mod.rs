use axum::{Extension, Router, middleware};
use axum_helpers::RateLimitTier;

pub mod auth;
pub mod cloud_resources;
pub mod health;
pub mod projects;
pub mod tasks;
pub mod tasks_async;
pub mod tasks_direct;
pub mod tasks_mongo;
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

    let router = Router::new()
        .nest(
            "/auth",
            auth::router(state)
                .layer(rl_layer())
                .layer(Extension(auth_tier)),
        )
        .nest(
            "/tasks",
            tasks::router(state.clone())
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
        .nest(
            "/tasks-direct",
            tasks_direct::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
        .nest(
            "/tasks-mongo",
            tasks_mongo::router(state.mongo_db.clone())
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
        .nest(
            domain_projects::entity::Model::URL,
            projects::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
        .nest(
            domain_cloud_resources::entity::Model::URL,
            cloud_resources::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
        .nest(
            "/users",
            users::router(state)
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        );

    // Add async task routes if Dapr is configured
    let router = if let Some(async_router) = tasks_async::router(state) {
        router.nest(
            "/tasks-async",
            async_router
                .layer(rl_layer())
                .layer(Extension(standard.clone())),
        )
    } else {
        router
    };

    // Add vector routes with stricter tier if Qdrant is configured
    let router = if let Some(vector_router) = vector::router(state) {
        router.nest(
            "/vector",
            vector_router
                .layer(rl_layer())
                .layer(Extension(vector_tier)),
        )
    } else {
        router
    };

    // optional auth is global (outermost) — inserts JwtClaims if token present,
    // so per-route rate_limit_middleware can key by user:<id> or fall back to ip:<addr>.
    router.layer(middleware::from_fn_with_state(
        state.jwt_auth.clone(),
        axum_helpers::optional_jwt_auth_middleware,
    ))
}

/// Creates a router with the /ready endpoint that performs actual health checks.
///
/// This router has state applied and can be merged with the stateless app router
/// from `create_router`. The /ready endpoint checks database and redis connections.
pub fn ready_router(state: crate::state::AppState) -> Router {
    use axum::routing::get;

    Router::new()
        .route("/ready", get(health::ready_handler))
        .with_state(state)
}
