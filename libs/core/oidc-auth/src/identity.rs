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

impl AuthIdentity {
    /// True when the principal carries `role`.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
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
