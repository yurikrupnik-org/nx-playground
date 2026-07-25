//! Server-side OAuth2 login flow: PKCE + CSRF `state`, persisted in Redis between the
//! authorize redirect and the callback.
//!
//! Generation uses the `oauth2` crate's vetted PKCE/CSRF helpers (no hand-rolled
//! base64/SHA), and the flow is consumed with an atomic `GETDEL` so a captured flow
//! cannot be replayed. The backend-for-frontend holds these secrets; the browser only
//! ever carries the opaque flow id in a short-lived cookie.

use oauth2::{CsrfToken, PkceCodeChallenge};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};

use crate::error::{AuthError, Result};

/// One in-flight login: the CSRF `state` echoed back by the IdP, the PKCE
/// `code_verifier` kept server-side, and the `code_challenge` (S256) sent to the IdP.
pub struct LoginFlow {
    pub state: String,
    pub code_verifier: String,
    pub code_challenge: String,
}

impl LoginFlow {
    /// Generate a fresh flow: a random CSRF `state` and an S256 PKCE challenge/verifier.
    pub fn new() -> Self {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        Self {
            state: CsrfToken::new_random().secret().clone(),
            code_verifier: verifier.secret().clone(),
            code_challenge: challenge.as_str().to_string(),
        }
    }
}

impl Default for LoginFlow {
    fn default() -> Self {
        Self::new()
    }
}

/// The part of a [`LoginFlow`] persisted between authorize and callback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredFlow {
    pub state: String,
    pub code_verifier: String,
}

/// Redis-backed store for in-flight login flows, keyed by an opaque flow id (held in a
/// short-lived cookie). Single-use: [`consume`](Self::consume) deletes atomically.
#[derive(Clone)]
pub struct LoginFlowStore {
    conn: ConnectionManager,
    prefix: String,
    ttl_secs: u64,
}

impl LoginFlowStore {
    /// Build a store namespaced by `prefix` (e.g. `"terran"`); flows expire after
    /// `ttl_secs`.
    pub fn new(conn: ConnectionManager, prefix: impl Into<String>, ttl_secs: u64) -> Self {
        Self {
            conn,
            prefix: prefix.into(),
            ttl_secs,
        }
    }

    fn key(&self, flow_id: &str) -> String {
        format!("{}:flow:{}", self.prefix, flow_id)
    }

    /// Persist a flow and return its opaque id. Store errors fail closed (deny login).
    pub async fn begin(&self, flow: &LoginFlow) -> Result<String> {
        let flow_id = CsrfToken::new_random().secret().clone();
        let payload = serde_json::to_string(&StoredFlow {
            state: flow.state.clone(),
            code_verifier: flow.code_verifier.clone(),
        })
        .map_err(|e| AuthError::Internal(e.to_string()))?;
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(self.key(&flow_id), payload, self.ttl_secs)
            .await
            .map_err(|e| fail_closed("flow store set failed", e))?;
        Ok(flow_id)
    }

    /// Atomically fetch and delete a flow. `Ok(None)` means "expired/unknown"
    /// (the caller rejects the callback); `Err` means the store was unavailable.
    pub async fn consume(&self, flow_id: &str) -> Result<Option<StoredFlow>> {
        let mut conn = self.conn.clone();
        let payload: Option<String> = redis::cmd("GETDEL")
            .arg(self.key(flow_id))
            .query_async(&mut conn)
            .await
            .map_err(|e| fail_closed("flow store getdel failed", e))?;
        match payload {
            None => Ok(None),
            Some(p) => Ok(Some(
                serde_json::from_str(&p).map_err(|e| AuthError::Internal(e.to_string()))?,
            )),
        }
    }
}

/// Log the backend cause and collapse to a fail-closed error.
fn fail_closed<E: std::fmt::Display>(context: &str, cause: E) -> AuthError {
    tracing::warn!(%cause, "{context} -> failing closed");
    AuthError::StoreUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_flows_are_unique_and_well_formed() {
        let a = LoginFlow::new();
        let b = LoginFlow::new();
        assert_ne!(a.state, b.state, "state is random per flow");
        assert_ne!(
            a.code_verifier, b.code_verifier,
            "verifier is random per flow"
        );
        // PKCE S256 challenge is base64url(no-pad) of SHA-256 -> 43 chars, url-safe.
        assert_eq!(a.code_challenge.len(), 43);
        assert!(
            a.code_challenge
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "challenge is base64url without padding"
        );
    }
}
