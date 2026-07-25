use axum_helpers::server::{create_production_app, health_router};
use core_config::app_info;
use core_config::tracing::{init_tracing, install_color_eyre};
use domain_vector::{OpenAIProvider, QdrantConfig, QdrantRepository, VectorService};
use email::NotificationService;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

mod api;
mod config;
mod error;
mod grpc_pool;
mod openapi;
mod orgs;
mod state;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Install color-eyre first for colored error output (before any fallible operations)
    install_color_eyre();

    // Load configuration from environment variables
    let config = Config::from_env()?;

    // Initialize tracing with ErrorLayer for span trace capture.
    // Guard must outlive main so OTEL spans flush before the tokio runtime drops.
    let _tracing_guard = init_tracing(&config.environment, app_info!());

    // Install the global Prometheus recorder before any metric is emitted.
    let metrics_handle = axum_helpers::init_metrics()?;

    let tasks_addr =
        std::env::var("TASKS_SERVICE_ADDR").unwrap_or_else(|_| "http://[::1]:50051".to_string());

    info!(
        "Configured lazy TasksService client at {} (connects on first RPC)",
        tasks_addr
    );

    let grpc_pool::TasksClients {
        tasks: tasks_client,
        health: tasks_health,
    } = grpc_pool::create_optimized_tasks_clients(tasks_addr)?;

    // Initialize database connections concurrently
    let postgres_future = async {
        database::postgres::connect_from_config_with_retry(config.database.clone(), None)
            .await
            .map_err(|e| eyre::eyre!("PostgreSQL connection failed: {}", e))
    };

    let redis_future = async {
        database::redis::connect_from_config_with_retry(config.redis.clone(), None)
            .await
            .map_err(|e| eyre::eyre!("Redis connection failed: {}", e))
    };

    // Initialize NATS JetStream for notifications (with retry).
    let nats_future = async {
        info!("Connecting to NATS at {}", config.nats_url);
        messaging::nats::jetstream_with_retry(&config.nats_url, None)
            .await
            .map_err(|e| eyre::eyre!("NATS connection failed: {}", e))
    };

    let (db, redis, jetstream) = tokio::try_join!(postgres_future, redis_future, nats_future)?;
    let notifications = NotificationService::from_jetstream_default(jetstream);
    info!("NotificationService initialized with NATS JetStream");

    // WorkOS OIDC auth: server-side sessions + login-flow store (Redis), AuthKit
    // provider, and the JWKS verifier for the Bearer path.
    let sessions = Arc::new(oidc_auth::RedisSessionStore::from_manager(
        redis.clone(),
        "zerg",
    ));
    let flows = oidc_auth::LoginFlowStore::new(redis.clone(), "zerg", api::auth::FLOW_TTL_SECS);
    let provider = Arc::new(oidc_auth::WorkosProvider::new(
        &config.workos_client_id,
        &config.workos_api_key,
        config.callback_url(),
        &config.oidc_issuer,
    ));
    let workos_admin = Arc::new(oidc_auth::WorkosAdmin::new(&config.workos_api_key));
    let verifier = Arc::new(oidc_auth::OidcVerifier::new(
        oidc_auth::VerifierConfig::workos(&config.workos_client_id, &config.oidc_issuer),
    ));

    // Initialize Qdrant/Vector service (optional)
    let vector_service = match QdrantConfig::from_env() {
        Ok(qdrant_config) => {
            info!("Connecting to Qdrant...");
            match QdrantRepository::new(qdrant_config).await {
                Ok(qdrant_repo) => {
                    info!("Connected to Qdrant");
                    let service = VectorService::new(qdrant_repo);
                    // Optionally add embedding provider
                    let service = if let Ok(provider) = OpenAIProvider::from_env() {
                        info!("OpenAI embedding provider configured");
                        service.with_embedding_provider(Arc::new(provider))
                    } else {
                        info!("No embedding provider configured!");
                        service
                    };
                    Some(Arc::new(service))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to connect to Qdrant (vector service disabled): {}",
                        e
                    );
                    None
                }
            }
        }
        Err(_) => {
            info!("Qdrant not configured - vector service disabled");
            None
        }
    };

    // Initialize distributed rate limiter
    let rate_limiter = axum_helpers::RateLimiter::new(redis.clone(), config.rate_limit.clone());
    info!(
        "Rate limiter initialized (enabled={}, limit={}/{}s)",
        config.rate_limit.enabled,
        config.rate_limit.requests_per_window,
        config.rate_limit.window_secs
    );

    // Initialize the application state with database connections
    let state = AppState {
        config,
        tasks_client,
        tasks_health,
        db,
        redis,
        flows,
        sessions,
        provider,
        workos_admin,
        verifier,
        notifications,
        vector_service,
        rate_limiter,
    };

    // Build router with API routes (pass reference, not ownership!)
    let api_routes = api::routes(&state);

    // create_router adds docs/middleware to our composed routes
    let router = axum_helpers::create_router::<openapi::ApiDoc>(api_routes).await?;

    // Merge health endpoints into the app
    // - /health: liveness check with app name/version
    // - /ready: readiness check with actual db/redis health checks
    let app = router
        .merge(health_router(state.config.app.clone()))
        .merge(api::ready_router(state.clone()))
        // RED metrics for all routes above; /metrics is merged after so it is not tracked.
        .layer(axum::middleware::from_fn(axum_helpers::track_metrics))
        .merge(axum_helpers::metrics_router(metrics_handle));

    // Sample the Postgres pool into gauges every 15s (runs for the process lifetime).
    axum_helpers::spawn_pool_metrics(
        state.db.get_postgres_connection_pool().clone(),
        Duration::from_secs(15),
    );

    info!("Starting zerg API with production-ready shutdown (30s timeout)");

    // Production-ready server with graceful shutdown and cleanup
    // State moves here for cleanup
    create_production_app(
        app,
        &state.config.server,
        Duration::from_secs(30), // 30s graceful shutdown timeout
        async move {
            info!("Shutting down: closing database connections");

            // Close connections concurrently
            tokio::join!(
                async {
                    match state.db.close().await {
                        Ok(_) => info!("PostgreSQL connection closed successfully"),
                        Err(e) => tracing::error!("Error closing PostgreSQL: {}", e),
                    }
                },
                async {
                    // Redis ConnectionManager closes automatically on drop
                    drop(state.redis);
                    info!("Redis connection closed successfully");
                }
            );
        },
    )
    .await
    .map_err(|e| eyre::eyre!("Server error: {}", e))?;

    info!("Zerg API shutdown complete");
    Ok(())
}
