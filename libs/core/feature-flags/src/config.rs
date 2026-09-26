use core_config::{ConfigError, FromEnv, env_or_default, env_parse_or};
use std::env;
use std::time::Duration;

/// Connection settings for the Flagsmith API.
///
/// `environment_key` is the pivot of the whole crate: when it is `None` the
/// client is *unconfigured* and every lookup resolves from hardcoded defaults
/// without touching the network. That is the normal state of a fresh checkout,
/// not an error.
#[derive(Clone, Debug)]
pub struct FlagsmithConfig {
    /// Base API URL with no trailing slash, e.g. `http://localhost:8000/api/v1`.
    pub api_url: String,
    /// Flagsmith environment key; `None` disables remote evaluation entirely.
    pub environment_key: Option<String>,
    /// How long a resolved flag set (including a failed lookup) is reused.
    pub cache_ttl: Duration,
    /// Per-request timeout for the Flagsmith HTTP calls.
    pub request_timeout: Duration,
}

impl FlagsmithConfig {
    /// Default base URL, matching the local docker-compose Flagsmith.
    pub const DEFAULT_API_URL: &'static str = "http://localhost:8000/api/v1";
    /// Default cache lifetime in seconds.
    pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 15;
    /// Default request timeout in milliseconds.
    pub const DEFAULT_TIMEOUT_MS: u64 = 1500;

    /// Trim whitespace and trailing slashes so callers can build endpoints with
    /// a plain `format!("{api_url}/flags/")`. An empty value falls back to
    /// [`Self::DEFAULT_API_URL`].
    pub fn normalize_api_url(raw: &str) -> String {
        let trimmed = raw.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            Self::DEFAULT_API_URL.to_string()
        } else {
            trimmed.to_string()
        }
    }
}

impl Default for FlagsmithConfig {
    fn default() -> Self {
        Self {
            api_url: Self::DEFAULT_API_URL.to_string(),
            environment_key: None,
            cache_ttl: Duration::from_secs(Self::DEFAULT_CACHE_TTL_SECONDS),
            request_timeout: Duration::from_millis(Self::DEFAULT_TIMEOUT_MS),
        }
    }
}

impl FromEnv for FlagsmithConfig {
    /// Reads:
    /// - `FLAGSMITH_API_URL` — defaults to `http://localhost:8000/api/v1`
    /// - `FLAGSMITH_ENVIRONMENT_KEY` — unset *or empty* yields `None`, i.e.
    ///   defaults-only mode. `.env.example` ships a placeholder value, so an
    ///   operator blanking the line must be equivalent to removing it.
    /// - `FLAGSMITH_CACHE_TTL_SECONDS` — defaults to `15`
    /// - `FLAGSMITH_TIMEOUT_MS` — defaults to `1500`
    ///
    /// A present-but-unparseable TTL or timeout is a hard error.
    fn from_env() -> Result<Self, ConfigError> {
        let api_url =
            Self::normalize_api_url(&env_or_default("FLAGSMITH_API_URL", Self::DEFAULT_API_URL));
        let environment_key = env::var("FLAGSMITH_ENVIRONMENT_KEY")
            .ok()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        let cache_ttl = Duration::from_secs(env_parse_or(
            "FLAGSMITH_CACHE_TTL_SECONDS",
            Self::DEFAULT_CACHE_TTL_SECONDS,
        )?);
        let request_timeout = Duration::from_millis(env_parse_or(
            "FLAGSMITH_TIMEOUT_MS",
            Self::DEFAULT_TIMEOUT_MS,
        )?);

        Ok(Self {
            api_url,
            environment_key,
            cache_ttl,
            request_timeout,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::test_env;

    #[test]
    fn from_env_uses_defaults_when_nothing_is_set() {
        let _env = test_env::guard();
        temp_env::with_vars_unset(
            [
                "FLAGSMITH_API_URL",
                "FLAGSMITH_ENVIRONMENT_KEY",
                "FLAGSMITH_CACHE_TTL_SECONDS",
                "FLAGSMITH_TIMEOUT_MS",
            ],
            || {
                let config = FlagsmithConfig::from_env().unwrap();
                assert_eq!(config.api_url, "http://localhost:8000/api/v1");
                assert_eq!(config.environment_key, None);
                assert_eq!(config.cache_ttl, Duration::from_secs(15));
                assert_eq!(config.request_timeout, Duration::from_millis(1500));
            },
        );
    }

    #[test]
    fn from_env_reads_every_variable() {
        let _env = test_env::guard();
        temp_env::with_vars(
            [
                (
                    "FLAGSMITH_API_URL",
                    Some("https://flags.example.com/api/v1"),
                ),
                ("FLAGSMITH_ENVIRONMENT_KEY", Some("ser.abc123")),
                ("FLAGSMITH_CACHE_TTL_SECONDS", Some("60")),
                ("FLAGSMITH_TIMEOUT_MS", Some("250")),
            ],
            || {
                let config = FlagsmithConfig::from_env().unwrap();
                assert_eq!(config.api_url, "https://flags.example.com/api/v1");
                assert_eq!(config.environment_key.as_deref(), Some("ser.abc123"));
                assert_eq!(config.cache_ttl, Duration::from_secs(60));
                assert_eq!(config.request_timeout, Duration::from_millis(250));
            },
        );
    }

    #[test]
    fn from_env_treats_an_empty_environment_key_as_absent() {
        let _env = test_env::guard();
        temp_env::with_var("FLAGSMITH_ENVIRONMENT_KEY", Some("   "), || {
            let config = FlagsmithConfig::from_env().unwrap();
            assert_eq!(config.environment_key, None);
        });
        temp_env::with_var("FLAGSMITH_ENVIRONMENT_KEY", Some(""), || {
            let config = FlagsmithConfig::from_env().unwrap();
            assert_eq!(config.environment_key, None);
        });
    }

    #[test]
    fn from_env_trims_trailing_slashes_from_the_api_url() {
        let _env = test_env::guard();
        temp_env::with_var(
            "FLAGSMITH_API_URL",
            Some("http://localhost:8000/api/v1//"),
            || {
                let config = FlagsmithConfig::from_env().unwrap();
                assert_eq!(config.api_url, "http://localhost:8000/api/v1");
            },
        );
        temp_env::with_var("FLAGSMITH_API_URL", Some("  "), || {
            let config = FlagsmithConfig::from_env().unwrap();
            assert_eq!(config.api_url, FlagsmithConfig::DEFAULT_API_URL);
        });
    }

    #[test]
    fn from_env_rejects_an_unparseable_ttl() {
        let _env = test_env::guard();
        temp_env::with_var("FLAGSMITH_CACHE_TTL_SECONDS", Some("soon"), || {
            let err = FlagsmithConfig::from_env().unwrap_err();
            assert!(
                err.to_string().contains("FLAGSMITH_CACHE_TTL_SECONDS"),
                "unexpected error: {err}"
            );
        });
    }
}
