//! Typed authentication errors.

use thiserror::Error;

/// Errors from JWT creation/verification and the Redis-backed token store.
///
/// Variants are distinct so callers can tell an expired token from a forged
/// one and a client failure from an unavailable auth store, while `source()`
/// preserves the underlying `jsonwebtoken`/`redis` error chain.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The token's `exp` claim is in the past.
    #[error("token expired")]
    Expired(#[source] jsonwebtoken::errors::Error),

    /// The token failed validation (bad signature, issuer, audience, shape, ...).
    #[error("invalid token")]
    Invalid(#[source] jsonwebtoken::errors::Error),

    /// The token verified but is of the wrong type for this code path
    /// (e.g. a refresh token presented on the access path).
    #[error("expected {expected} token, got {actual:?}")]
    WrongTokenType {
        /// Token type this code path requires (`"access"` or `"refresh"`).
        expected: &'static str,
        /// Token type actually carried in the `token_type` claim.
        actual: String,
    },

    /// Signing a new token failed.
    #[error("failed to sign token")]
    Signing(#[source] jsonwebtoken::errors::Error),

    /// The Redis whitelist/blacklist store could not be reached.
    #[error("auth store unavailable")]
    StoreUnavailable(#[from] redis::RedisError),
}
