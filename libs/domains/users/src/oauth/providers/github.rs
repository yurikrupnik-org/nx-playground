use crate::error::UserError;
use crate::oauth::providers::{OAuthProvider, OAuthResult};
use crate::oauth::types::OAuthUserInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct GithubProvider {
    client_id: String,
    client_secret: String,
    http_client: reqwest::Client,
    oauth_http_client: oauth2::reqwest::Client,
}

#[derive(Debug, Serialize, Deserialize)]
struct GithubUserInfo {
    id: i64,
    login: String,
    email: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

impl GithubProvider {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            http_client: reqwest::Client::new(),
            oauth_http_client: oauth2::reqwest::Client::default(),
        }
    }

    /// Fetch the user's email of record from GitHub's /user/emails endpoint,
    /// together with GitHub's own `verified` attestation for it.
    ///
    /// Prefers the primary address, falling back to any verified one. Returns
    /// `None` when the endpoint is unavailable (e.g. missing `user:email`
    /// scope) — callers MUST then treat any email as unverified.
    async fn fetch_email_of_record(&self, access_token: &str) -> OAuthResult<Option<GithubEmail>> {
        let response = self
            .http_client
            .get("https://api.github.com/user/emails")
            .bearer_auth(access_token)
            .header("User-Agent", "Zerg-OAuth-App")
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let mut emails: Vec<GithubEmail> = response.json().await?;

        let chosen = emails
            .iter()
            .position(|e| e.primary)
            .or_else(|| emails.iter().position(|e| e.verified));

        Ok(chosen.map(|idx| emails.swap_remove(idx)))
    }
}

#[async_trait]
impl OAuthProvider for GithubProvider {
    fn name(&self) -> &'static str {
        "github"
    }

    fn required_scopes(&self) -> &'static [&'static str] {
        &["user:email", "read:user"]
    }

    fn auth_url(&self) -> &str {
        "https://github.com/login/oauth/authorize"
    }

    fn token_url(&self) -> &str {
        "https://github.com/login/oauth/access_token"
    }

    fn client_id(&self) -> &str {
        &self.client_id
    }

    fn client_secret(&self) -> &str {
        &self.client_secret
    }

    fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    fn oauth_http_client(&self) -> &oauth2::reqwest::Client {
        &self.oauth_http_client
    }

    async fn get_user_info(&self, access_token: &str) -> OAuthResult<OAuthUserInfo> {
        let response = self
            .http_client
            .get("https://api.github.com/user")
            .bearer_auth(access_token)
            .header("User-Agent", "Zerg-OAuth-App")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UserError::OAuth(format!(
                "GitHub API returned error: {}",
                response.status()
            )));
        }

        let user_info: GithubUserInfo = response.json().await?;

        let raw_data = serde_json::to_value(&user_info)?;

        // GitHub's /user `email` field carries no verification status, so the
        // /user/emails endpoint is the only source of truth for `verified`.
        // Reporting an unverified email as verified enables account takeover
        // through auto-linking.
        let (email, email_verified) = match self.fetch_email_of_record(access_token).await? {
            Some(record) => (Some(record.email), record.verified),
            None => (user_info.email, false),
        };

        Ok(OAuthUserInfo {
            provider_user_id: user_info.id.to_string(),
            email,
            email_verified,
            name: user_info.name,
            avatar_url: user_info.avatar_url,
            username: Some(user_info.login),
            raw_data,
        })
    }
}
