use redis::{AsyncCommands, RedisResult, aio::ConnectionManager};

/// Result of the combined blacklist/whitelist lookup for a JWT id.
#[derive(Debug, Clone, Copy)]
pub struct TokenStatus {
    /// The token has been explicitly revoked.
    pub blacklisted: bool,
    /// The token is present in the active-session whitelist.
    pub whitelisted: bool,
}

/// Redis-backed store for JWT authentication
/// Handles whitelist/blacklist for tokens and CSRF tokens
///
/// Methods take `&self`: `ConnectionManager` is a cheap handle that is cloned
/// internally per operation, so callers never need to clone the store.
#[derive(Clone)]
pub struct RedisAuthStore {
    client: ConnectionManager,
}

impl RedisAuthStore {
    pub fn new(manager: ConnectionManager) -> Self {
        tracing::info!("Redis auth store initialized");
        Self { client: manager }
    }

    /// Store JWT in whitelist with TTL
    pub async fn store_jwt_whitelist(
        &self,
        jti: &str,
        user_id: &str,
        ttl_seconds: u64,
    ) -> RedisResult<()> {
        let key = format!("jwt:whitelist:{jti}");
        let mut conn = self.client.clone();
        conn.set_ex::<_, _, ()>(&key, user_id, ttl_seconds).await?;
        Ok(())
    }

    /// Check if JWT is in whitelist
    pub async fn check_jwt_whitelist(&self, jti: &str) -> RedisResult<bool> {
        let key = format!("jwt:whitelist:{jti}");
        let mut conn = self.client.clone();
        conn.exists(&key).await
    }

    /// Add JWT to blacklist with TTL
    pub async fn blacklist_jwt(&self, jti: &str, ttl_seconds: u64) -> RedisResult<()> {
        let key = format!("jwt:blacklist:{jti}");
        let mut conn = self.client.clone();
        conn.set_ex::<_, _, ()>(&key, "1", ttl_seconds).await?;
        Ok(())
    }

    /// Check if JWT is in blacklist
    pub async fn check_jwt_blacklist(&self, jti: &str) -> RedisResult<bool> {
        let key = format!("jwt:blacklist:{jti}");
        let mut conn = self.client.clone();
        conn.exists(&key).await
    }

    /// Check blacklist and whitelist membership in a single round trip.
    pub async fn jwt_status(&self, jti: &str) -> RedisResult<TokenStatus> {
        let blacklist_key = format!("jwt:blacklist:{jti}");
        let whitelist_key = format!("jwt:whitelist:{jti}");
        let mut conn = self.client.clone();
        let (blacklisted, whitelisted): (bool, bool) = redis::pipe()
            .exists(&blacklist_key)
            .exists(&whitelist_key)
            .query_async(&mut conn)
            .await?;
        Ok(TokenStatus {
            blacklisted,
            whitelisted,
        })
    }

    /// Remove JWT from whitelist (on logout or refresh)
    pub async fn revoke_jwt_whitelist(&self, jti: &str) -> RedisResult<()> {
        let key = format!("jwt:whitelist:{jti}");
        let mut conn = self.client.clone();
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }

    /// Store CSRF token with TTL
    pub async fn store_csrf_token(&self, token: &str, ttl_seconds: u64) -> RedisResult<()> {
        let key = format!("csrf:{token}");
        let mut conn = self.client.clone();
        let _: () = conn.set_ex(&key, "1", ttl_seconds).await?;
        Ok(())
    }

    /// Validate and consume CSRF token (one-time use).
    ///
    /// `DEL` returns the number of keys removed, which makes the
    /// check-and-delete atomic without a Lua script.
    pub async fn validate_and_remove_csrf_token(&self, token: &str) -> RedisResult<bool> {
        let key = format!("csrf:{token}");
        let mut conn = self.client.clone();
        let removed: i64 = conn.del(&key).await?;
        Ok(removed == 1)
    }
}
