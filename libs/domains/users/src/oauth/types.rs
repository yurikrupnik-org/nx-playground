use serde::{Deserialize, Serialize};

/// OAuth provider enumeration for identifying which provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Github,
}

impl Provider {
    /// Stable lowercase identifier used in URLs and database rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::Github => "github",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Provider {
    type Err = crate::error::UserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("google") {
            Ok(Provider::Google)
        } else if s.eq_ignore_ascii_case("github") {
            Ok(Provider::Github)
        } else {
            Err(crate::error::UserError::Validation(format!(
                "Unsupported OAuth provider: {s}"
            )))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    pub provider_user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub username: Option<String>,
    pub raw_data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthState {
    pub state: String,
    pub pkce_verifier: String,
    pub nonce: Option<String>,
    pub redirect_uri: String,
    pub provider: String,
    /// The origin URL the user started the OAuth flow from (e.g. https://127.0.0.1.nip.io:8443)
    pub origin_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCallbackParams {
    pub code: String,
    pub state: String,
}
