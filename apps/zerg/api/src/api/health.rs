//! Application-specific health check handlers with real database/redis checks.

use crate::state::AppState;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use axum_helpers::server::{HealthCheckFuture, run_health_checks};
use std::time::Duration;
use tonic_health::pb::HealthCheckRequest;

/// Readiness check endpoint that actually checks database and redis connections.
///
/// This uses the generic `run_health_checks` utility from axum-helpers
/// to verify all service dependencies are healthy.
pub async fn ready_handler(State(state): State<AppState>) -> Response {
    let checks: Vec<(&str, HealthCheckFuture<'_>)> = vec![
        (
            "database",
            Box::pin(async {
                state
                    .db
                    .ping()
                    .await
                    .map_err(|e| format!("Database ping failed: {}", e))
            }),
        ),
        (
            "redis",
            Box::pin(async {
                let mut redis = state.redis.clone();
                redis::cmd("PING")
                    .query_async::<String>(&mut redis)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("Redis ping failed: {}", e))
            }),
        ),
        (
            "tasks_grpc",
            Box::pin(async {
                let mut health = state.tasks_health.clone();
                let mut req = tonic::Request::new(HealthCheckRequest {
                    // Empty service name = overall server health
                    service: String::new(),
                });
                req.set_timeout(Duration::from_secs(2));
                health
                    .check(req)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("Tasks gRPC health check failed: {}", e))
            }),
        ),
    ];

    match run_health_checks(checks).await {
        Ok((status, json)) => (status, json).into_response(),
        Err((status, json)) => (status, json).into_response(),
    }
}
