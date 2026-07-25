//! Organization endpoints (BFF over the WorkOS management API).
//!
//! WorkOS is the source of truth for orgs, memberships, and invitations; these
//! handlers add tenant guards (from [`TenantContext`]) and shape responses for the
//! SPA. Invitation emails and the accept flow are WorkOS-hosted: an invitee signs
//! up via the emailed AuthKit link, their token carries the org's `org_id`, and
//! the login callback JIT-provisions the local mirror — no accept endpoint here.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use oidc_auth::{SessionStore, cookie};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::orgs::{self, TenantContext};
use crate::state::AppState;

pub fn router(state: &AppState) -> Router {
    Router::new()
        .route("/", get(get_org).post(create_org))
        .route("/members", get(list_members))
        .route("/invitations", get(list_invitations).post(create_invitation))
        .route("/invitations/{id}/revoke", post(revoke_invitation))
        .with_state(state.clone())
}

/// The caller's active organization.
#[derive(Serialize, ToSchema)]
pub struct OrgResponse {
    pub id: Uuid,
    pub external_id: String,
    pub name: String,
    pub role: String,
    pub is_personal: bool,
}

impl From<&TenantContext> for OrgResponse {
    fn from(t: &TenantContext) -> Self {
        Self {
            id: t.org_id,
            external_id: t.external_org_id.clone(),
            name: t.org_name.clone(),
            role: t.role.clone(),
            is_personal: t.is_personal(),
        }
    }
}

/// An org member (WorkOS-shaped; local-only for personal workspaces).
#[derive(Serialize, ToSchema)]
pub struct MemberResponse {
    pub workos_user_id: String,
    pub email: String,
    pub name: String,
    pub role: String,
    pub status: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateOrgBody {
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct InviteBody {
    pub email: String,
    pub role: Option<String>,
}

/// `GET /api/org` → the caller's active org context.
#[utoipa::path(
    get,
    path = "",
    tag = "org",
    responses((status = 200, description = "Active organization", body = OrgResponse))
)]
pub async fn get_org(Extension(tenant): Extension<TenantContext>) -> Json<OrgResponse> {
    Json(OrgResponse::from(&tenant))
}

/// `POST /api/org` → self-serve org creation: create the WorkOS org + admin
/// membership, then re-mint the current session for the new org (org-switching
/// refresh grant) so no re-login is needed.
#[utoipa::path(
    post,
    path = "",
    tag = "org",
    request_body = CreateOrgBody,
    responses(
        (status = 200, description = "Organization created; session switched", body = OrgResponse),
        (status = 400, description = "Empty name, or caller already belongs to an organization"),
        (status = 502, description = "WorkOS error")
    )
)]
pub async fn create_org(
    State(st): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    identity: oidc_auth::AuthIdentity,
    headers: HeaderMap,
    Json(body): Json<CreateOrgBody>,
) -> ApiResult<Json<OrgResponse>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("organization name is required"));
    }
    if !tenant.is_personal() {
        return Err(ApiError::bad_request("already a member of an organization"));
    }

    // 1. WorkOS org + admin membership (the WorkOS user id IS the token subject).
    let org = st.workos_admin.create_organization(name).await?;
    st.workos_admin
        .create_membership(&org.id, &identity.subject, "admin")
        .await?;

    // 2. Re-mint the session for the new org. Failure here is recoverable by
    //    logging out and back in (org + membership already exist at WorkOS).
    let sid = cookie::parse(&headers, &st.config.cookie_name).ok_or_else(|| {
        tracing::error!("create_org reached without a session cookie");
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized")
    })?;
    let mut rec = st
        .sessions
        .get(sid)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let refresh_token = rec.refresh_token.clone().ok_or_else(|| {
        tracing::warn!("session has no refresh token; cannot switch org in place");
        relogin_err()
    })?;
    let tokens = st
        .provider
        .refresh_for_org(&refresh_token, &org.id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "org-scoped refresh failed after org creation");
            relogin_err()
        })?;
    let new_identity = st.verifier.verify(&tokens.access_token).await.map_err(|e| {
        tracing::warn!(error = %e, "verify failed after org-scoped refresh");
        relogin_err()
    })?;

    let now = now_secs();
    rec.access_token = tokens.access_token;
    if tokens.refresh_token.is_some() {
        rec.refresh_token = tokens.refresh_token;
    }
    rec.org_id = new_identity.org_id.clone();
    rec.roles = new_identity.roles.clone();
    rec.access_expires_at = now + tokens.expires_in.unwrap_or(300);
    let ttl = rec.session_expires_at.saturating_sub(now).max(1);
    st.sessions.update(sid, &rec, ttl).await.map_err(|e| {
        tracing::warn!(error = %e, "session update failed after org-scoped refresh");
        relogin_err()
    })?;

    // 3. Mirror locally (org row + admin membership) off the re-verified identity.
    let new_tenant = orgs::provision_tenant(&st, &new_identity, tenant.user_id, None).await?;

    Ok(Json(OrgResponse::from(&new_tenant)))
}

