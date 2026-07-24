//! Readiness endpoint backed by real dependency checks (Postgres + Redis), using
//! the shared `run_health_checks` helper. `/health` (liveness) comes from
//! `axum_helpers::server::health_router`.

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum_helpers::server::{HealthCheckFuture, run_health_checks};

use crate::state::AppState;

/// Router exposing `/ready` (checks DB + Redis). Mounted at the root, alongside
/// `health_router`, after `create_router`.
pub fn ready_router(state: AppState) -> Router {
    Router::new()
        .route("/ready", get(ready_handler))
        .with_state(state)
}

async fn ready_handler(State(state): State<AppState>) -> Response {
    let checks: Vec<(&str, HealthCheckFuture<'_>)> = vec![
        (
            "database",
            Box::pin(async {
                sqlx::query("SELECT 1")
                    .execute(&state.db)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("database query failed: {e}"))
            }),
        ),
        (
            "redis",
            Box::pin(async {
                let mut conn = state.redis.clone();
                redis::cmd("PING")
                    .query_async::<String>(&mut conn)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("redis ping failed: {e}"))
            }),
        ),
    ];

    match run_health_checks(checks).await {
        Ok((status, json)) => (status, json).into_response(),
        Err((status, json)) => (status, json).into_response(),
    }
}
