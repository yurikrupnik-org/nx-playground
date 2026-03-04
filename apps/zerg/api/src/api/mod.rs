use axum::{Extension, Router, middleware};
use axum_helpers::RateLimitTier;

pub mod auth;
pub mod cloud_resources;
pub mod health;
pub mod projects;
pub mod tasks;
pub mod tasks_direct;
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

    let router = Router::new()
        .nest("/auth", auth::router(state)) // No tier = exempt from rate limiting
        .nest(
            "/tasks",
            tasks::router(state.clone())
                .layer(Extension(standard.clone())),
        )
        .nest(
            "/tasks-direct",
            tasks_direct::router(state)
                .layer(Extension(standard.clone())),
        )
        .nest(
            domain_projects::entity::Model::URL,
            projects::router(state)
                .layer(Extension(standard.clone())),
        )
        .nest(
            domain_cloud_resources::entity::Model::URL,
            cloud_resources::router(state)
                .layer(Extension(standard.clone())),
        )
        .nest(
            "/users",
            users::router(state)
                .layer(Extension(standard.clone())),
        );

    // Add vector routes with stricter tier if Qdrant is configured
    let router = if let Some(vector_router) = vector::router(state) {
        router.nest("/vector", vector_router.layer(Extension(vector_tier)))
    } else {
        router
    };

    // Axum onion model: last .layer() is outermost (runs first).
    // optional auth runs first (inserts JwtClaims if token present),
    // then rate limiter keys by user:<id> or falls back to ip:<addr>.
    router
        .layer(middleware::from_fn_with_state(
            state.rate_limiter.clone(),
            axum_helpers::rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
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
