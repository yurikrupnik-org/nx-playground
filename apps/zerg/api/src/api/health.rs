//! Application-specific health check handlers with real database/redis checks.

use crate::state::AppState;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use axum_helpers::server::{HealthCheckFuture, run_health_checks};
use std::time::Duration;
use tonic_health::pb::HealthCheckRequest;

/// Readiness check endpoint: verifies the dependencies this process *owns*.
///
/// Deliberately excludes the tasks gRPC service. Readiness controls load-balancer
/// membership, so gating it on a downstream meant tasks being down also stopped
/// projects, users, and org endpoints from serving - strictly worse availability than
/// not having split at all. Tasks routes degrade on their own (503 from the upstream
/// error mapping) while everything else keeps serving.
///
/// Downstream reachability is reported by `/health` as information, not as a gate.
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
    ];

    match run_health_checks(checks).await {
        Ok((status, json)) => (status, json).into_response(),
        Err((status, json)) => (status, json).into_response(),
    }
}

/// Informational upstream status. **Always 200** — this reports reachability, it does
/// not gate anything. Kept separate from `/ready` on purpose: see the note there.
pub async fn upstreams_handler(State(state): State<AppState>) -> Response {
    let mut health = state.tasks_health.clone();
    let mut req = tonic::Request::new(HealthCheckRequest {
        // Empty service name = overall server health
        service: String::new(),
    });
    req.set_timeout(Duration::from_secs(2));

    let tasks_grpc = match health.check(req).await {
        Ok(_) => "reachable",
        Err(e) => {
            tracing::debug!(error = %e, "tasks gRPC unreachable");
            "unreachable"
        }
    };

    axum::Json(serde_json::json!({ "tasks_grpc": tasks_grpc })).into_response()
}
