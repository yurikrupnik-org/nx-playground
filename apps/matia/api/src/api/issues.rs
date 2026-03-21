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
        .route("/", get(list_issues))
        .route("/{id}", get(get_issue).put(update_issue))
}

#[utoipa::path(get, path = "/api/analytics/issues", responses((status = 200, body = Vec<Issue>)))]
pub async fn list_issues(Query(query): Query<IssueListQuery>) -> Json<Vec<Issue>> {
    let _ = query;
    Json(vec![])
}

#[utoipa::path(get, path = "/api/analytics/issues/{id}", responses((status = 200, body = Issue)))]
pub async fn get_issue(Path(id): Path<String>) -> Result<Json<Issue>, StatusCode> {
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

#[utoipa::path(put, path = "/api/analytics/issues/{id}", responses((status = 200, body = Issue)))]
pub async fn update_issue(
    Path(id): Path<String>,
    Json(request): Json<UpdateIssueRequest>,
) -> Result<Json<Issue>, StatusCode> {
    let _ = (id, request);
    Err(StatusCode::NOT_FOUND)
}
