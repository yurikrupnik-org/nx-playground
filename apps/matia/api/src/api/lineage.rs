use axum::{
    Json,
    extract::Path,
    routing::get,
    Router,
};

use crate::state::AppState;

pub fn router(_state: &AppState) -> Router {
    Router::new()
        .route("/{name}", get(get_lineage))
}

#[utoipa::path(get, path = "/api/analytics/lineage/{name}", responses((status = 200, body = Vec<String>)))]
pub async fn get_lineage(Path(name): Path<String>) -> Json<Vec<String>> {
    let _ = name;
    Json(vec![])
}
