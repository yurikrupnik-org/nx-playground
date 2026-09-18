//! `GET /api/stacks` — frontend stack profiles.
//!
//! Reference data (seeded by `manifests/db/todo/migrations/*_stack_profiles.sql`)
//! answering "which UI language is the default and which is cheapest".
//! App-level metadata about the todo vertical's frontends, not todo domain
//! data — hence it lives in the API bin, not `domain_todo`. Rows are ordered
//! cheapest-first (total first-render transfer); consumers rely on that.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use sea_orm::{DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::Serialize;

/// One frontend stack profile row.
///
/// Mirrored by the `StackProfile` type in
/// `apps/todo/web-astro/src/lib/api.ts` — keep the two in sync.
#[derive(Debug, Serialize, FromQueryResult)]
pub struct StackProfile {
    pub slug: String,
    pub name: String,
    pub language: String,
    pub is_default: bool,
    pub js_kb: f32,
    pub html_kb: f32,
    pub requests: i32,
    pub notes: String,
}

pub fn router(db: DatabaseConnection) -> Router {
    Router::new().route("/", get(list_stacks)).with_state(db)
}

async fn list_stacks(
    State(db): State<DatabaseConnection>,
) -> Result<Json<Vec<StackProfile>>, (StatusCode, String)> {
    let rows = StackProfile::find_by_statement(Statement::from_string(
        DbBackend::Postgres,
        "SELECT slug, name, language, is_default, js_kb, html_kb, requests, notes \
         FROM stack_profiles ORDER BY js_kb + html_kb ASC",
    ))
    .all(&db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(rows))
}