fn relogin_err() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "organization created — please log out and back in",
    )
}

/// `GET /api/org/members` → the org's members. Personal workspaces contain only
/// the owner; real orgs join WorkOS memberships (roles) with WorkOS users (emails).
#[utoipa::path(
    get,
    path = "/members",
    tag = "org",
    responses((status = 200, description = "Org members", body = Vec<MemberResponse>))
)]
pub async fn list_members(
    State(st): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    identity: oidc_auth::AuthIdentity,
) -> ApiResult<Json<Vec<MemberResponse>>> {
    if tenant.is_personal() {
        return Ok(Json(vec![MemberResponse {
            workos_user_id: identity.subject,
            email: identity.email.unwrap_or_default(),
            name: identity.name.unwrap_or_default(),
            role: "admin".to_string(),
            status: "active".to_string(),
        }]));
    }

    let ext = tenant.external_org_id.as_str();
    let (memberships, users) = tokio::try_join!(
        st.workos_admin.list_memberships(ext),
        st.workos_admin.list_org_users(ext),
    )?;

    let members = memberships
        .into_iter()
        .filter_map(|m| {
            let Some(user) = users.iter().find(|u| u.id == m.user_id) else {
                tracing::warn!(membership = %m.id, user = %m.user_id, "membership without a user record; skipping");
                return None;
            };
            let name = match (&user.first_name, &user.last_name) {
                (Some(f), Some(l)) => format!("{f} {l}"),
                (Some(f), None) => f.clone(),
                (None, Some(l)) => l.clone(),
                (None, None) => String::new(),
            };
            Some(MemberResponse {
                workos_user_id: m.user_id,
                email: user.email.clone().unwrap_or_default(),
                name,
                role: m.role.slug,
                status: m.status,
            })
        })
        .collect();

    Ok(Json(members))
}

/// `GET /api/org/invitations` → pending/recent invitations (empty for personal).
#[utoipa::path(
    get,
    path = "/invitations",
    tag = "org",
    responses((status = 200, description = "Org invitations"))
)]
pub async fn list_invitations(
    State(st): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<Vec<oidc_auth::WorkosInvitation>>> {
    if tenant.is_personal() {
        return Ok(Json(vec![]));
    }
    let invitations = st
        .workos_admin
        .list_invitations(&tenant.external_org_id)
        .await?;
    Ok(Json(invitations))
}

/// `POST /api/org/invitations` → invite `email` to the org (admin only). WorkOS
/// sends the email and hosts the accept flow.
#[utoipa::path(
    post,
    path = "/invitations",
    tag = "org",
    request_body = InviteBody,
    responses(
        (status = 200, description = "Invitation created"),
        (status = 400, description = "Personal workspaces cannot invite"),
        (status = 403, description = "Only org admins can invite"),
        (status = 502, description = "WorkOS error")
    )
)]
pub async fn create_invitation(
    State(st): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    identity: oidc_auth::AuthIdentity,
    Json(body): Json<InviteBody>,
) -> ApiResult<Json<oidc_auth::WorkosInvitation>> {
    guard_admin_org(&tenant)?;
    let invitation = st
        .workos_admin
        .create_invitation(
            &tenant.external_org_id,
            body.email.trim(),
            body.role.as_deref(),
            Some(&identity.subject),
        )
        .await?;
    Ok(Json(invitation))
}

/// `POST /api/org/invitations/{id}/revoke` → revoke a pending invitation (admin only).
#[utoipa::path(
    post,
    path = "/invitations/{id}/revoke",
    tag = "org",
    params(("id" = String, Path, description = "WorkOS invitation id")),
    responses(
        (status = 200, description = "Invitation revoked"),
        (status = 403, description = "Only org admins can revoke"),
        (status = 502, description = "WorkOS error")
    )
)]
pub async fn revoke_invitation(
    State(st): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<oidc_auth::WorkosInvitation>> {
    guard_admin_org(&tenant)?;
    let invitation = st.workos_admin.revoke_invitation(&id).await?;
    Ok(Json(invitation))
}

fn guard_admin_org(tenant: &TenantContext) -> ApiResult<()> {
    if tenant.is_personal() {
        return Err(ApiError::bad_request(
            "personal workspace cannot invite members",
        ));
    }
    if !tenant.is_admin() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only org admins can invite",
        ));
    }
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
