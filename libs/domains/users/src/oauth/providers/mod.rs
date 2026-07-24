pub mod github;
pub mod google;

use crate::oauth::types::{OAuthUserInfo, TokenResponse};
use async_trait::async_trait;

pub type OAuthResult<T> = Result<T, crate::error::UserError>;

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn required_scopes(&self) -> &'static [&'static str];

    fn auth_url(&self) -> &str;
    fn token_url(&self) -> &str;
    fn client_id(&self) -> &str;
    fn client_secret(&self) -> &str;
    /// Shared client for provider REST calls (user info, emails).
    fn http_client(&self) -> &reqwest::Client;
    /// Shared client for the oauth2 token exchange.
    ///
    /// Separate from [`Self::http_client`] because the `oauth2` crate is pinned
    /// to a different major version of `reqwest` than the workspace.
    fn oauth_http_client(&self) -> &oauth2::reqwest::Client;

    /// Generate OAuth authorization URL with PKCE support
    fn authorize_url(
        &self,
        state: &str,
        pkce_verifier_str: &str,
        redirect_uri: &str,
        _nonce: Option<&str>,
    ) -> Result<String, crate::error::UserError> {
        use oauth2::{
            AuthUrl, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
            RedirectUrl, Scope, basic::BasicClient,
        };

        let auth_url = AuthUrl::new(self.auth_url().to_string())
            .map_err(|e| crate::error::UserError::OAuth(format!("Invalid auth URL: {e}")))?;
        let redirect_url = RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| crate::error::UserError::OAuth(format!("Invalid redirect URL: {e}")))?;

        let client = BasicClient::new(ClientId::new(self.client_id().to_string()))
            .set_client_secret(ClientSecret::new(self.client_secret().to_string()))
            .set_auth_uri(auth_url)
            .set_redirect_uri(redirect_url);

        let pkce_verifier = PkceCodeVerifier::new(pkce_verifier_str.to_string());
        let pkce_challenge = PkceCodeChallenge::from_code_verifier_sha256(&pkce_verifier);

        let auth_request = self
            .required_scopes()
            .iter()
            .fold(
                client.authorize_url(|| CsrfToken::new(state.to_string())),
                |acc, scope| acc.add_scope(Scope::new(scope.to_string())),
            )
            .set_pkce_challenge(pkce_challenge);

        let (url, _) = auth_request.url();
        Ok(url.to_string())
    }

    /// Exchange authorization code for access token with PKCE
    async fn exchange_code(
        &self,
        code: &str,
        pkce_verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, crate::error::UserError> {
        use oauth2::{
            AuthUrl, AuthorizationCode, ClientId, ClientSecret, PkceCodeVerifier, RedirectUrl,
            TokenResponse as OAuth2TokenResponse, TokenUrl, basic::BasicClient,
        };

        let client =
            BasicClient::new(ClientId::new(self.client_id().to_string()))
                .set_client_secret(ClientSecret::new(self.client_secret().to_string()))
                .set_auth_uri(AuthUrl::new(self.auth_url().to_string()).map_err(|e| {
                    crate::error::UserError::OAuth(format!("Invalid auth URL: {e}"))
                })?)
                .set_token_uri(TokenUrl::new(self.token_url().to_string()).map_err(|e| {
                    crate::error::UserError::OAuth(format!("Invalid token URL: {e}"))
                })?)
                .set_redirect_uri(RedirectUrl::new(redirect_uri.to_string()).map_err(|e| {
                    crate::error::UserError::OAuth(format!("Invalid redirect URL: {e}"))
                })?);

        let pkce_verifier = PkceCodeVerifier::new(pkce_verifier.to_string());

        let token_result = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .set_pkce_verifier(pkce_verifier)
            .request_async(self.oauth_http_client())
            .await
            .map_err(|e| crate::error::UserError::OAuth(format!("Failed to exchange code: {e}")))?;

        Ok(TokenResponse {
            access_token: token_result.access_token().secret().clone(),
            refresh_token: token_result.refresh_token().map(|t| t.secret().clone()),
            expires_in: token_result.expires_in().map(|d| d.as_secs()),
            token_type: "Bearer".to_string(),
        })
    }

    /// Fetch user information from the OAuth provider
    async fn get_user_info(&self, access_token: &str) -> OAuthResult<OAuthUserInfo>;
}
