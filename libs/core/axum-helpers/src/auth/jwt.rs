use super::config::JwtConfig;
use super::store::RedisAuthStore;
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JWT token time-to-live constants
pub const ACCESS_TOKEN_TTL: i64 = 900; // 15 minutes
pub const REFRESH_TOKEN_TTL: i64 = 604800; // 7 days

/// Token-type discriminator in the `token_type` claim, so an access token and a
/// refresh token (otherwise identical) cannot be substituted for one another.
pub const TOKEN_TYPE_ACCESS: &str = "access";
pub const TOKEN_TYPE_REFRESH: &str = "refresh";
/// Issuer/audience bound into every token and enforced on verify.
pub const TOKEN_ISSUER: &str = "zerg-api";
pub const TOKEN_AUDIENCE: &str = "zerg-api";

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,        // Subject (user ID)
    pub email: String,      // User email
    pub name: String,       // User name
    pub roles: Vec<String>, // User roles
    pub exp: i64,           // Expiration time
    pub iat: i64,           // Issued at
    pub jti: String,        // JWT ID (for whitelist/blacklist)
    pub iss: String,        // Issuer
    pub aud: String,        // Audience
    pub token_type: String, // "access" | "refresh"
}

/// Hybrid JWT + Redis authentication
/// Combines stateless JWT tokens with Redis-backed whitelist/blacklist
#[derive(Clone)]
pub struct JwtRedisAuth {
    secret: String,
    store: RedisAuthStore,
}

impl JwtRedisAuth {
    /// Create a new JWT + Redis auth instance.
    ///
    /// # Arguments
    /// * `manager` - Redis connection manager
    /// * `config` - JWT configuration (use `JwtConfig::from_env()` or construct manually)
    ///
    /// # Example
    /// ```ignore
    /// use axum_helpers::{JwtConfig, JwtRedisAuth};
    /// use core_config::FromEnv;
    ///
    /// let config = JwtConfig::from_env()?;
    /// let jwt_auth = JwtRedisAuth::new(redis_manager, &config)?;
    /// ```
    pub fn new(manager: ConnectionManager, config: &JwtConfig) -> eyre::Result<Self> {
        let store = RedisAuthStore::new(manager);
        let secret = config.secret.clone();

        tracing::info!("JWT + Redis auth initialized");
        Ok(Self { secret, store })
    }

    /// Create access token (15 min)
    pub fn create_access_token(
        &self,
        user_id: &str,
        email: &str,
        name: &str,
        roles: &[String],
    ) -> eyre::Result<String> {
        self.create_token(
            user_id,
            email,
            name,
            roles,
            ACCESS_TOKEN_TTL,
            TOKEN_TYPE_ACCESS,
        )
    }

    /// Create refresh token (7 days)
    pub fn create_refresh_token(
        &self,
        user_id: &str,
        email: &str,
        name: &str,
        roles: &[String],
    ) -> eyre::Result<String> {
        self.create_token(
            user_id,
            email,
            name,
            roles,
            REFRESH_TOKEN_TTL,
            TOKEN_TYPE_REFRESH,
        )
    }

    /// Create JWT token with specified TTL
    fn create_token(
        &self,
        user_id: &str,
        email: &str,
        name: &str,
        roles: &[String],
        ttl_seconds: i64,
        token_type: &str,
    ) -> eyre::Result<String> {
        let now = Utc::now();
        let exp = (now + Duration::seconds(ttl_seconds)).timestamp();
        let iat = now.timestamp();
        let jti = Uuid::new_v4().to_string();

        let claims = JwtClaims {
            sub: user_id.to_string(),
            email: email.to_string(),
            name: name.to_string(),
            roles: roles.to_vec(),
            exp,
            iat,
            jti,
            iss: TOKEN_ISSUER.to_string(),
            aud: TOKEN_AUDIENCE.to_string(),
            token_type: token_type.to_string(),
        };

        let header = Header {
            alg: jsonwebtoken::Algorithm::HS256,
            ..Default::default()
        };

        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )?;

        Ok(token)
    }

    /// Verify JWT token signature and decode claims
    pub fn verify_token(&self, token: &str) -> eyre::Result<JwtClaims> {
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_issuer(&[TOKEN_ISSUER]);
        validation.set_audience(&[TOKEN_AUDIENCE]);
        let token_data = decode::<JwtClaims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &validation,
        )?;
        Ok(token_data.claims)
    }

    /// Verify a token and require it to be an **access** token (rejects a refresh
    /// token replayed on the API auth path).
    pub fn verify_access_token(&self, token: &str) -> eyre::Result<JwtClaims> {
        let claims = self.verify_token(token)?;
        if claims.token_type != TOKEN_TYPE_ACCESS {
            return Err(eyre::eyre!("expected an access token"));
        }
        Ok(claims)
    }

    /// Verify a token and require it to be a **refresh** token (the only token the
    /// refresh endpoint accepts).
    pub fn verify_refresh_token(&self, token: &str) -> eyre::Result<JwtClaims> {
        let claims = self.verify_token(token)?;
        if claims.token_type != TOKEN_TYPE_REFRESH {
            return Err(eyre::eyre!("expected a refresh token"));
        }
        Ok(claims)
    }

    /// Add token to whitelist in Redis
    pub async fn whitelist_token(&self, jti: &str, user_id: &str, ttl: u64) -> eyre::Result<()> {
        let mut store = self.store.clone();
        store
            .store_jwt_whitelist(jti, user_id, ttl)
            .await
            .map_err(|e| eyre::eyre!("Failed to whitelist token: {}", e))?;
        Ok(())
    }

    /// Check if token is whitelisted
    pub async fn is_token_whitelisted(&self, jti: &str) -> eyre::Result<bool> {
        let mut store = self.store.clone();
        store
            .check_jwt_whitelist(jti)
            .await
            .map_err(|e| eyre::eyre!("Failed to check whitelist: {}", e))
    }

    /// Add token to blacklist in Redis
    pub async fn blacklist_token(&self, jti: &str, ttl: u64) -> eyre::Result<()> {
        let mut store = self.store.clone();
        store
            .blacklist_jwt(jti, ttl)
            .await
            .map_err(|e| eyre::eyre!("Failed to blacklist token: {}", e))?;
        Ok(())
    }

    /// Check if token is blacklisted
    pub async fn is_token_blacklisted(&self, jti: &str) -> eyre::Result<bool> {
        let mut store = self.store.clone();
        store
            .check_jwt_blacklist(jti)
            .await
            .map_err(|e| eyre::eyre!("Failed to check blacklist: {}", e))
    }

    /// Remove token from whitelist (on logout/refresh)
    pub async fn revoke_token(&self, jti: &str) -> eyre::Result<()> {
        let mut store = self.store.clone();
        store
            .revoke_jwt_whitelist(jti)
            .await
            .map_err(|e| eyre::eyre!("Failed to revoke token: {}", e))?;
        Ok(())
    }
}
