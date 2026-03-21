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
        .route("/", get(list_connectors).post(create_connector))
        .route("/{id}", get(get_connector).delete(delete_connector))
        .route("/{id}/test", post(test_connector))
}

#[utoipa::path(get, path = "/api/analytics/connectors", responses((status = 200, body = Vec<Connector>)))]
pub async fn list_connectors() -> Json<Vec<Connector>> {
    Json(vec![])
}

#[utoipa::path(get, path = "/api/analytics/connectors/{id}", responses((status = 200, body = Connector)))]
pub async fn get_connector(Path(id): Path<String>) -> Result<Json<Connector>, StatusCode> {
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/api/analytics/connectors", request_body = CreateConnectorRequest, responses((status = 201, body = Connector)))]
pub async fn create_connector(
    Json(request): Json<CreateConnectorRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let _ = request;
    (StatusCode::CREATED, Json(serde_json::json!({"status": "created"})))
}

#[utoipa::path(post, path = "/api/analytics/connectors/{id}/test", responses((status = 200, body = ConnectorTestResult)))]
pub async fn test_connector(Path(id): Path<String>) -> Json<ConnectorTestResult> {
    let _ = id;
    Json(ConnectorTestResult {
        status: "ok".to_string(),
        error: None,
    })
}

async fn delete_connector(Path(id): Path<String>) -> StatusCode {
    let _ = id;
    StatusCode::NO_CONTENT
}
