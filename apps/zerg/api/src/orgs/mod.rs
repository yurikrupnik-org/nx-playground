//! JIT tenant provisioning for zerg (port of terran's `provisioning.rs` to SeaORM).
//!
//! Maps a verified [`AuthIdentity`] to internal user/org/membership rows, creating
//! them on first sight (idempotent). B2B vs B2C is decided by org membership, not
//! email domain: a token `org_id` claim means the WorkOS org is the tenant; no
//! claim means a personal workspace (`personal:{subject}`) where the user is admin.
//!
//! WorkOS is authoritative for orgs/memberships/invitations; Postgres mirrors them
//! only for FK integrity and task scoping.

pub mod entity;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use domain_users::{PostgresUserRepository, UserService};
use oidc_auth::AuthIdentity;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

use entity::{memberships, organizations};

/// Prefix marking a B2C personal workspace's external org id.
const PERSONAL_PREFIX: &str = "personal:";

/// Internal tenant context for a request: which user, which org, and the role.
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub external_org_id: String,
    pub org_name: String,
    /// WorkOS role slug; `admin` for personal-workspace owners.
    pub role: String,
}

impl TenantContext {
    pub fn is_personal(&self) -> bool {
        self.external_org_id.starts_with(PERSONAL_PREFIX)
    }

    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// Resolve the internal tenant context for a request. Read-first: when the user,
/// org, and membership already exist this performs only reads; it provisions
/// (writes) just on first sight (e.g. an invited user's first login).
pub async fn resolve_tenant(st: &AppState, identity: &AuthIdentity) -> ApiResult<TenantContext> {
    // A session always follows JIT user provisioning, so a missing row means the
    // principal was deleted out-of-band — treat as unauthenticated.
    let user = user_service(st)
        .get_user_by_subject(&identity.subject)
        .await
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "unknown principal"))?;

    let (external_org, default_role) = derive_org(identity);
    let existing = organizations::Entity::find()
        .filter(organizations::Column::ExternalOrgId.eq(external_org.as_str()))
        .one(&st.db)
        .await
        .map_err(db_err)?;

    let Some(org) = existing else {
        return provision_tenant(st, identity, user.id, Some(&user.name)).await;
    };

    let role = pick_role(identity).unwrap_or_else(|| default_role.to_string());
    ensure_membership(&st.db, user.id, org.id, &role).await?;

    Ok(TenantContext {
        user_id: user.id,
        org_id: org.id,
        external_org_id: org.external_org_id,
        org_name: org.name,
        role,
    })
}

/// Find-or-create the internal organization + membership for an identity
/// (idempotent). Called from the login callback and on first sight. `user_name`
/// (from the local user row) names personal workspaces; the WorkOS access token
/// carries no `name` claim.
pub async fn provision_tenant(
    st: &AppState,
    identity: &AuthIdentity,
    user_id: Uuid,
    user_name: Option<&str>,
) -> ApiResult<TenantContext> {
    let (external_org, default_role) = derive_org(identity);

    // Display name: personal workspaces derive it from the token; WorkOS orgs get
    // their real name from the management API, falling back to the raw org id so
    // provisioning never depends on WorkOS availability.
    let org_name = if external_org.starts_with(PERSONAL_PREFIX) {
        let owner = user_name
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| display_name(identity));
        format!("{owner}'s workspace")
    } else {
        match st.workos_admin.get_organization(&external_org).await {
            Ok(org) => org.name,
            Err(e) => {
                tracing::warn!(error = %e, org = %external_org, "workos org lookup failed; using id as name");
                external_org.clone()
            }
        }
    };

    let org_id = upsert_org(&st.db, &external_org, &org_name).await?;
    let role = pick_role(identity).unwrap_or_else(|| default_role.to_string());
    ensure_membership(&st.db, user_id, org_id, &role).await?;

    Ok(TenantContext {
        user_id,
        org_id,
        external_org_id: external_org,
        org_name,
        role,
    })
}

/// Tenant-context middleware: runs after `oidc_auth::auth_required`, resolves the
/// tenant for the verified identity, and injects [`TenantContext`] into request
/// extensions for downstream handlers.
///
/// It deliberately does **not** inject a scope for the tasks service. That service
/// derives its own tenant from the caller's forwarded access token, so anything this
/// process resolved locally would be an untrusted duplicate.
pub async fn tenant_context_mw(
    State(st): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let Some(identity) = req.extensions().get::<AuthIdentity>().cloned() else {
        // Programming error: this layer must sit inside auth_required.
        tracing::error!("tenant middleware ran without an AuthIdentity in extensions");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let tenant = match resolve_tenant(&st, &identity).await {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    req.extensions_mut().insert(tenant);
    next.run(req).await
}

// --- helpers ----------------------------------------------------------------------

fn user_service(st: &AppState) -> UserService<PostgresUserRepository> {
    UserService::new(PostgresUserRepository::new(st.db.clone()))
}

/// Derive the tenant's external org id and default role from the token.
///
/// The ref itself comes from [`AuthIdentity::tenant_ref`] so this mirror and the
/// `tasks` service's `org_ref` are guaranteed to agree - they read the same claim
/// through the same function.
fn derive_org(identity: &AuthIdentity) -> (String, &'static str) {
    let role = if identity.is_personal_tenant() {
        "admin"
    } else {
        "member"
    };
    (identity.tenant_ref(), role)
}

/// Best display name from the token, falling back to email then subject.
fn display_name(identity: &AuthIdentity) -> &str {
    identity
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(identity.email.as_deref())
        .unwrap_or(&identity.subject)
}

/// The WorkOS role slug from the verified token (`role` claim), if any.
fn pick_role(identity: &AuthIdentity) -> Option<String> {
    identity.roles.first().cloned()
}

fn db_err(e: sea_orm::DbErr) -> ApiError {
    tracing::error!(error = %e, "org provisioning database error");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// Insert-or-fetch an organization by external id (race-safe: conflicting inserts
/// no-op and the follow-up select wins).
async fn upsert_org(db: &DatabaseConnection, external: &str, name: &str) -> ApiResult<Uuid> {
    let am = organizations::ActiveModel {
        id: Set(Uuid::now_v7()),
        external_org_id: Set(external.to_string()),
        name: Set(name.to_string()),
        created_at: Set(chrono::Utc::now().into()),
    };
    organizations::Entity::insert(am)
        .on_conflict(
            OnConflict::column(organizations::Column::ExternalOrgId)
                .do_nothing()
                .to_owned(),
        )
        .try_insert()
        .exec(db)
        .await
        .map_err(db_err)?;

    organizations::Entity::find()
        .filter(organizations::Column::ExternalOrgId.eq(external))
        .one(db)
        .await
        .map_err(db_err)?
        .map(|o| o.id)
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
}

/// Upsert the membership row, re-syncing the role from the verified token so the
/// mirror never drifts from the IdP between logins.
async fn ensure_membership(
    db: &DatabaseConnection,
    user_id: Uuid,
    org_id: Uuid,
    role: &str,
) -> ApiResult<()> {
    let am = memberships::ActiveModel {
        user_id: Set(user_id),
        org_id: Set(org_id),
        role: Set(role.to_string()),
        created_at: Set(chrono::Utc::now().into()),
    };
    memberships::Entity::insert(am)
        .on_conflict(
            OnConflict::columns([memberships::Column::UserId, memberships::Column::OrgId])
                .update_column(memberships::Column::Role)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .map_err(db_err)?;
    Ok(())
}
