//! `/api/cloud-resources` — read-only inventory observed from the cluster by the
//! Crossplane `CloudInventory` composition (`platform/cloud-inventory/`).
//!
//! **Tenancy:** unlike `/api/assets`, this is org-global *platform* data —
//! cluster infrastructure, not tenant rows. Authentication is required, but no
//! organization filter is applied. The `platform.playground.io/inventory` claim
//! label is the seam for per-tenant scoping later: label claims with an org id
//! and force `ObservedFilter::claim` from the resolved tenant.
//!
//! Every operation is a read; there is no write path to the cluster.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use domain_cloud_resources::observed::{K8sInventory, ObservedCloudResource, ObservedFilter};
use oidc_auth::AuthIdentity;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The API boots without cluster access; inventory endpoints are then disabled.
fn inventory(st: &AppState) -> ApiResult<&K8sInventory> {
    st.inventory.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "cloud-resource inventory unavailable",
        )
    })
}

/// Cluster errors are an upstream dependency failure, not a client error.
/// Generic over the error type so this crate needs no direct `kube` dependency.
fn cluster_error<E: std::fmt::Display>(e: E) -> ApiError {
    tracing::error!(error = %e, "cloud inventory query failed");
    ApiError::new(StatusCode::BAD_GATEWAY, "cluster query failed")
}

/// `GET /api/cloud-resources` — list observed cloud resources.
#[utoipa::path(
    get,
    path = "/cloud-resources",
    tag = "cloud-resources",
    params(ObservedFilter),
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Observed cloud resources", body = Vec<ObservedCloudResource>),
        (status = 401, description = "Missing or invalid session"),
        (status = 502, description = "Cluster query failed"),
        (status = 503, description = "Inventory disabled (no cluster access)")
    )
)]
pub async fn list_cloud_resources(
    State(st): State<AppState>,
    _identity: AuthIdentity,
    Query(filter): Query<ObservedFilter>,
) -> ApiResult<Json<Vec<ObservedCloudResource>>> {
    let resources = inventory(&st)?.list(&filter).await.map_err(cluster_error)?;
    Ok(Json(resources))
}

/// `GET /api/cloud-resources/{id}` — fetch one observed cloud resource.
#[utoipa::path(
    get,
    path = "/cloud-resources/{id}",
    tag = "cloud-resources",
    params(("id" = Uuid, Path, description = "Inventory entry id (uid of the observing Object)")),
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "The requested cloud resource", body = ObservedCloudResource),
        (status = 401, description = "Missing or invalid session"),
        (status = 404, description = "No such observed cloud resource"),
        (status = 502, description = "Cluster query failed"),
        (status = 503, description = "Inventory disabled (no cluster access)")
    )
)]
pub async fn get_cloud_resource(
    State(st): State<AppState>,
    _identity: AuthIdentity,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ObservedCloudResource>> {
    let resource = inventory(&st)?
        .get(id)
        .await
        .map_err(cluster_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "cloud resource not found"))?;
    Ok(Json(resource))
}
