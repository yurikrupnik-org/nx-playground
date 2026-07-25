//! WorkOS AuthKit adapter (see `docs/auth-idp-decision.md`).
//!
//! Translates WorkOS's **proprietary** flow into the [`crate::provider::IdentityProvider`]
//! seam — it is OAuth2-flavored but not standard OIDC:
//!
//! - **Authorize:** `GET {api}/user_management/authorize` (hosted AuthKit;
//!   `provider=authkit`, or `GoogleOAuth`/`GitHubOAuth` for direct social).
//! - **Exchange/refresh/password:** `POST {api}/user_management/authenticate` with a
//!   JSON body carrying `client_secret` (the WorkOS API key) and the grant type. The
//!   response is a proprietary shape: a `user` object beside `access_token` +
//!   `refresh_token` — no `id_token`, no `expires_in`, and the access token itself
//!   carries no profile claims, so the profile rides in [`TokenSet::profile`].
//! - **Verify:** RS256 access tokens via JWKS at `{api}/sso/jwks/{client_id}` — reuses
//!   [`crate::verifier::OidcVerifier`] unchanged (claims: `sub`, `sid`, `org_id`,
//!   `role`, `permissions`; no `aud`/`azp`). Build the config with
//!   [`crate::verifier::VerifierConfig::workos`].
//! - **Logout:** top-level navigation to
//!   `{api}/user_management/sessions/logout?session_id={sid}&return_to={url}`; the
//!   `sid` comes from the access token, which the caller passes as the logout hint.

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;

use crate::error::{AuthError, Result};
use crate::provider::{IdentityProvider, TokenSet, UserProfile};

/// Default WorkOS API base; also the default token issuer (custom auth domains
/// change both).
const DEFAULT_API_BASE: &str = "https://api.workos.com";

/// WorkOS AuthKit identity provider.
pub struct WorkosProvider {
    api_base: String,
    client_id: String,
    /// The WorkOS API key (`sk_...`) — sent as `client_secret` in authenticate calls.
    client_secret: String,
    redirect_uri: String,
    issuer: String,
    jwks_url: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct AuthenticateResponse {
    access_token: String,
    refresh_token: Option<String>,
    user: Option<WorkosUser>,
}

#[derive(Deserialize)]
struct WorkosUser {
    id: String,
    email: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

impl WorkosProvider {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
        issuer: impl Into<String>,
    ) -> Self {
        let client_id = client_id.into();
        Self {
            api_base: DEFAULT_API_BASE.to_string(),
            jwks_url: format!("{DEFAULT_API_BASE}/sso/jwks/{client_id}"),
            client_id,
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            issuer: issuer.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Override the API base (tests / custom auth domains). Re-derives the JWKS URL.
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self.jwks_url = format!("{}/sso/jwks/{}", self.api_base, self.client_id);
        self
    }

    async fn authenticate(&self, body: serde_json::Value) -> Result<TokenSet> {
        let resp = self
            .http
            .post(format!("{}/user_management/authenticate", self.api_base))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Provider(format!("authenticate request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // WorkOS answers 400/401 for bad credentials, replayed codes, and dead
            // refresh tokens — surface those as a 401, not a 502.
            if matches!(
                status,
                reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNAUTHORIZED
            ) {
                return Err(AuthError::InvalidCredentials);
            }
            return Err(AuthError::Provider(format!(
                "authenticate endpoint returned {status}: {text}"
            )));
        }
        let r: AuthenticateResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::Provider(format!("authenticate parse failed: {e}")))?;
        let profile = r.user.map(|u| {
            let name = match (u.first_name, u.last_name) {
                (Some(f), Some(l)) => Some(format!("{f} {l}")),
                (Some(f), None) => Some(f),
                (None, Some(l)) => Some(l),
                (None, None) => None,
            };
            UserProfile {
                subject: u.id,
                email: u.email,
                name,
            }
        });
        Ok(TokenSet {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            id_token: None,
            // WorkOS reports no expires_in; callers fall back to a short cadence and
            // the middleware's lazy refresh keeps the session alive.
            expires_in: None,
            profile,
        })
    }
}

/// Extract the `sid` claim from a JWT without verifying it. The value only feeds a
/// logout redirect URL, so no trust is required.
fn unverified_sid(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("sid")?.as_str().map(str::to_string)
}

#[async_trait]
impl IdentityProvider for WorkosProvider {
    fn authorize_url(&self, state: &str, code_challenge: &str, idp_hint: Option<&str>) -> String {
        // Hint mapping: social providers deep-link past AuthKit; "sign-up" lands on
        // AuthKit's sign-up screen; everything else uses the hosted AuthKit flow.
        let (provider, screen_hint) = match idp_hint {
            Some("google") => ("GoogleOAuth", None),
            Some("github") => ("GitHubOAuth", None),
            Some("sign-up") => ("authkit", Some("sign-up")),
            _ => ("authkit", None),
        };
        let mut params: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("provider", provider),
        ];
        if let Some(hint) = screen_hint {
            params.push(("screen_hint", hint));
        }
        reqwest::Url::parse_with_params(
            &format!("{}/user_management/authorize", self.api_base),
            &params,
        )
        .expect("valid authorize url")
        .to_string()
    }

    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<TokenSet> {
        self.authenticate(serde_json::json!({
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": code_verifier,
        }))
        .await
    }

