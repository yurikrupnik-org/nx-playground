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
        .route("/", post(configure_quality))
        .route("/{dataset}", get(quality_report))
}

#[utoipa::path(get, path = "/api/analytics/quality/{dataset}", responses((status = 200, body = QualityReport)))]
pub async fn quality_report(Path(dataset): Path<String>) -> Result<Json<QualityReport>, StatusCode> {
    let _ = dataset;
    Err(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/api/analytics/quality", request_body = ConfigureQualityRequest, responses((status = 204)))]
pub async fn configure_quality(Json(request): Json<ConfigureQualityRequest>) -> StatusCode {
    let _ = request;
    StatusCode::NO_CONTENT
}
