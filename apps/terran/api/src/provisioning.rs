//! JIT tenant provisioning: map a verified [`AuthIdentity`] to internal
//! user/organization rows, creating them on first sight (idempotent).
//!
//! Until Keycloak Organizations are configured (Phase 5), tokens carry no `org_id`,
//! so each user is mapped to their own personal workspace (`personal:{subject}`).
//! Once the token carries an org claim, that becomes the authoritative tenant.

use oidc_auth::AuthIdentity;
use uuid::Uuid;

use crate::db::{self, Db};

/// Internal tenant context for a request: which user, which org, and the coarse role.
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub role: String,
}

/// Resolve the internal tenant context for a request. Read-first: when the user, org,
/// and membership already exist this performs **only reads**; it provisions (writes)
/// just on first sight (e.g. a machine client's first Bearer call).
pub async fn resolve_tenant(db: &Db, identity: &AuthIdentity) -> sqlx::Result<TenantContext> {
    if let Some(ctx) = lookup_tenant(db, identity).await? {
        return Ok(ctx);
    }
    provision_tenant(db, identity).await
}

/// Read-only path: map a verified identity to existing internal rows, or `None` when
/// the tenant is not fully provisioned yet.
async fn lookup_tenant(db: &Db, identity: &AuthIdentity) -> sqlx::Result<Option<TenantContext>> {
    let Some(user_id) = db::find_user_id(db, &identity.subject).await? else {
        return Ok(None);
    };
    let (external_org, _, default_role) = derive_org(identity);
    let Some(org_id) = db::find_org_id(db, &external_org).await? else {
        return Ok(None);
    };
    // Membership must exist (the user belongs to this org). The coarse role itself is
    // taken from the verified token, not this DB snapshot, so it never drifts from the
    // IdP between logins (matches provision_tenant, which re-syncs the row at login).
    if db::find_membership_role(db, user_id, org_id)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let role = pick_role(&identity.roles).unwrap_or_else(|| default_role.to_string());
    Ok(Some(TenantContext {
        user_id,
        org_id,
        role,
    }))
}

/// Find-or-create the internal user + organization + membership for an identity
/// (idempotent). Called on first sight and from the login callback.
pub async fn provision_tenant(db: &Db, identity: &AuthIdentity) -> sqlx::Result<TenantContext> {
    let email = identity.email.as_deref().unwrap_or("");
    let user_id = db::upsert_user(db, &identity.subject, email, display_name(identity)).await?;

    let (external_org, org_name, default_role) = derive_org(identity);
    let org_id = db::upsert_org(db, &external_org, &org_name).await?;

    let role = pick_role(&identity.roles).unwrap_or_else(|| default_role.to_string());
    db::ensure_membership(db, user_id, org_id, &role).await?;

    Ok(TenantContext {
        user_id,
        org_id,
        role,
    })
}

/// Derive the tenant's external org id, display name, and default role from the token.
/// Until Keycloak Organizations are configured the token carries no org, so each user
/// maps to their own personal workspace (`personal:{subject}`), of which they are admin.
fn derive_org(identity: &AuthIdentity) -> (String, String, &'static str) {
    match &identity.org_id {
        Some(org) => (org.clone(), org.clone(), "member"),
        None => (
            format!("personal:{}", identity.subject),
            format!("{}'s workspace", display_name(identity)),
            "org_admin",
        ),
    }
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

/// First recognized a coarse role, in descending privilege.
fn pick_role(roles: &[String]) -> Option<String> {
    ["org_admin", "member", "viewer"]
        .into_iter()
        .find(|candidate| roles.iter().any(|r| r == candidate))
        .map(str::to_string)
}
