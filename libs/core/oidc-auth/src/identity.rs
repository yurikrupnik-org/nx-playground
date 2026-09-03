use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// Authenticated principal for a request, inserted into request extensions by the
/// `auth_required` middleware.
///
/// Identity is global (`subject`); authorization is tenant-scoped (`org_id`). The
/// `org_id` and `roles` are taken from the verified IdP token (or the server-side
/// session created from it) — never from app-local state — so they cannot drift
/// from the IdP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthIdentity {
    /// IdP subject (`sub`) — stable, unique user id.
    pub subject: String,
    /// Active organization (tenant) external id from the token, when present.
    pub org_id: Option<String>,
    /// Coarse roles from the token (Keycloak realm/client roles).
    pub roles: Vec<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    /// Opaque server-side session id (browser/BFF path); `None` on the Bearer path.
    pub session_id: Option<String>,
}

/// Prefix marking a B2C personal workspace's tenant ref.
pub const PERSONAL_PREFIX: &str = "personal:";

impl AuthIdentity {
    /// True when the principal carries `role`.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// The tenant this principal acts within, as an identity-provider reference.
    ///
    /// An `org_id` claim means the IdP organization is the tenant (B2B); no claim
    /// means the user's own personal workspace (B2C), `personal:{subject}`.
    ///
    /// This derivation is authorization-critical and **must** be identical in every
    /// service that scopes data by tenant - `apps/zerg/tasks` stores it as `org_ref`
    /// while `apps/zerg/api` mirrors it as `organizations.external_org_id`, and the
    /// two only agree because they call this one function. Do not reimplement it.
    pub fn tenant_ref(&self) -> String {
        match &self.org_id {
            Some(org) => org.clone(),
            None => format!("{PERSONAL_PREFIX}{}", self.subject),
        }
    }

    /// True when [`Self::tenant_ref`] denotes a personal workspace rather than a
    /// real IdP organization.
    pub fn is_personal_tenant(&self) -> bool {
        self.org_id.is_none()
    }
}

/// Handler extractor: pulls the `AuthIdentity` that `auth_required` placed in
/// request extensions. Missing identity is a 401 (the route must be guarded).
impl<S> FromRequestParts<S> for AuthIdentity
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthIdentity>()
            .cloned()
            .ok_or(AuthError::MissingCredentials)
    }
}
