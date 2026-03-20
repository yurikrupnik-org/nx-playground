//! Multi-Database Worker Service (Dapr + NATS JetStream)
//!
//! An event-driven worker that processes database operations delivered
//! via Dapr pub/sub (backed by NATS JetStream). Each deployment targets
//! ONE database backend for independent scaling and failure isolation.
//!
//! ## Architecture
//!
//! ```text
//! Dapr Sidecar (pub/sub subscription)
//!   ↓ HTTP POST to app (CloudEvent envelope)
//! Axum Router (subscription routes)
//!   ↓ (extracts DbOpEvent → CreateTask/UpdateTask/TaskFilter)
//! Handler (per backend)
//!   ├─ PgTaskHandler    → TaskService<PgTaskRepository>    → PostgreSQL
//!   ├─ MongoTaskHandler → TaskService<MongoTaskRepository> → MongoDB
//!   ├─ DaprStateHandler → Dapr state store API (fallback for non-task entities)
//!   ├─ QdrantHandler    → Qdrant (direct client)
//!   ├─ InfluxDbHandler  → InfluxDB (direct client)
//!   └─ Neo4jHandler     → Neo4j (direct client)
//! ```
//!
//! ## Configuration
//!
//! - `DB_BACKEND`: Target backend (postgres, mongo, qdrant, influxdb, neo4j)
//! - `APP_PORT`: Port for Dapr subscription delivery (default: 8081)
//! - `HEALTH_PORT`: Port for K8s health probes (default: 8082)
//! - `DAPR_PUBSUB_NAME`: Dapr pub/sub component name (default: pubsub-nats)
//! - `DATABASE_URL`: PostgreSQL connection string (when DB_BACKEND=postgres)
//! - `MONGODB_URL`: MongoDB connection string (when DB_BACKEND=mongo)
//! - `MONGODB_DATABASE`: MongoDB database name (when DB_BACKEND=mongo)
//!
//! ## Feature Flags
//!
//! - `postgres`: Enable PostgreSQL backend with direct TaskService
//! - `mongo`: Enable MongoDB backend with direct TaskService
//! - `qdrant`: Enable Qdrant vector backend
//! - `influxdb`: Enable InfluxDB time-series backend
//! - `neo4j`: Enable Neo4j graph backend

pub mod config;
pub mod handlers;
pub mod subscription;

use axum::Router;
use axum::routing::post;
use config::WorkerConfig;
use dapr_client::subscription::SubscriptionRoute;
use database::DatabaseBackend;
use eyre::{Result, WrapErr};
use tokio::signal;
use tracing::info;

/// Run the database worker service.
pub async fn run() -> Result<()> {
    // Initialize tracing
    let environment = core_config::Environment::from_env();
    core_config::tracing::init_tracing(&environment);

    let app_info = core_config::app_info!();
    info!(
        name = %app_info.name,
        version = %app_info.version,
        "Starting DB worker service"
    );

    // Load configuration
    let config = WorkerConfig::from_env()?;
    info!(
        backend = %config.backend,
        app_port = config.app_port,
        health_port = config.health_port,
        "Worker configuration loaded"
    );

    // Build the Dapr subscription discovery + event handler routes
    let app_router = build_router(&config).await?;

    // Merge health routes
    let router = app_router.merge(subscription::health_router());

    // Start the HTTP server for Dapr subscription delivery
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.app_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .wrap_err_with(|| format!("Failed to bind to {}", addr))?;

    info!(addr = %addr, "Listening for Dapr subscription deliveries");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .wrap_err("Server error")?;

    info!("DB worker service stopped");
    Ok(())
}

