//! Standalone Todo REST API.
//!
//! - Owns a Postgres connection (SeaORM) and bootstraps the `todos` schema.
//! - Serves the `domain_todo` router under `/api/todos`.
//! - Publishes lifecycle events to NATS JetStream (`todos.>`) when reachable;
//!   degrades to a no-op publisher otherwise (events must not block the API).
//! - Fans the same events out to browsers over SSE (`/api/events/sse`) and
//!   WebSocket (`/api/events/ws`) via an in-process broadcast channel.

use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, Router};
use axum_helpers::{create_app, create_permissive_cors_layer};
use config::AppConfig;
use core_config::{app_info, FromEnv};
use domain_todo::{
    open_cache_bucket, CachedTodoRepository, NatsTodoPublisher, NoopTodoPublisher,
    PgTodoRepository, TodoEventPublisher, TodoService,
};
use eyre::{Result, WrapErr};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod config;
mod events;
mod stacks;

/// Embedded schema applied on startup (idempotent).
// const SCHEMA: &str = include_str!("../../../../manifests/db/todo/migrations/20240101000000_init.sql");

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());

    info!(database_url = %config.database.url(), "connecting to Postgres");
    let db = database::postgres::connect_from_config_with_retry(config.database, None)
        .await
        .wrap_err("failed to connect to Postgres")?;

    let nats_url = config.nats_url;

    // One NATS connection shared by the event publisher and the KV read-cache.
    let jetstream = match messaging::nats::jetstream(&nats_url).await {
        Ok(js) => {
            info!(%nats_url, "connected to NATS");
            Some(js)
        }
        Err(e) => {
            warn!(error = %e, %nats_url, "NATS unreachable; events + cache disabled");
            None
        }
    };

    // Event publisher (JetStream when available, else no-op).
    let publisher: Arc<dyn TodoEventPublisher> = match &jetstream {
        Some(js) => match NatsTodoPublisher::new(js.clone()).await {
            Ok(p) => {
                info!("publishing todo events to NATS JetStream (TODOS)");
                Arc::new(p)
            }
            Err(e) => {
                warn!(error = %e, "failed to init TODOS stream; events disabled");
                Arc::new(NoopTodoPublisher)
            }
        },
        None => Arc::new(NoopTodoPublisher),
    };

    // DB read-cache backed by a NATS KV bucket (best-effort; passthrough if absent).
    let cache_kv = match &jetstream {
        Some(js) => match open_cache_bucket(js, "TODO_CACHE", Duration::from_secs(30)).await {
            Ok(store) => {
                info!("DB read-cache enabled (NATS KV bucket TODO_CACHE, ttl=30s)");
                Some(store)
            }
            Err(e) => {
                warn!(error = %e, "KV cache unavailable; serving uncached");
                None
            }
        },
        None => None,
    };

    let repository = CachedTodoRepository::with_kv(PgTodoRepository::new(db.clone()), cache_kv);

    // Tee events into the in-process bus feeding the SSE/WS routes.
    let event_tx = events::channel();
    let publisher: Arc<dyn TodoEventPublisher> =
        Arc::new(events::BroadcastTodoPublisher::new(publisher, event_tx.clone()));
    let service = TodoService::new(repository, publisher);

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/todos", domain_todo::router(service))
        .nest("/api/stacks", stacks::router(db))
        .nest("/api/events", events::router(event_tx))
        .layer(create_permissive_cors_layer())
        .layer(TraceLayer::new_for_http());

    let server_config = config.server;
    info!(addr = %server_config.addr(), "todo-api listening");
    create_app(app, &server_config)
        .await
        .wrap_err("server error")?;

    Ok(())
}
