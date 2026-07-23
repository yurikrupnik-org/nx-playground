//! Argon2 password hashing helpers.
//!
//! Argon2 hashing/verification takes tens of milliseconds by design, so both
//! operations run on the blocking thread pool instead of the async executor.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::error::{UserError, UserResult};

/// Hash a password with Argon2 on the blocking thread pool.
pub async fn hash_password(password: String) -> UserResult<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(UserError::PasswordHash)
    })
    .await
    .map_err(|e| UserError::Internal(format!("password hashing task failed: {e}")))?
}

/// Verify a password against an Argon2 hash on the blocking thread pool.
pub async fn verify_password(password: String, hash: String) -> UserResult<bool> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&hash).map_err(UserError::PasswordHash)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(|e| UserError::Internal(format!("password verification task failed: {e}")))?
}
