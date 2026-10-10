//! The `IdentityProvider` seam: the only layer that diverges between providers.
//!
//! Verification (JWKS/RS256) and sessions are provider-agnostic; only the login /
//! token-acquisition flow differs. Keycloak speaks standard OIDC ([`keycloak`]);
//! WorkOS would need a proprietary REST adapter ([`workos`], deferred).

pub mod keycloak;
pub mod workos;
pub mod workos_admin;

use async_trait::async_trait;

use crate::error::Result;

/// Tokens returned by an identity provider after code exchange or refresh.
#[derive(Debug, Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Access-token lifetime in seconds, when reported.
    pub expires_in: Option<u64>,
    /// Provider-supplied user profile, for providers whose access tokens carry no
    /// profile claims (WorkOS returns a `user` object beside the tokens; Keycloak
    /// tokens already embed email/name, so its adapter leaves this `None`).
    pub profile: Option<UserProfile>,
}

/// Minimal user profile delivered out-of-band with a [`TokenSet`].
#[derive(Debug, Clone)]
pub struct UserProfile {
    /// Provider user id (matches the token `sub`).
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Abstraction over an OIDC identity provider's login/acquisition flow.
///
/// The backend-for-frontend holds the client secret and performs these calls; the
/// browser never sees an IdP token (see the token-handler model in the plan).
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Build the authorization redirect URL (PKCE). `idp_hint` deep-links a brokered
    /// social provider (e.g. `"google"`/`"github"`) via Keycloak's `kc_idp_hint`.
    fn authorize_url(&self, state: &str, code_challenge: &str, idp_hint: Option<&str>) -> String;

    /// Exchange an authorization `code` (+ PKCE `code_verifier`) for tokens.
    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<TokenSet>;

    /// Exchange a username + password directly for tokens (OAuth2 Resource Owner
    /// Password Credentials / Keycloak "Direct Access Grant"). Backs a native
    /// in-app login form; the BFF holds the client secret and never persists the
    /// password. Returns [`crate::error::AuthError::InvalidCredentials`] on a bad pair.
    async fn exchange_password(&self, username: &str, password: &str) -> Result<TokenSet>;

    /// Exchange a refresh token for a fresh token set.
    async fn refresh(&self, refresh_token: &str) -> Result<TokenSet>;

    /// Build the RP-initiated logout URL. The `hint` is provider-specific: Keycloak
    /// expects the raw `id_token` (sent as `id_token_hint`); WorkOS expects the
    /// **access token**, from which the adapter extracts the `sid` session claim.
    fn logout_url(&self, id_token_hint: Option<&str>, post_logout_redirect: &str) -> String;

    /// Issuer this provider authenticates against (feeds the shared verifier).
    fn issuer(&self) -> &str;

    /// JWKS endpoint (feeds the shared verifier).
    fn jwks_url(&self) -> &str;
}
