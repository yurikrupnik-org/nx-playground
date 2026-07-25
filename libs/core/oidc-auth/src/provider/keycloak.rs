use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AuthError, Result};
use crate::provider::{IdentityProvider, TokenSet};

/// Standard-OIDC identity provider backed by a Keycloak realm.
///
/// All endpoints are derived from the realm `issuer`
/// (`{base}/realms/{realm}`), so this also fits any OIDC IdP that follows the same
/// `protocol/openid-connect/*` layout.
pub struct KeycloakProvider {
    issuer: String,
    jwks_url: String,
    auth_endpoint: String,
    token_endpoint: String,
    logout_endpoint: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    scopes: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

impl KeycloakProvider {
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        let issuer = issuer.into();
        Self {
            jwks_url: format!("{issuer}/protocol/openid-connect/certs"),
            auth_endpoint: format!("{issuer}/protocol/openid-connect/auth"),
            token_endpoint: format!("{issuer}/protocol/openid-connect/token"),
            logout_endpoint: format!("{issuer}/protocol/openid-connect/logout"),
            issuer,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            scopes: "openid profile email".to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Override the requested OAuth scopes. For Keycloak Organizations, include
    /// `organization` so the access token carries the `organization` claim.
    pub fn with_scopes(mut self, scopes: impl Into<String>) -> Self {
        self.scopes = scopes.into();
        self
    }

    async fn token_request(&self, params: &[(&str, &str)]) -> Result<TokenSet> {
        // Build the application/x-www-form-urlencoded body via the URL serializer
        // (the `reqwest` feature set here does not expose `RequestBuilder::form`).
        let body = reqwest::Url::parse_with_params("http://form.local/", params)
            .map_err(|e| AuthError::Internal(e.to_string()))?
            .query()
            .unwrap_or_default()
            .to_string();
        let resp = self
            .http
            .post(&self.token_endpoint)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AuthError::Provider(format!("token request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Keycloak answers 400/401 `invalid_grant` for bad passwords, replayed
            // codes, and dead refresh tokens — surface that as a 401, not a 502.
            if matches!(
                status,
                reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNAUTHORIZED
            ) {
                return Err(AuthError::InvalidCredentials);
            }
            return Err(AuthError::Provider(format!(
                "token endpoint returned {status}: {body}"
            )));
        }
        let r: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::Provider(format!("token response parse failed: {e}")))?;
        Ok(TokenSet {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            id_token: r.id_token,
            expires_in: r.expires_in,
            profile: None,
        })
    }
}

#[async_trait]
impl IdentityProvider for KeycloakProvider {
    fn authorize_url(&self, state: &str, code_challenge: &str, idp_hint: Option<&str>) -> String {
        let mut params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("scope", &self.scopes),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
        ];
        if let Some(hint) = idp_hint {
            params.push(("kc_idp_hint", hint));
        }
        reqwest::Url::parse_with_params(&self.auth_endpoint, &params)
            .expect("valid authorize url")
            .to_string()
    }

    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<TokenSet> {
        self.token_request(&[
            ("grant_type", "authorization_code"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("code", code),
            ("redirect_uri", &self.redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .await
    }

    async fn exchange_password(&self, username: &str, password: &str) -> Result<TokenSet> {
        // Request the configured scopes so the access token carries `openid`/email and
        // the Keycloak `organization` claim, exactly like the authorization-code path.
        self.token_request(&[
            ("grant_type", "password"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("username", username),
            ("password", password),
            ("scope", &self.scopes),
        ])
        .await
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenSet> {
        self.token_request(&[
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    fn logout_url(&self, id_token_hint: Option<&str>, post_logout_redirect: &str) -> String {
        let mut params: Vec<(&str, &str)> = vec![
            ("post_logout_redirect_uri", post_logout_redirect),
            ("client_id", &self.client_id),
        ];
        if let Some(hint) = id_token_hint {
            params.push(("id_token_hint", hint));
        }
        reqwest::Url::parse_with_params(&self.logout_endpoint, &params)
            .expect("valid logout url")
            .to_string()
    }

    fn issuer(&self) -> &str {
        &self.issuer
    }

    fn jwks_url(&self) -> &str {
        &self.jwks_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> KeycloakProvider {
        KeycloakProvider::new(
            "https://kc.test/realms/terran",
            "terran-api",
            "secret",
            "https://app.test/api/auth/callback",
        )
    }

    #[test]
    fn derives_endpoints_from_issuer() {
        let p = provider();
        assert_eq!(p.issuer(), "https://kc.test/realms/terran");
        assert_eq!(
            p.jwks_url(),
            "https://kc.test/realms/terran/protocol/openid-connect/certs"
        );
    }

    #[test]
    fn authorize_url_carries_pkce_and_idp_hint() {
        let p = provider();
        let url = p.authorize_url("xyz-state", "chal-123", Some("google"));
        assert!(url.starts_with("https://kc.test/realms/terran/protocol/openid-connect/auth?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=terran-api"));
        assert!(url.contains("code_challenge=chal-123"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=xyz-state"));
        assert!(url.contains("kc_idp_hint=google"));
        // redirect_uri is URL-encoded
        assert!(url.contains("redirect_uri=https%3A%2F%2Fapp.test%2Fapi%2Fauth%2Fcallback"));
    }

    #[test]
    fn authorize_url_omits_hint_when_absent() {
        let p = provider();
        let url = p.authorize_url("s", "c", None);
        assert!(!url.contains("kc_idp_hint"));
    }

    #[test]
    fn logout_url_includes_post_logout_and_hint() {
        let p = provider();
        let url = p.logout_url(Some("id-token-abc"), "https://app.test/");
        assert!(url.starts_with("https://kc.test/realms/terran/protocol/openid-connect/logout?"));
        assert!(url.contains("id_token_hint=id-token-abc"));
        assert!(url.contains("post_logout_redirect_uri=https%3A%2F%2Fapp.test%2F"));
    }
}
