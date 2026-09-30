//! Health endpoints for K8s probes.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Health status of the worker.
#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub stream_connected: bool,
    pub processor_healthy: bool,
}

impl HealthStatus {
    /// Create a healthy status.
    pub fn healthy() -> Self {
        Self {
            status: "healthy".to_string(),
            stream_connected: true,
            processor_healthy: true,
        }
    }

    /// Create an unhealthy status.
    pub fn unhealthy(reason: &str) -> Self {
        Self {
            status: format!("unhealthy: {reason}"),
            stream_connected: false,
            processor_healthy: false,
        }
    }
}

/// Shared health state.
#[derive(Clone)]
pub struct HealthState {
    inner: Arc<RwLock<HealthStateInner>>,
}

struct HealthStateInner {
    stream_connected: bool,
    processor_healthy: bool,
    last_error: Option<String>,
}

impl HealthState {
    /// Create new health state.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HealthStateInner {
                stream_connected: true,
                processor_healthy: true,
                last_error: None,
            })),
        }
    }

    /// Mark stream as connected.
    pub async fn set_stream_connected(&self, connected: bool) {
        let mut inner = self.inner.write().await;
        inner.stream_connected = connected;
    }

    /// Mark processor as healthy.
    pub async fn set_processor_healthy(&self, healthy: bool) {
        let mut inner = self.inner.write().await;
        inner.processor_healthy = healthy;
    }

    /// Set last error.
    pub async fn set_error(&self, error: Option<String>) {
        let mut inner = self.inner.write().await;
        inner.last_error = error;
    }

    /// Check if alive (for liveness).
    ///
    /// Only checks processor_healthy, not stream_connected.
    /// A temporary NATS disconnection should not trigger a pod restart;
    /// only a fatal processor error should.
    pub async fn is_alive(&self) -> bool {
        let inner = self.inner.read().await;
        inner.processor_healthy
    }

    /// Check if healthy (for readiness).
    pub async fn is_healthy(&self) -> bool {
        let inner = self.inner.read().await;
        inner.stream_connected && inner.processor_healthy
    }

    /// Get status.
    pub async fn status(&self) -> HealthStatus {
        let inner = self.inner.read().await;
        if inner.stream_connected && inner.processor_healthy {
            HealthStatus::healthy()
        } else {
            let reason = inner
                .last_error
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            HealthStatus::unhealthy(&reason)
        }
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Health server for K8s probes.
pub struct HealthServer {
    port: u16,
    state: HealthState,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
}

impl HealthServer {
    /// Create a new health server.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            state: HealthState::new(),
            metrics_handle: None,
        }
    }

    /// Set the metrics handle for /metrics endpoint.
    pub fn with_metrics(mut self, handle: metrics_exporter_prometheus::PrometheusHandle) -> Self {
        self.metrics_handle = Some(handle);
        self
    }

    /// Get the health state for updates.
    pub fn state(&self) -> HealthState {
        self.state.clone()
    }

    /// Build the router.
    pub fn router(&self) -> Router {
        let state = self.state.clone();
        let metrics_handle = self.metrics_handle.clone();

        let mut router = Router::new()
            .route("/health", get(health_handler))
            .route("/healthz", get(health_handler))
            .route("/ready", get(ready_handler))
            .route("/readyz", get(ready_handler))
            .with_state(state);

        if let Some(handle) = metrics_handle {
            router = router.route(
                "/metrics",
                get(move || {
                    let handle = handle.clone();
                    async move { handle.render() }
                }),
            );
        }

        router
    }

    /// Run the health server.
    pub async fn run(self) -> Result<(), std::io::Error> {
        let router = self.router();
        let addr = format!("0.0.0.0:{}", self.port);

        info!(addr = %addr, "Starting health server");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }
}

/// Liveness probe handler.
///
/// Checks `processor_healthy` — returns 503 only on fatal processor errors,
/// not on transient NATS disconnections (K8s failureThreshold provides grace period).
async fn health_handler(State(state): State<HealthState>) -> impl IntoResponse {
    let status = state.status().await;
    if state.is_alive().await {
        (StatusCode::OK, Json(status))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(status))
    }
}

/// Readiness probe handler.
async fn ready_handler(State(state): State<HealthState>) -> impl IntoResponse {
    if state.is_healthy().await {
        (StatusCode::OK, Json(state.status().await))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(state.status().await))
    }
}
