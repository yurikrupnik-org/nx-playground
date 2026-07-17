pub mod server;
pub mod tracing;

use std::env;
use std::fmt::Display;
use std::str::FromStr;
use thiserror::Error;

/// Application metadata from Cargo.toml (compile-time)
///
/// This struct holds app name and version from the consuming crate's Cargo.toml.
/// Use the `app_info!()` macro to create it, which ensures the values come from
/// the correct crate at compile time.
///
/// # Example
/// ```ignore
/// use core_config::{AppInfo, app_info};
///
/// let app = app_info!();
/// println!("Running {} v{}", app.name, app.version);
/// ```
#[derive(Clone, Debug)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// Create AppInfo from the consuming crate's Cargo.toml at compile time
///
/// This macro expands in the calling crate, so it reads CARGO_PKG_NAME and
/// CARGO_PKG_VERSION from the correct Cargo.toml (not from core_config).
///
/// # Example
/// ```ignore
/// use core_config::app_info;
///
/// let info = app_info!();
/// assert_eq!(info.name, "my_app");  // From my_app's Cargo.toml
/// ```
#[macro_export]
macro_rules! app_info {
    () => {
        $crate::AppInfo {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        }
    };
}

/// Configuration error type
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Environment variable '{0}' is required but not set")]
    MissingEnvVar(String),

    #[error("Failed to parse environment variable '{key}': {details}")]
    ParseError { key: String, details: String },
}

/// Application environment (dev = local/kind, prod = full k8s)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Environment {
    Development, // Local dev or kind cluster (no HTTPS)
    Production,  // Full k8s cluster (with HTTPS)
}

impl Environment {
    pub fn from_env() -> Self {
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

        if app_env.eq_ignore_ascii_case("production") {
            Environment::Production
        } else {
            Environment::Development
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }

    pub fn is_development(&self) -> bool {
        matches!(self, Environment::Development)
    }

    // Whether HTTPS features should be enabled
    pub fn use_https(&self) -> bool {
        self.is_production()
    }
}

/// Trait for configuration that can be loaded from environment variables
pub trait FromEnv: Sized {
    fn from_env() -> Result<Self, ConfigError>;
}

/// Helper to load and parse environment variable with a default value
pub fn env_or_default(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Helper to load and parse environment variable or return error
pub fn env_required(key: &str) -> Result<String, ConfigError> {
    env::var(key).map_err(|_| ConfigError::MissingEnvVar(key.to_string()))
}

/// Load an environment variable and parse it into `T`, falling back to
/// `default` when the variable is unset.
///
/// A present-but-unparseable value is a hard [`ConfigError::ParseError`] that
/// names both the key and the offending value, so misconfiguration fails fast
/// with a message that is useful in logs rather than silently defaulting.
pub fn env_parse_or<T>(key: &str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: Display,
{
    match env::var(key) {
        Ok(v) => v.parse().map_err(|e: T::Err| ConfigError::ParseError {
            key: key.to_string(),
            details: format!("invalid value {v:?}: {e}"),
        }),
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_defaults_to_development() {
        temp_env::with_var_unset("APP_ENV", || {
            let env = Environment::from_env();
            assert_eq!(env, Environment::Development);
            assert!(env.is_development());
            assert!(!env.is_production());
            assert!(!env.use_https());
        });
    }

    #[test]
    fn test_environment_production() {
        temp_env::with_var("APP_ENV", Some("production"), || {
            let env = Environment::from_env();
            assert_eq!(env, Environment::Production);
            assert!(env.is_production());
            assert!(!env.is_development());
            assert!(env.use_https());
        });
    }

    #[test]
    fn test_environment_production_case_insensitive() {
        temp_env::with_var("APP_ENV", Some("PRODUCTION"), || {
            let env = Environment::from_env();
            assert_eq!(env, Environment::Production);
        });

        temp_env::with_var("APP_ENV", Some("Production"), || {
            let env = Environment::from_env();
            assert_eq!(env, Environment::Production);
        });
    }

    #[test]
    fn test_environment_unknown_defaults_to_development() {
        temp_env::with_var("APP_ENV", Some("staging"), || {
            let env = Environment::from_env();
            assert_eq!(env, Environment::Development);
        });
    }

    #[test]
    fn test_env_or_default_with_value() {
        temp_env::with_var("TEST_VAR", Some("test_value"), || {
            let result = env_or_default("TEST_VAR", "default");
            assert_eq!(result, "test_value");
        });
    }

    #[test]
    fn test_env_or_default_without_value() {
        temp_env::with_var_unset("MISSING_VAR", || {
            let result = env_or_default("MISSING_VAR", "default_value");
            assert_eq!(result, "default_value");
        });
    }

    #[test]
    fn test_env_required_success() {
        temp_env::with_var("REQUIRED_VAR", Some("required_value"), || {
            let result = env_required("REQUIRED_VAR");
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "required_value");
        });
    }

    #[test]
    fn test_env_required_missing() {
        temp_env::with_var_unset("MISSING_REQUIRED", || {
            let result = env_required("MISSING_REQUIRED");
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("MISSING_REQUIRED"));
            assert!(err.to_string().contains("required"));
        });
    }
}
