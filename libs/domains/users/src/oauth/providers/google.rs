use crate::error::UserError;
use crate::oauth::providers::{OAuthProvider, OAuthResult};
use crate::oauth::types::OAuthUserInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct GoogleProvider {
    client_id: String,
    client_secret: String,
    http_client: reqwest::Client,
    oauth_http_client: oauth2::reqwest::Client,
}

#[derive(Debug, Serialize, Deserialize)]
struct GoogleUserInfo {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    picture: Option<String>,
}

impl GoogleProvider {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            http_client: reqwest::Client::new(),
            oauth_http_client: oauth2::reqwest::Client::default(),
        }
    }
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
    }

    fn required_scopes(&self) -> &'static [&'static str] {
        &["openid", "email", "profile"]
    }

    fn auth_url(&self) -> &str {
        "https://accounts.google.com/o/oauth2/v2/auth"
    }

    fn token_url(&self) -> &str {
        "https://oauth2.googleapis.com/token"
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
            .get("https://openidconnect.googleapis.com/v1/userinfo")
            .bearer_auth(access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UserError::OAuth(format!(
                "Google API returned error: {}",
                response.status()
            )));
        }

        let user_info: GoogleUserInfo = response.json().await?;

        let raw_data = serde_json::to_value(&user_info)?;

        Ok(OAuthUserInfo {
            provider_user_id: user_info.sub,
            email: user_info.email.clone(),
            email_verified: user_info.email_verified.unwrap_or(false),
            name: user_info.name,
            avatar_url: user_info.picture,
            username: user_info.email,
            raw_data,
        })
    }
}