    async fn exchange_password(&self, username: &str, password: &str) -> Result<TokenSet> {
        self.authenticate(serde_json::json!({
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "password",
            "email": username,
            "password": password,
        }))
        .await
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenSet> {
        self.authenticate(serde_json::json!({
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .await
    }

    fn logout_url(&self, access_token_hint: Option<&str>, post_logout_redirect: &str) -> String {
        // WorkOS session logout needs the `sid` claim from the access token. Without
        // one the session can only be ended locally — send the browser straight home.
        let Some(sid) = access_token_hint.and_then(unverified_sid) else {
            return post_logout_redirect.to_string();
        };
        reqwest::Url::parse_with_params(
            &format!("{}/user_management/sessions/logout", self.api_base),
            &[
                ("session_id", sid.as_str()),
                ("return_to", post_logout_redirect),
            ],
        )
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

    fn provider() -> WorkosProvider {
        WorkosProvider::new(
            "client_123",
            "sk_test",
            "http://localhost:8080/api/auth/callback",
            "https://api.workos.com",
        )
    }

    /// Build an unsigned JWT with the given JSON payload (header/signature are dummies).
    fn unsigned_jwt(payload: serde_json::Value) -> String {
        let enc = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        format!(
            "{}.{}.{}",
            enc(br#"{"alg":"RS256","typ":"JWT"}"#),
            enc(payload.to_string().as_bytes()),
            enc(b"sig")
        )
    }

    #[test]
    fn authorize_url_defaults_to_authkit_with_pkce() {
        let url = provider().authorize_url("st4te", "ch4llenge", None);
        assert!(url.starts_with("https://api.workos.com/user_management/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client_123"));
        assert!(url.contains("state=st4te"));
        assert!(url.contains("code_challenge=ch4llenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("provider=authkit"));
        assert!(!url.contains("screen_hint"));
    }

    #[test]
    fn authorize_url_maps_social_and_signup_hints() {
        let p = provider();
        assert!(
            p.authorize_url("s", "c", Some("google"))
                .contains("provider=GoogleOAuth")
        );
        assert!(
            p.authorize_url("s", "c", Some("github"))
                .contains("provider=GitHubOAuth")
        );
        let signup = p.authorize_url("s", "c", Some("sign-up"));
        assert!(signup.contains("provider=authkit"));
        assert!(signup.contains("screen_hint=sign-up"));
        // Unknown hints fall back to hosted AuthKit rather than erroring.
        assert!(
            p.authorize_url("s", "c", Some("bogus"))
                .contains("provider=authkit")
        );
    }

    #[test]
    fn logout_url_extracts_sid_from_access_token() {
        let token = unsigned_jwt(serde_json::json!({
            "sub": "user_1", "sid": "session_abc123", "exp": 1
        }));
        let url = provider().logout_url(Some(&token), "http://localhost:3000/login");
        assert!(url.starts_with("https://api.workos.com/user_management/sessions/logout?"));
        assert!(url.contains("session_id=session_abc123"));
        assert!(url.contains("return_to=http%3A%2F%2Flocalhost%3A3000%2Flogin"));
    }

    #[test]
    fn logout_url_falls_back_without_sid() {
        let p = provider();
        // No hint at all.
        assert_eq!(
            p.logout_url(None, "http://localhost:3000/login"),
            "http://localhost:3000/login"
        );
        // Token without a sid claim.
        let token = unsigned_jwt(serde_json::json!({ "sub": "user_1" }));
        assert_eq!(
            p.logout_url(Some(&token), "http://localhost:3000/login"),
            "http://localhost:3000/login"
        );
    }
}
