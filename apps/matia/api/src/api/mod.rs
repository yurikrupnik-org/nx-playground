use axum::Router;

pub mod catalog;
pub mod connectors;
pub mod issues;
pub mod lineage;
pub mod pipelines;
pub mod quality;

/// Creates the analytics API routes without the `/api` prefix.
/// The `/api` prefix is added by `create_router`.
/// Routes match the TypeScript api-client at `libs/matia/api-client/`.
pub fn routes(state: &crate::state::AppState) -> Router {
    Router::new()
        .nest("/analytics/catalog", catalog::router(state))
        .nest("/analytics/pipelines", pipelines::router(state))
        .nest("/analytics/quality", quality::router(state))
        .nest("/analytics/connectors", connectors::router(state))
        .nest("/analytics/issues", issues::router(state))
        .nest("/analytics/lineage", lineage::router(state))
}
