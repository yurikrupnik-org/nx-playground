//! `taskgraph_api` — the read side of taskgraph.
//!
//! Folds the `TASKGRAPH` JetStream stream (published by the `taskgraph` CLI)
//! into an in-memory projection and serves it:
//!
//! - `GET /` — drill-down UI (single embedded page, no build step)
//! - `GET /api/graphs`, `/api/graphs/{id}`, `/api/graphs/{id}/tasks/{task}`
//! - `GET /api/runs`, `/api/runs/{id}`
//! - `GET /api/events/sse` — live facts
//! - `GET /healthz` (liveness), `/readyz` (replay caught up), `/metrics`
//!
//! There is no database: the stream is the source of truth and the model is
//! rebuilt from it on every start (bounded by the stream's retention).

use std::sync::Arc;

use axum::middleware;
use axum_helpers::{create_app, init_metrics, metrics_router, track_metrics};
use config::AppConfig;
use core_config::{FromEnv, app_info};
use eyre::{Result, WrapErr};
use tower_http::trace::TraceLayer;
use tracing::info;

mod api;
mod config;
mod projector;

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());
    let metrics = init_metrics().wrap_err("installing Prometheus recorder")?;

    // The projection IS this service: without NATS there is nothing to serve,
    // so keep retrying (bounded) and let the startup probe decide.
    let jetstream = messaging::nats::jetstream_with_retry(&config.nats_url, None)
        .await
        .wrap_err_with(|| format!("connecting to NATS at {}", config.nats_url))?;
    info!(nats_url = %config.nats_url, "connected to NATS");

    let shared = Arc::new(projector::Shared::new());
    tokio::spawn(projector::run(jetstream, shared.clone()));

    let app = api::router(api::AppState {
        shared,
        trace_url: config.trace_url,
    })
    .layer(middleware::from_fn(track_metrics))
    .layer(TraceLayer::new_for_http())
    .merge(metrics_router(metrics));

    info!(addr = %config.server.addr(), "taskgraph-api listening");
    create_app(app, &config.server)
        .await
        .wrap_err("server error")?;
    Ok(())
}
