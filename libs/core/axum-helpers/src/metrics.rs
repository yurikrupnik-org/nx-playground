//! Prometheus metrics for HTTP services.
//!
//! Provides the three pieces an Axum app needs to expose RED-style metrics:
//! - [`init_metrics`]: install the global Prometheus recorder (once per process).
//! - [`track_metrics`]: middleware recording request rate / errors / duration.
//! - [`metrics_router`]: a `GET /metrics` route rendering the Prometheus text format.
//! - [`spawn_pool_metrics`]: background sampler for Postgres connection-pool gauges.
//!
//! The `metrics` facade is used for emission, so the recorder is decoupled from the
//! call sites; without a recorder installed every macro is a cheap no-op (which is why
//! [`track_metrics`] is safe to layer in tests).

use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
// Leading `::` forces the external crates, since this module is itself named `metrics`.
use ::metrics::{counter, gauge, histogram};
use ::metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;

/// Latency histogram buckets in seconds (5ms … 10s), covering typical API responses.
const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Install the global Prometheus recorder and return its render handle.
///
/// Call exactly once per process, before emitting any metric. Returns an error if a
/// recorder is already installed (a programming error, not a runtime condition).
pub fn init_metrics() -> eyre::Result<PrometheusHandle> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_request_duration_seconds".to_owned()),
            LATENCY_BUCKETS,
        )?
        .install_recorder()?;
    Ok(handle)
}

/// Router exposing `GET /metrics` in the Prometheus text exposition format.
///
/// Merge this *after* applying [`track_metrics`] so the scrape endpoint itself is not
/// counted as application traffic.
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let handle = handle.clone();
            async move { handle.render() }
        }),
    )
}

/// Middleware recording RED metrics for every request:
/// - `http_requests_total{method,path,status}` — counter
/// - `http_request_duration_seconds{method,path,status}` — histogram
/// - `http_requests_in_flight` — gauge
///
/// `path` is the matched route template (e.g. `/api/assets/{id}`), not the raw URI, to
/// keep label cardinality bounded. Unmatched requests (404s) are labelled `unknown`.
pub async fn track_metrics(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_owned();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    gauge!("http_requests_in_flight").increment(1.0);
    let response = next.run(req).await;
    gauge!("http_requests_in_flight").decrement(1.0);

    let status = response.status().as_u16().to_string();
    let latency = start.elapsed().as_secs_f64();

    counter!(
        "http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.clone(),
    )
    .increment(1);
    histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "path" => path,
        "status" => status,
    )
    .record(latency);

    response
}

/// Spawn a background task that samples a Postgres pool into
/// `db_pool_connections{state="total|idle|active"}` gauges every `interval`.
///
/// The returned handle may be dropped; the task runs for the process lifetime. The
/// `pool` is a cheap `Arc` clone, so closing the application's pool on shutdown does not
/// affect sampling (it simply reports zero).
pub fn spawn_pool_metrics(pool: PgPool, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let total = pool.size() as f64;
            let idle = pool.num_idle() as f64;
            gauge!("db_pool_connections", "state" => "total").set(total);
            gauge!("db_pool_connections", "state" => "idle").set(idle);
            gauge!("db_pool_connections", "state" => "active").set((total - idle).max(0.0));
        }
    })
}
