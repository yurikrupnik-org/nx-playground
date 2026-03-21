use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Router,
};

use crate::state::AppState;
use crate::types::*;

pub fn router(_state: &AppState) -> Router {
    Router::new()
        .route("/", get(list_pipelines).post(create_pipeline))
        .route("/{id}", get(get_pipeline).delete(delete_pipeline))
        .route("/{id}/run", post(run_pipeline))
        .route("/{id}/status", get(pipeline_status))
        .route("/{id}/result", get(pipeline_result))
}

#[utoipa::path(get, path = "/api/analytics/pipelines", responses((status = 200, body = Vec<Pipeline>)))]
pub async fn list_pipelines() -> Json<Vec<Pipeline>> {
    Json(vec![])
}

#[utoipa::path(get, path = "/api/analytics/pipelines/{id}", responses((status = 200, body = Pipeline)))]
pub async fn get_pipeline(Path(id): Path<String>) -> Result<Json<Pipeline>, StatusCode> {
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/api/analytics/pipelines", request_body = CreatePipelineRequest, responses((status = 201, body = Pipeline)))]
pub async fn create_pipeline(
    Json(request): Json<CreatePipelineRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let _ = request;
    (StatusCode::CREATED, Json(serde_json::json!({"status": "created"})))
}

#[utoipa::path(post, path = "/api/analytics/pipelines/{id}/run", responses((status = 202, body = PipelineRunResult)))]
pub async fn run_pipeline(Path(id): Path<String>) -> (StatusCode, Json<serde_json::Value>) {
    let _ = id;
    (StatusCode::ACCEPTED, Json(serde_json::json!({"status": "running"})))
}

pub async fn pipeline_status(Path(id): Path<String>) -> Result<Json<PipelineRunResult>, StatusCode> {
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

pub async fn pipeline_result(Path(id): Path<String>) -> Json<Vec<serde_json::Value>> {
    let _ = id;
    Json(vec![])
}

async fn delete_pipeline(Path(id): Path<String>) -> StatusCode {
    let _ = id;
    StatusCode::NO_CONTENT
}
