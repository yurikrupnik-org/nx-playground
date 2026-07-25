use async_trait::async_trait;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, Result};

/// Server-side session record (token-handler / BFF model). The IdP tokens live here,
/// never in the browser — the browser holds only the opaque session id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub subject: String,
    pub org_id: Option<String>,
    pub roles: Vec<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Stored for RP-initiated logout (`id_token_hint`); the browser never sees it.
    pub id_token: Option<String>,
    /// Unix seconds at which the stored access token expires.
    pub access_expires_at: u64,
    /// Unix seconds at which the session is hard-expired regardless of refresh.
    /// Bounds how long a refreshed session can live before forcing re-login.
    pub session_expires_at: u64,
}

/// Storage for opaque server-side sessions. Implementations MUST **fail closed**:
/// any backend error surfaces as an error (the middleware then denies the request)
/// rather than being treated as "no session".
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Persist a new session and return its opaque id.
    async fn create(&self, record: &SessionRecord, ttl_secs: u64) -> Result<String>;
    /// Overwrite an existing session in place (after a token refresh), preserving the
    /// id. `ttl_secs` is the remaining lifetime to apply to the stored key.
    async fn update(&self, session_id: &str, record: &SessionRecord, ttl_secs: u64) -> Result<()>;
    /// Look up a session by id. `Ok(None)` means "not found"; `Err` means the store
    /// was unavailable (deny, do not bypass).
    async fn get(&self, session_id: &str) -> Result<Option<SessionRecord>>;
    /// Revoke a single session.
    async fn delete(&self, session_id: &str) -> Result<()>;
    /// Revoke every session for a user (e.g. on a security event / deactivation).
    async fn delete_all_for_user(&self, subject: &str) -> Result<()>;
}

/// Redis-backed [`SessionStore`].
///
/// Layout: `{prefix}:session:{id}` → JSON record (with TTL); `{prefix}:user:{subject}`
/// → set of session ids (for bulk revocation).
#[derive(Clone)]
pub struct RedisSessionStore {
    conn: ConnectionManager,
    prefix: String,
}

impl RedisSessionStore {
    /// Connect to Redis and build a store namespaced by `prefix` (e.g. `"terran"`).
    pub async fn connect(redis_url: &str, prefix: impl Into<String>) -> Result<Self> {
        let client =
            redis::Client::open(redis_url).map_err(|e| AuthError::Internal(e.to_string()))?;
        let conn = client
            .get_connection_manager()
            .await
            .map_err(|e| AuthError::Provider(format!("redis connect failed: {e}")))?;
        Ok(Self {
            conn,
            prefix: prefix.into(),
        })
    }

    /// Build a store from an already-connected [`ConnectionManager`], so callers can
    /// share one Redis connection across the session store and other Redis usage.
    pub fn from_manager(conn: ConnectionManager, prefix: impl Into<String>) -> Self {
        Self {
            conn,
            prefix: prefix.into(),
        }
    }

    fn session_key(&self, id: &str) -> String {
        format!("{}:session:{}", self.prefix, id)
    }

    fn user_key(&self, subject: &str) -> String {
        format!("{}:user:{}", self.prefix, subject)
    }

    /// 256 bits of opaque entropy (two v4 UUIDs), hex, no dashes.
    fn new_session_id() -> String {
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
    }
}

#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn create(&self, record: &SessionRecord, ttl_secs: u64) -> Result<String> {
        let id = Self::new_session_id();
        let payload =
            serde_json::to_string(record).map_err(|e| AuthError::Internal(e.to_string()))?;
        let mut conn = self.conn.clone();
        let skey = self.session_key(&id);
        let ukey = self.user_key(&record.subject);

        let _: () = conn
            .set_ex(&skey, payload, ttl_secs)
            .await
            .map_err(|e| AuthError::Provider(format!("redis set failed: {e}")))?;
        let _: () = conn
            .sadd(&ukey, &id)
            .await
            .map_err(|e| AuthError::Provider(format!("redis sadd failed: {e}")))?;
        // Keep the user index from outliving its sessions.
        let _: () = conn
            .expire(&ukey, ttl_secs as i64)
            .await
            .map_err(|e| AuthError::Provider(format!("redis expire failed: {e}")))?;
        Ok(id)
    }

    async fn update(&self, session_id: &str, record: &SessionRecord, ttl_secs: u64) -> Result<()> {
        let payload =
            serde_json::to_string(record).map_err(|e| AuthError::Internal(e.to_string()))?;
        let mut conn = self.conn.clone();
        let skey = self.session_key(session_id);
        let ukey = self.user_key(&record.subject);

        let _: () = conn
            .set_ex(&skey, payload, ttl_secs)
            .await
            .map_err(|e| AuthError::Provider(format!("redis set failed: {e}")))?;
        // Re-affirm the user index (subject is stable across refresh) and its TTL.
        let _: () = conn
            .sadd(&ukey, session_id)
            .await
            .map_err(|e| AuthError::Provider(format!("redis sadd failed: {e}")))?;
        let _: () = conn
            .expire(&ukey, ttl_secs as i64)
            .await
            .map_err(|e| AuthError::Provider(format!("redis expire failed: {e}")))?;
        Ok(())
    }

    async fn get(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        let mut conn = self.conn.clone();
        let payload: Option<String> = conn
            .get(self.session_key(session_id))
            .await
            .map_err(|e| AuthError::StoreUnavailable.wrap(e))?;
        match payload {
            None => Ok(None),
            Some(p) => {
                let rec = serde_json::from_str(&p)
                    .map_err(|e| AuthError::Internal(format!("session decode failed: {e}")))?;
                Ok(Some(rec))
            }
        }
    }

    async fn delete(&self, session_id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        // Read the record first so we can drop it from the user index.
        if let Ok(Some(rec)) = self.get(session_id).await {
            let _: std::result::Result<(), _> =
                conn.srem(self.user_key(&rec.subject), session_id).await;
        }
        let _: () = conn
            .del(self.session_key(session_id))
            .await
            .map_err(|e| AuthError::Provider(format!("redis del failed: {e}")))?;
        Ok(())
    }

    async fn delete_all_for_user(&self, subject: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let ukey = self.user_key(subject);
        let ids: Vec<String> = conn
            .smembers(&ukey)
            .await
            .map_err(|e| AuthError::Provider(format!("redis smembers failed: {e}")))?;
        for id in &ids {
            let _: std::result::Result<(), _> = conn.del(self.session_key(id)).await;
        }
        let _: std::result::Result<(), _> = conn.del(&ukey).await;
        Ok(())
    }
}

impl AuthError {
    /// Collapse a backend error into [`AuthError::StoreUnavailable`] while logging cause.
    fn wrap<E: std::fmt::Display>(self, cause: E) -> AuthError {
        tracing::warn!(%cause, "session store error -> failing closed");
        self
    }
}
