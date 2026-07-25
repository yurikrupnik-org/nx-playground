//! WorkOS management-API client (Organizations, Organization Memberships,
//! Invitations). Complements [`super::workos::WorkosProvider`], which only covers
//! the login/token seam; this client backs org onboarding and member management.
//!
//! All calls authenticate with the WorkOS API key (`sk_...`) as a Bearer token.
//! WorkOS ships no Rust SDK, so this is a thin `reqwest` wrapper over the REST
//! endpoints; list endpoints return the `{ "data": [...] }` envelope.

use serde::{Deserialize, Serialize};

use crate::error::{AuthError, Result};

/// Default WorkOS API base (mirrors `WorkosProvider`).
const DEFAULT_API_BASE: &str = "https://api.workos.com";

/// WorkOS management-API client.
pub struct WorkosAdmin {
    api_base: String,
    api_key: String,
    http: reqwest::Client,
}

/// A WorkOS organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkosOrganization {
    pub id: String,
    pub name: String,
}

/// A user as returned by `GET /user_management/users`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkosOrgUser {
    pub id: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

/// The role slug wrapper used by membership objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleSlug {
    pub slug: String,
}

/// An organization membership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkosMembership {
    pub id: String,
    pub user_id: String,
    pub organization_id: String,
    pub role: RoleSlug,
    pub status: String,
}

/// An invitation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkosInvitation {
    pub id: String,
    pub email: String,
    pub state: String,
    pub expires_at: Option<String>,
    pub organization_id: Option<String>,
}

/// The `{ "data": [...] }` list envelope.
#[derive(Deserialize)]
struct ListEnvelope<T> {
    data: Vec<T>,
}

