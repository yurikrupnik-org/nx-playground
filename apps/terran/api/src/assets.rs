//! `/api/assets` — the Phase 3 sample tenant-scoped resource (cloud inventory).
//! Every operation is scoped to the caller's resolved organization; there is no
//! cross-tenant access and no IDOR (an asset id from another org returns 404).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use oidc_auth::AuthIdentity;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::db::{self, NewAsset};
use crate::error::{ApiError, ApiResult};
use crate::provisioning::resolve_tenant;
use crate::state::AppState;

const PROVIDERS: [&str; 3] = ["aws", "gcp", "azure"];
const STATUSES: [&str; 4] = ["active", "stopped", "terminated", "unknown"];

/// `GET /api/assets` — list the caller's organization's assets.
#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Assets for the caller's organization", body = Vec<db::CloudAsset>),
        (status = 401, description = "Missing or invalid session")
    )
)]
pub async fn list_assets(
    State(st): State<AppState>,
    identity: AuthIdentity,
) -> ApiResult<Json<Vec<db::CloudAsset>>> {
    let tenant = resolve_tenant(&st.db, &identity).await?;
    let assets = db::list_assets_for_org(&st.db, tenant.org_id).await?;
    Ok(Json(assets))
}

/// `GET /api/assets/{id}` — fetch one asset within the caller's organization.
#[utoipa::path(
    get,
    path = "/assets/{id}",
    tag = "assets",
    params(("id" = Uuid, Path, description = "Asset id")),
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "The requested asset", body = db::CloudAsset),
        (status = 401, description = "Missing or invalid session"),
        (status = 404, description = "Asset not found in the caller's organization")
    )
)]
pub async fn get_asset(
    State(st): State<AppState>,
    identity: AuthIdentity,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::CloudAsset>> {
    let tenant = resolve_tenant(&st.db, &identity).await?;
    let asset = db::get_asset_for_org(&st.db, tenant.org_id, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "asset not found"))?;
    Ok(Json(asset))
}

/// `GET /api/assets/by-user/{user_id}` — list the caller's organization's assets
/// discovered by a specific user. The organization is always the caller's resolved
/// tenant (never a client-supplied value), so `user_id` can only narrow *within* that
/// org and never surface another tenant's data; an unknown/foreign user id yields `[]`.
#[utoipa::path(
    get,
    path = "/assets/by-user/{user_id}",
    tag = "assets",
    params(("user_id" = Uuid, Path, description = "Internal user id (users.id) that discovered the assets")),
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Assets discovered by the user within the caller's organization", body = Vec<db::CloudAsset>),
        (status = 401, description = "Missing or invalid session")
    )
)]
pub async fn list_assets_by_user(
    State(st): State<AppState>,
    identity: AuthIdentity,
    Path(user_id): Path<Uuid>,
) -> ApiResult<Json<Vec<db::CloudAsset>>> {
    let tenant = resolve_tenant(&st.db, &identity).await?;
    let assets = db::list_assets_for_user(&st.db, tenant.org_id, user_id).await?;
    Ok(Json(assets))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateAsset {
    pub provider: String,
    pub external_id: String,
    pub name: String,
    pub asset_type: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub monthly_cost: f64,
    #[serde(default)]
    pub metadata: Value,
}

/// `POST /api/assets` — create an asset in the caller's organization.
#[utoipa::path(
    post,
    path = "/assets",
    tag = "assets",
    request_body = CreateAsset,
    security(("session_cookie" = [])),
    responses(
        (status = 201, description = "Asset created", body = db::CloudAsset),
        (status = 400, description = "Invalid provider, status, or monthly_cost"),
        (status = 401, description = "Missing or invalid session")
    )
)]
pub async fn create_asset(
    State(st): State<AppState>,
    identity: AuthIdentity,
    Json(body): Json<CreateAsset>,
) -> ApiResult<(StatusCode, Json<db::CloudAsset>)> {
    if !PROVIDERS.contains(&body.provider.as_str()) {
        return Err(ApiError::bad_request("invalid provider (aws|gcp|azure)"));
    }
    let status = body.status.unwrap_or_else(|| "unknown".to_string());
    if !STATUSES.contains(&status.as_str()) {
        return Err(ApiError::bad_request(
            "invalid status (active|stopped|terminated|unknown)",
        ));
    }
    if body.monthly_cost < 0.0 {
        return Err(ApiError::bad_request("monthly_cost must be >= 0"));
    }

    let tenant = resolve_tenant(&st.db, &identity).await?;
    let input = NewAsset {
        provider: body.provider,
        external_id: body.external_id,
        name: body.name,
        asset_type: body.asset_type,
        region: body.region,
        status,
        monthly_cost: body.monthly_cost,
        metadata: if body.metadata.is_null() {
            json!({})
        } else {
            body.metadata
        },
    };
    let asset = db::create_asset(&st.db, tenant.org_id, Some(tenant.user_id), &input).await?;
    Ok((StatusCode::CREATED, Json(asset)))
}
