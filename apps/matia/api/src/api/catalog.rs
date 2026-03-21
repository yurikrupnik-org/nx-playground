use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    routing::get,
    Router,
};

use crate::state::AppState;
use crate::types::*;

pub fn router(_state: &AppState) -> Router {
    Router::new()
        .route("/", get(list_datasets).post(register_dataset))
        .route("/{name}", get(get_dataset).delete(delete_dataset))
        .route("/{name}/sample", get(sample_dataset))
        .route("/{name}/stats", get(dataset_stats))
}

#[utoipa::path(get, path = "/api/analytics/catalog", responses((status = 200, body = Vec<DatasetMeta>)))]
pub async fn list_datasets() -> Json<Vec<DatasetMeta>> {
    Json(vec![])
}

#[utoipa::path(get, path = "/api/analytics/catalog/{name}", responses((status = 200, body = DatasetMeta)))]
pub async fn get_dataset(Path(name): Path<String>) -> Result<Json<DatasetMeta>, StatusCode> {
    let _ = name;
    Err(StatusCode::NOT_FOUND)
}

pub async fn sample_dataset(
    Path(name): Path<String>,
    Query(query): Query<SampleQuery>,
) -> Json<Vec<serde_json::Value>> {
    let _ = (name, query);
    Json(vec![])
}

pub async fn dataset_stats(Path(name): Path<String>) -> Json<serde_json::Value> {
    let _ = name;
    Json(serde_json::json!({}))
}

#[utoipa::path(post, path = "/api/analytics/catalog", request_body = RegisterDatasetRequest, responses((status = 201, body = DatasetMeta)))]
pub async fn register_dataset(
    Json(request): Json<RegisterDatasetRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let _ = request;
    (StatusCode::CREATED, Json(serde_json::json!({"status": "created"})))
}

async fn delete_dataset(Path(name): Path<String>) -> StatusCode {
    let _ = name;
    StatusCode::NO_CONTENT
}