impl WorkosAdmin {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_base: DEFAULT_API_BASE.to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Override the API base (tests / custom auth domains).
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        // reqwest's `.query()` sits behind a non-enabled cargo feature; build the
        // URL with query pairs directly (same approach as `WorkosProvider`).
        let url = reqwest::Url::parse_with_params(&format!("{}{}", self.api_base, path), query)
            .map_err(|e| AuthError::Internal(format!("bad workos url for {path}: {e}")))?;
        let mut req = self.http.request(method, url).bearer_auth(&self.api_key);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AuthError::Provider(format!("workos request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AuthError::Provider(format!(
                "workos {path} returned {status}: {text}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| AuthError::Provider(format!("workos {path} parse failed: {e}")))
    }

    /// `POST /organizations` — create an organization.
    pub async fn create_organization(&self, name: &str) -> Result<WorkosOrganization> {
        self.request(
            reqwest::Method::POST,
            "/organizations",
            &[],
            Some(serde_json::json!({ "name": name })),
        )
        .await
    }

    /// `GET /organizations/{id}` — fetch an organization (e.g. for its display name).
    pub async fn get_organization(&self, org_id: &str) -> Result<WorkosOrganization> {
        self.request(
            reqwest::Method::GET,
            &format!("/organizations/{org_id}"),
            &[],
            None,
        )
        .await
    }

    /// `POST /user_management/organization_memberships` — add a user to an org
    /// with the given role slug.
    pub async fn create_membership(
        &self,
        org_id: &str,
        user_id: &str,
        role_slug: &str,
    ) -> Result<WorkosMembership> {
        self.request(
            reqwest::Method::POST,
            "/user_management/organization_memberships",
            &[],
            Some(serde_json::json!({
                "organization_id": org_id,
                "user_id": user_id,
                "role_slug": role_slug,
            })),
        )
        .await
    }

    /// `GET /user_management/users?organization_id=...` — the org's users.
    pub async fn list_org_users(&self, org_id: &str) -> Result<Vec<WorkosOrgUser>> {
        let envelope: ListEnvelope<WorkosOrgUser> = self
            .request(
                reqwest::Method::GET,
                "/user_management/users",
                &[("organization_id", org_id), ("limit", "100")],
                None,
            )
            .await?;
        Ok(envelope.data)
    }

    /// `GET /user_management/organization_memberships?organization_id=...` —
    /// active memberships (carry the role slugs).
    pub async fn list_memberships(&self, org_id: &str) -> Result<Vec<WorkosMembership>> {
        let envelope: ListEnvelope<WorkosMembership> = self
            .request(
                reqwest::Method::GET,
                "/user_management/organization_memberships",
                &[
                    ("organization_id", org_id),
                    ("statuses", "active"),
                    ("limit", "100"),
                ],
                None,
            )
            .await?;
        Ok(envelope.data)
    }

    /// `GET /user_management/invitations?organization_id=...` — the org's invitations.
    pub async fn list_invitations(&self, org_id: &str) -> Result<Vec<WorkosInvitation>> {
        let envelope: ListEnvelope<WorkosInvitation> = self
            .request(
                reqwest::Method::GET,
                "/user_management/invitations",
                &[("organization_id", org_id), ("limit", "100")],
                None,
            )
            .await?;
        Ok(envelope.data)
    }

    /// `POST /user_management/invitations` — invite `email` to the org. WorkOS
    /// sends the invitation email and hosts the accept flow.
    pub async fn create_invitation(
        &self,
        org_id: &str,
        email: &str,
        role_slug: Option<&str>,
        inviter_user_id: Option<&str>,
    ) -> Result<WorkosInvitation> {
        let mut body = serde_json::json!({
            "organization_id": org_id,
            "email": email,
        });
        if let Some(role) = role_slug {
            body["role_slug"] = role.into();
        }
        if let Some(inviter) = inviter_user_id {
            body["inviter_user_id"] = inviter.into();
        }
        self.request(
            reqwest::Method::POST,
            "/user_management/invitations",
            &[],
            Some(body),
        )
        .await
    }

    /// `POST /user_management/invitations/{id}/revoke` — revoke a pending invitation.
    pub async fn revoke_invitation(&self, invitation_id: &str) -> Result<WorkosInvitation> {
        self.request(
            reqwest::Method::POST,
            &format!("/user_management/invitations/{invitation_id}/revoke"),
            &[],
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire-format mapping is the load-bearing part: WorkOS payloads must
    /// deserialize into the structs the BFF forwards to the SPA.
    #[test]
    fn deserializes_membership_envelope() {
        let json = serde_json::json!({
            "data": [{
                "object": "organization_membership",
                "id": "om_01",
                "user_id": "user_01",
                "organization_id": "org_01",
                "role": { "slug": "admin" },
                "status": "active",
                "created_at": "2026-01-01T00:00:00.000Z"
            }]
        });
        let env: ListEnvelope<WorkosMembership> = serde_json::from_value(json).unwrap();
        assert_eq!(env.data.len(), 1);
        assert_eq!(env.data[0].role.slug, "admin");
        assert_eq!(env.data[0].status, "active");
    }

    #[test]
    fn deserializes_invitation_and_org() {
        let inv: WorkosInvitation = serde_json::from_value(serde_json::json!({
            "object": "invitation",
            "id": "invitation_01",
            "email": "new@example.com",
            "state": "pending",
            "expires_at": "2026-02-01T00:00:00.000Z",
            "organization_id": "org_01",
            "token": "iGz…",
        }))
        .unwrap();
        assert_eq!(inv.state, "pending");

        let org: WorkosOrganization = serde_json::from_value(serde_json::json!({
            "object": "organization",
            "id": "org_01",
            "name": "Acme",
            "domains": []
        }))
        .unwrap();
        assert_eq!(org.name, "Acme");
    }

    #[test]
    fn with_api_base_overrides_default() {
        let admin = WorkosAdmin::new("sk_test").with_api_base("http://localhost:9999");
        assert_eq!(admin.api_base, "http://localhost:9999");
    }
}
