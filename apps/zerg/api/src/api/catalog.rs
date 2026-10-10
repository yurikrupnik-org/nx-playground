//! Development-only data catalog: browse every database on the configured
//! Postgres server and preview rows.
//!
//! The introspection and row paging live in `database::postgres::catalog`
//! (generic infra, no domain knowledge); these handlers only map HTTP onto it.
//! The router is mounted by `api::routes` solely when `APP_ENV` is
//! development, so production serves 404 here.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use axum_helpers::AppError;
use database::postgres::catalog::{
    Catalog, CatalogColumn, CatalogDatabase, CatalogError, CatalogForeignKey, CatalogTable,
    DEFAULT_ROW_LIMIT, RelationKind, RowsPage,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};

use crate::openapi::SecurityAddon;
use crate::state::AppState;

/// OpenAPI documentation for the catalog routes (nested at `/catalog`).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Catalog API",
        version = "1.0.0",
        description = "Development-only Postgres catalog: databases, relations, columns, foreign keys and row previews"
    ),
    servers((url = "/api/catalog", description = "zerg-api mount path (development only)")),
    paths(list_databases, list_table_rows),
    components(schemas(
        DatabasesResponse,
        CatalogDatabase,
        CatalogTable,
        CatalogColumn,
        CatalogForeignKey,
        RelationKind,
        RowsPage
    )),
    modifiers(&SecurityAddon),
    tags((
        name = "catalog",
        description = "Live Postgres introspection (mounted only when APP_ENV=development)"
    ))
)]
pub struct CatalogApiDoc;

pub fn router(state: &AppState) -> Router {
    Router::new()
        .route("/databases", get(list_databases))
        .route(
            "/databases/{db}/tables/{schema}/{table}/rows",
            get(list_table_rows),
        )
        .with_state(state.clone())
}

/// Every browsable database on the server.
#[derive(Serialize, ToSchema)]
pub struct DatabasesResponse {
    pub databases: Vec<CatalogDatabase>,
}

/// Row page window; out-of-range values are clamped (`limit` to 1..=500).
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RowsQuery {
    /// Page size, default 50, clamped to 1..=500.
    pub limit: Option<i64>,
    /// Rows to skip, default 0.
    pub offset: Option<i64>,
}

fn catalog_error(e: CatalogError) -> AppError {
    match e {
        CatalogError::NotFound(what) => AppError::NotFound(format!("{what} not found")),
        CatalogError::Connect { .. } => AppError::ServiceUnavailable(e.to_string()),
        CatalogError::Query(_) | CatalogError::Decode(_) => {
            AppError::InternalServerError(e.to_string())
        }
    }
}

/// List databases with their tables, columns and foreign keys
#[utoipa::path(
    get,
    path = "/databases",
    tag = "catalog",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Databases on the server; one that cannot be introspected carries `error`", body = DatabasesResponse),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Catalog query failed", body = axum_helpers::ErrorResponse)
    )
)]
pub async fn list_databases(
    State(st): State<AppState>,
) -> Result<Json<DatabasesResponse>, AppError> {
    let databases = Catalog::new(&st.db, &st.config.database.url)
        .databases()
        .await
        .map_err(catalog_error)?;
    Ok(Json(DatabasesResponse { databases }))
}

/// Page through a table's rows
#[utoipa::path(
    get,
    path = "/databases/{db}/tables/{schema}/{table}/rows",
    tag = "catalog",
    params(
        ("db" = String, Path, description = "Database name"),
        ("schema" = String, Path, description = "Schema name"),
        ("table" = String, Path, description = "Table, view or materialized view name"),
        RowsQuery
    ),
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "One page of rows as JSON objects", body = RowsPage),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Database, schema or table not found", body = axum_helpers::ErrorResponse),
        (status = 500, description = "Row query failed", body = axum_helpers::ErrorResponse),
        (status = 503, description = "Database exists but is unreachable", body = axum_helpers::ErrorResponse)
    )
)]
pub async fn list_table_rows(
    State(st): State<AppState>,
    Path((db, schema, table)): Path<(String, String, String)>,
    Query(window): Query<RowsQuery>,
) -> Result<Json<RowsPage>, AppError> {
    let page = Catalog::new(&st.db, &st.config.database.url)
        .sample_rows(
            &db,
            &schema,
            &table,
            window.limit.unwrap_or(DEFAULT_ROW_LIMIT),
            window.offset.unwrap_or(0),
        )
        .await
        .map_err(catalog_error)?;
    Ok(Json(page))
}
