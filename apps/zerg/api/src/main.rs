use axum_helpers::server::{create_production_app, health_router};
use core_config::tracing::{init_tracing, install_color_eyre};
use domain_vector::{OpenAIProvider, QdrantConfig, QdrantRepository, VectorService};
use email::NotificationService;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

mod api;
mod config;
mod grpc_pool;
mod openapi;
mod state;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Install color-eyre first for colored error output (before any fallible operations)
    install_color_eyre();

    // Load configuration from environment variables
    let config = Config::from_env()?;

    // Initialize tracing with ErrorLayer for span trace capture
    init_tracing(&config.environment);

    let tasks_addr =
        std::env::var("TASKS_SERVICE_ADDR").unwrap_or_else(|_| "http://[::1]:50051".to_string());

    info!("Connecting to TasksService at {} (optimized)", tasks_addr);

    let tasks_client = grpc_pool::create_optimized_tasks_client(tasks_addr).await?;

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

    // Initialize NATS connection for notifications
    let nats_future = async {
        info!("Connecting to NATS at {}", &config.nats_url);
        async_nats::connect(&config.nats_url)
            .await
            .map_err(|e| eyre::eyre!("NATS connection failed: {}", e))
    };

    // Initialize MongoDB connection
    let mongo_future = async {
        let mongo_url =
            std::env::var("MONGODB_URL").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
        let mongo_db_name =
            std::env::var("MONGODB_DATABASE").unwrap_or_else(|_| "zerg".to_string());
        database::mongo::connect(&mongo_url, &mongo_db_name)
            .await
            .map_err(|e| eyre::eyre!("MongoDB connection failed: {}", e))
    };

    let (db, redis, nats_client, mongo_db) =
        tokio::try_join!(postgres_future, redis_future, nats_future, mongo_future)?;

    // Create JetStream context for notifications
    let jetstream = async_nats::jetstream::new(nats_client);
    let notifications = NotificationService::from_jetstream_default(jetstream);
    info!("NotificationService initialized with NATS JetStream");

    // Initialize JWT + Redis authentication
    let jwt_auth = axum_helpers::JwtRedisAuth::new(redis.clone(), &config.jwt)
        .map_err(|e| eyre::eyre!("Failed to initialize JWT auth: {}", e))?;

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

    // Initialize Dapr pub/sub client (optional - requires Dapr sidecar)
    let pubsub = if std::env::var("DAPR_ENABLED").unwrap_or_default() == "true" {
        let dapr = dapr_client::DaprClient::new();
        let pubsub_name =
            std::env::var("DAPR_PUBSUB_NAME").unwrap_or_else(|_| "pubsub-nats".to_string());
        info!(pubsub_name = %pubsub_name, "Dapr pub/sub client initialized");
        Some(dapr_client::PubSubClient::new(dapr, pubsub_name))
    } else {
        info!("Dapr not enabled - async DB endpoints disabled (set DAPR_ENABLED=true to enable)");
        None
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
        db,
        redis,
        jwt_auth,
        notifications,
        vector_service,
        mongo_db,
        rate_limiter,
        pubsub,
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
        .merge(api::ready_router(state.clone()));

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
