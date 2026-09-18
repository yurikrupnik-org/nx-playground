//! Standalone Todo API — one backend, four transports on one port.
//!
//! - Owns a Postgres connection (SeaORM) over the `todos` schema (migrations are
//!   applied out of band: `just migrate todo`, or the Atlas operator in-cluster).
//! - Serves the `domain_todo` REST router under `/api/todos`.
//! - Serves the same service as gRPC (`todo.v1.TodoService`, plus
//!   `grpc.health.v1.Health`) on the SAME listener: tonic routes are merged
//!   into the axum router and `axum::serve` speaks h2c. See `grpc.rs`.
//! - Publishes lifecycle events to NATS JetStream (`todos.>`) when reachable;
//!   degrades to a no-op publisher otherwise (events must not block the API).
//! - Streams **database-sourced** changes to clients over SSE
//!   (`/api/events/sse`), WebSocket (`/api/events/ws`) and the gRPC `Watch`
//!   server stream: a Postgres trigger NOTIFYs on every committed write and
//!   `domain_todo::db_events` fans it out, so UIs stay correct regardless of
//!   which process made the change.

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use axum_helpers::{create_app, create_permissive_cors_layer};
use config::AppConfig;
use core_config::{FromEnv, app_info};
use domain_todo::{
    CachedTodoRepository, NatsTodoPublisher, NoopTodoPublisher, PgTodoRepository,
    TodoEventPublisher, TodoService, open_cache_bucket,
};
use eyre::{Result, WrapErr};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

mod config;
mod events;
mod grpc;
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

    // Two decorators over the same pool and KV store: one for the request path,
    // one for the change listener. They hold only shared handles, so this is a
    // cheap way to give the listener its own reference without an API change.
    let listener_repository =
        CachedTodoRepository::with_kv(PgTodoRepository::new(db.clone()), cache_kv.clone());
    let repository = CachedTodoRepository::with_kv(PgTodoRepository::new(db.clone()), cache_kv);

    // Realtime bus. The producer is the DATABASE: `db_events::listen` turns the
    // `todos_notify` trigger's NOTIFY into TodoEvents, so writes from any process
    // (or a plain psql session) reach every connected browser. The service keeps
    // publishing to NATS for todo-worker; it no longer feeds the browser bus.
    let event_tx = events::channel();
    let listener_pool = db.get_postgres_connection_pool().clone();
    let listener_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Err(e) =
            domain_todo::db_events::listen(&listener_pool, &listener_repository, listener_tx).await
        {
            error!(error = %e, "todo database change listener stopped; realtime UI is stale");
        }
    });

    let service = TodoService::new(repository, publisher);
    let grpc = grpc::router(service.clone(), event_tx.clone()).await;

    // CORS + tracing wrap the HTTP routes only. The gRPC routes are merged
    // afterwards: a CORS preflight has no meaning for h2c gRPC, and tonic's
    // own status codes must not be reshaped by an HTTP-flavoured layer.
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/todos", domain_todo::router(service))
        .nest("/api/stacks", stacks::router(db))
        .nest("/api/events", events::router(event_tx))
        .layer(create_permissive_cors_layer())
        .layer(TraceLayer::new_for_http())
        .merge(grpc);

    let server_config = config.server;
    info!(addr = %server_config.addr(), "todo-api listening");
    create_app(app, &server_config)
        .await
        .wrap_err("server error")?;

    Ok(())
}
