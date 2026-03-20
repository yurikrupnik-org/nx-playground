//! Axum routes for Dapr subscription delivery and health checks.

use axum::{Json, Router, routing::get};
use serde::Serialize;

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// Build health check routes.
pub fn health_router() -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ready" })
}
