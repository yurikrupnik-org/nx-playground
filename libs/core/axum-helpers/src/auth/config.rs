//! Configuration types for axum-helpers.
//!
//! This module provides configuration structs that implement the `FromEnv` trait
//! from `core_config`, following the same pattern as `PostgresConfig` and `RedisConfig`.

use core_config::{ConfigError, FromEnv, env_required};

/// JWT authentication configuration.
///
/// Loaded from environment variables:
/// - `JWT_SECRET` (required) - Must be at least 32 characters for security
///
/// # Example
///
/// ```ignore
/// use axum_helpers::JwtConfig;
/// use core_config::FromEnv;
///
/// // From environment variables
/// let config = JwtConfig::from_env()?;
///
/// // Manual construction (for testing)
/// let config = JwtConfig::new("my-super-secret-key-that-is-at-least-32-chars");
/// ```
#[derive(Clone, Debug)]
pub struct JwtConfig {
    /// JWT signing secret (minimum 32 characters)
    pub secret: String,
}

impl JwtConfig {
    /// Create a new JwtConfig with the given secret.
    ///
    /// # Panics
    /// Panics if the secret is less than 32 characters.
    pub fn new(secret: impl Into<String>) -> Self {
        let secret = secret.into();
        assert!(
            secret.len() >= 32,
            "JWT secret must be at least 32 characters"
        );
        Self { secret }
    }
}

impl FromEnv for JwtConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let secret = env_required("JWT_SECRET")?;

        if secret.len() < 32 {
            return Err(ConfigError::ParseError {
                key: "JWT_SECRET".to_string(),
                details: format!(
                    "must be at least 32 characters for security (got {}). Generate one with: openssl rand -base64 32",
                    secret.len()
                ),
            });
        }

        Ok(Self { secret })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_jwt_config_new_valid() {
        let secret = "this-is-a-valid-secret-with-32-chars!";
        let config = JwtConfig::new(secret);
        assert_eq!(config.secret, secret);
    }

    #[test]
    #[should_panic(expected = "JWT secret must be at least 32 characters")]
    fn test_jwt_config_new_too_short() {
        JwtConfig::new("short");
    }

    #[test]
    fn test_jwt_config_from_env_valid() {
        temp_env::with_var(
            "JWT_SECRET",
            Some("this-is-a-valid-secret-with-32-chars!"),
            || {
                let config = JwtConfig::from_env();
                assert!(config.is_ok());
                assert_eq!(
                    config.unwrap().secret,
                    "this-is-a-valid-secret-with-32-chars!"
                );
            },
        );
    }

    #[test]
    fn test_jwt_config_from_env_missing() {
        temp_env::with_var_unset("JWT_SECRET", || {
            let config = JwtConfig::from_env();
            assert!(config.is_err());
            let err = config.unwrap_err();
            assert!(err.to_string().contains("JWT_SECRET"));
        });
    }

    #[test]
    fn test_jwt_config_from_env_too_short() {
        temp_env::with_var("JWT_SECRET", Some("short"), || {
            let config = JwtConfig::from_env();
            assert!(config.is_err());
            let err = config.unwrap_err();
            assert!(err.to_string().contains("32 characters"));
        });
    }
}
