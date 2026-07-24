use crate::error::UserError;
use crate::oauth::types::OAuthState;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

/// TTL for OAuth state in Redis (10 minutes)
const STATE_TTL: i64 = 600;

/// Manages OAuth state and PKCE verifiers in Redis
#[derive(Clone)]
pub struct OAuthStateManager {
    redis: ConnectionManager,
}

impl OAuthStateManager {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    /// Generate a random state parameter for CSRF protection
    pub fn generate_state(&self) -> String {
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        const_hex::encode(random_bytes)
    }

    /// Generate PKCE verifier (challenge will be computed from it)
    pub fn generate_pkce_verifier(&self) -> String {
        use oauth2::PkceCodeChallenge;
        let (_pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        pkce_verifier.secret().clone()
    }

    /// Store OAuth state in Redis with TTL
    pub async fn store_state(&self, oauth_state: &OAuthState) -> Result<(), UserError> {
        let mut conn = self.redis.clone();
        let key = format!("oauth:state:{}", oauth_state.state);
        let value = serde_json::to_string(oauth_state)
            .map_err(|e| UserError::Internal(format!("Failed to serialize state: {e}")))?;

        conn.set_ex::<_, _, ()>(&key, value, STATE_TTL as u64)
            .await
            .map_err(|e| UserError::Internal(format!("Redis error: {e}")))?;

        Ok(())
    }

    /// Verify and consume OAuth state (atomic read-and-delete)
    pub async fn verify_and_consume_state(&self, state: &str) -> Result<OAuthState, UserError> {
        let mut conn = self.redis.clone();
        let key = format!("oauth:state:{state}");

        // GETDEL atomically gets and deletes the key (prevents replay attacks)
        let value: Option<String> = redis::cmd("GETDEL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| UserError::Internal(format!("Redis error: {e}")))?;

        match value {
            Some(v) => {
                let oauth_state: OAuthState = serde_json::from_str(&v)
                    .map_err(|e| UserError::OAuth(format!("Invalid state format: {e}")))?;
                Ok(oauth_state)
            }
            None => Err(UserError::OAuth(
                "Invalid or expired OAuth state".to_string(),
            )),
        }
    }
}
