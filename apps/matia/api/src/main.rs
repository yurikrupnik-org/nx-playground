use axum_helpers::server::{create_production_app, health_router};
use core_config::tracing::{init_tracing, install_color_eyre};
use std::time::Duration;
use tracing::info;

mod api;
mod config;
mod openapi;
mod state;
mod types;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    install_color_eyre();

    let config = Config::from_env()?;
    init_tracing(&config.environment);

    let state = AppState {
        config,
    };

    // Build router with analytics API routes
    let api_routes = api::routes(&state);

    // create_router adds /api prefix, OpenAPI docs, compression, tracing
    let router = axum_helpers::create_router::<openapi::ApiDoc>(api_routes).await?;

    // Merge health endpoints: /health (liveness), no /ready yet (no DB)
    let app = router.merge(health_router(state.config.app.clone()));

    info!("Starting Matia API on {}:{}", state.config.server.host, state.config.server.port);

    create_production_app(
        app,
        &state.config.server,
        Duration::from_secs(30),
        async {
            info!("Matia API shutdown complete");
        },
    )
    .await
    .map_err(|e| eyre::eyre!("Server error: {}", e))?;

    Ok(())
}