/// Build the Axum router based on the configured database backend.
async fn build_router(config: &WorkerConfig) -> Result<Router> {
    // Build Dapr subscription discovery route
    let topic = config.backend.topic();
    let route = format!("/events/{}", topic.replace('.', "-"));

    let discovery = SubscriptionRoute::new()
        .subscribe(&config.pubsub_name, topic, &route)
        .build_discovery_router();

    // Build handler routes based on backend type
    let handler_router = match &config.backend {
        // ── PostgreSQL: direct TaskService via PgTaskRepository ───
        #[cfg(feature = "postgres")]
        DatabaseBackend::Postgres => {
            let db_url = std::env::var("DATABASE_URL")
                .wrap_err("DATABASE_URL required for postgres backend")?;
            info!(url = %db_url, "Connecting to PostgreSQL...");
            let db = database::postgres::connect(&db_url)
                .await
                .map_err(|e| eyre::eyre!("PostgreSQL connection failed: {}", e))?;
            info!("Connected to PostgreSQL");

            let handler = handlers::tasks_pg::PgTaskHandler::new(db);
            Router::new()
                .route(&route, post(handlers::tasks_pg::handle_event))
                .with_state(handler)
        }

        // ── MongoDB: direct TaskService via MongoTaskRepository ──
        #[cfg(feature = "mongo")]
        DatabaseBackend::Mongo => {
            let mongo_url = std::env::var("MONGODB_URL")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
            let mongo_db_name = std::env::var("MONGODB_DATABASE")
                .unwrap_or_else(|_| "zerg".to_string());
            info!(url = %mongo_url, db = %mongo_db_name, "Connecting to MongoDB...");
            let db = database::mongo::connect(&mongo_url, &mongo_db_name)
                .await
                .map_err(|e| eyre::eyre!("MongoDB connection failed: {}", e))?;
            info!("Connected to MongoDB");

            let handler = handlers::tasks_mongo::MongoTaskHandler::new(db);
            Router::new()
                .route(&route, post(handlers::tasks_mongo::handle_event))
                .with_state(handler)
        }

        // ── Fallback: Dapr state store for PG/Mongo without feature flags ──
        #[cfg(not(any(feature = "postgres", feature = "mongo")))]
        DatabaseBackend::Postgres | DatabaseBackend::Mongo => {
            let dapr = dapr_client::DaprClient::new();
            let store_name = config
                .backend
                .dapr_state_store_name()
                .expect("Dapr state store name must exist for PG/Mongo");
            info!(store = %store_name, "Using Dapr state store (no direct DB feature enabled)");
            let state_client = dapr_client::StateClient::new(dapr, store_name);
            let handler = handlers::dapr_state::DaprStateHandler::new(state_client);

            Router::new()
                .route(&route, post(handlers::dapr_state::handle_db_event))
                .with_state(handler)
        }

        // ── Qdrant: direct vector client ─────────────────────────
        #[cfg(feature = "qdrant")]
        DatabaseBackend::Qdrant => {
            let qdrant_url = std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "http://qdrant.dbs.svc.cluster.local:6334".to_string());
            let client = qdrant_client::Qdrant::from_url(&qdrant_url)
                .build()
                .wrap_err("Failed to create Qdrant client")?;
            let handler = handlers::qdrant::QdrantHandler::new(client);

            Router::new()
                .route(&route, post(handlers::qdrant::handle_vector_event))
                .with_state(handler)
        }

        // ── InfluxDB: direct time-series client ──────────────────
        #[cfg(feature = "influxdb")]
        DatabaseBackend::InfluxDb => {
            let influx_url = std::env::var("INFLUXDB_URL")
                .unwrap_or_else(|_| "http://influxdb.dbs.svc.cluster.local:8086".to_string());
            let influx_token =
                std::env::var("INFLUXDB_TOKEN").unwrap_or_else(|_| String::new());
            let influx_bucket =
                std::env::var("INFLUXDB_BUCKET").unwrap_or_else(|_| "zerg".to_string());
            let influx_org =
                std::env::var("INFLUXDB_ORG").unwrap_or_else(|_| "zerg".to_string());

            let client = influxdb2::Client::new(&influx_url, &influx_org, &influx_token);
            let handler =
                handlers::influxdb::InfluxDbHandler::new(client, influx_bucket, influx_org);

            Router::new()
                .route(&route, post(handlers::influxdb::handle_timeseries_event))
                .with_state(handler)
        }

        // ── Neo4j: direct graph client ───────────────────────────
        #[cfg(feature = "neo4j")]
        DatabaseBackend::Neo4j => {
            let neo4j_url = std::env::var("NEO4J_URL")
                .unwrap_or_else(|_| "bolt://neo4j.dbs.svc.cluster.local:7687".to_string());
            let neo4j_user =
                std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
            let neo4j_password =
                std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j".to_string());

            let graph = neo4rs::Graph::new(&neo4j_url, &neo4j_user, &neo4j_password)
                .await
                .wrap_err("Failed to connect to Neo4j")?;
            let handler = handlers::neo4j::Neo4jHandler::new(graph);

            Router::new()
                .route(&route, post(handlers::neo4j::handle_graph_event))
                .with_state(handler)
        }

        #[allow(unreachable_patterns)]
        backend => {
            return Err(eyre::eyre!(
                "Backend '{}' is not enabled. Compile with the appropriate feature flag.\n\
                 Available flags: --features postgres, --features mongo, \
                 --features qdrant, --features influxdb, --features neo4j",
                backend
            ));
        }
    };

    Ok(discovery.merge(handler_router))
}

/// Wait for shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating shutdown...");
        },
        _ = terminate => {
            info!("Received SIGTERM, initiating shutdown...");
        },
    }
}
