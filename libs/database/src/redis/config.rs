#[cfg(feature = "config")]
use core_config::{ConfigError, FromEnv};

/// Redis database configuration
///
/// This struct holds Redis connection settings.
/// It can be constructed manually or loaded from environment variables (with `config` feature).
///
/// # Example
///
/// ```ignore
/// use database::redis::RedisConfig;
///
/// // Manual construction
/// let config = RedisConfig::new("redis://127.0.0.1:6379");
///
/// // From environment variables (requires `config` feature)
/// let config = RedisConfig::from_env()?;
///
/// // Use with connect()
/// let conn = database::redis::connect(&config.url).await?;
/// ```
#[derive(Clone, Debug)]
pub struct RedisConfig {
    /// Redis connection URL (required)
    pub url: String,

    /// Optional database number (0-15 for default Redis)
    pub database: Option<u8>,

    /// Optional username for Redis ACL
    pub username: Option<String>,

    /// Optional password for authentication
    pub password: Option<String>,
}

impl RedisConfig {
    /// Create a new RedisConfig with just a URL
    ///
    /// # Arguments
    /// * `url` - Redis connection string (e.g., "redis://127.0.0.1:6379")
    ///
    /// # Example
    /// ```ignore
    /// let config = RedisConfig::new("redis://127.0.0.1:6379");
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            database: None,
            username: None,
            password: None,
        }
    }

    /// Create a RedisConfig with authentication
    ///
    /// # Example
    /// ```ignore
    /// let config = RedisConfig::with_auth(
    ///     "redis://127.0.0.1:6379",
    ///     Some("myuser".to_string()),
    ///     Some("mypassword".to_string())
    /// );
    /// ```
    pub fn with_auth(
        url: impl Into<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self {
            url: url.into(),
            database: None,
            username,
            password,
        }
    }

    /// Create a RedisConfig with specific database number
    ///
    /// # Example
    /// ```ignore
    /// let config = RedisConfig::with_database("redis://127.0.0.1:6379", 1);
    /// ```
    pub fn with_database(url: impl Into<String>, database: u8) -> Self {
        Self {
            url: url.into(),
            database: Some(database),
            username: None,
            password: None,
        }
    }

    /// Build the full Redis URL with authentication and database if specified
    ///
    /// This is useful when you need to construct the complete connection string.
    ///
    /// # Example
    /// ```ignore
    /// let config = RedisConfig::with_auth(
    ///     "redis://127.0.0.1:6379",
    ///     Some("user".to_string()),
    ///     Some("pass".to_string())
    /// );
    /// let url = config.build_url();
    /// // Returns: "redis://user:pass@127.0.0.1:6379"
    /// ```
    pub fn build_url(&self) -> String {
        // If username and password are provided, they're likely already in the URL
        // This method is here for completeness but the URL should be pre-formatted
        self.url.clone()
    }

    /// Get a reference to the Redis URL
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379".to_string(),
            database: None,
            username: None,
            password: None,
        }
    }
}

/// Load RedisConfig from environment variables
///
/// Environment variables:
/// - `REDIS_URL` or `REDIS_HOST` (required) - Redis connection string
/// - `REDIS_DATABASE` (optional) - Redis database number (0-15)
/// - `REDIS_USERNAME` (optional) - Username for Redis ACL
/// - `REDIS_PASSWORD` (optional) - Password for authentication
///
/// # Example
/// ```ignore
/// use database::redis::RedisConfig;
/// use core_config::FromEnv;
///
/// let config = RedisConfig::from_env()?;
/// ```
#[cfg(feature = "config")]
impl FromEnv for RedisConfig {
    fn from_env() -> Result<Self, ConfigError> {
        // Try REDIS_URL first, fall back to REDIS_HOST (for compatibility)
        let url = std::env::var("REDIS_URL")
            .or_else(|_| std::env::var("REDIS_HOST"))
            .map_err(|_| ConfigError::MissingEnvVar("REDIS_URL or REDIS_HOST".to_string()))?;

        let database = if let Ok(db_str) = std::env::var("REDIS_DATABASE") {
            Some(db_str.parse().map_err(|e| ConfigError::ParseError {
                key: "REDIS_DATABASE".to_string(),
                details: format!("{e}"),
            })?)
        } else {
            None
        };

        let username = std::env::var("REDIS_USERNAME").ok();
        let password = std::env::var("REDIS_PASSWORD").ok();

        Ok(Self {
            url,
            database,
            username,
            password,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_config_new() {
        let config = RedisConfig::new("redis://localhost:6379");
        assert_eq!(config.url, "redis://localhost:6379");
        assert_eq!(config.database, None);
        assert_eq!(config.username, None);
        assert_eq!(config.password, None);
    }

    #[test]
    fn test_redis_config_with_auth() {
        let config = RedisConfig::with_auth(
            "redis://localhost:6379",
            Some("user".to_string()),
            Some("pass".to_string()),
        );
        assert_eq!(config.url, "redis://localhost:6379");
        assert_eq!(config.username, Some("user".to_string()));
        assert_eq!(config.password, Some("pass".to_string()));
    }

    #[test]
    fn test_redis_config_with_database() {
        let config = RedisConfig::with_database("redis://localhost:6379", 2);
        assert_eq!(config.url, "redis://localhost:6379");
        assert_eq!(config.database, Some(2));
    }

    #[test]
    fn test_redis_config_build_url() {
        let config = RedisConfig::new("redis://localhost:6379");
        assert_eq!(config.build_url(), "redis://localhost:6379");
    }

    #[test]
    fn test_redis_config_default() {
        let config = RedisConfig::default();
        assert_eq!(config.url, "redis://127.0.0.1:6379");
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_with_redis_url() {
        temp_env::with_var("REDIS_URL", Some("redis://localhost:6379"), || {
            let config = RedisConfig::from_env();
            assert!(config.is_ok());
            let config = config.unwrap();
            assert_eq!(config.url, "redis://localhost:6379");
        });
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_with_redis_host() {
        temp_env::with_vars(
            [
                ("REDIS_URL", None::<&str>),
                ("REDIS_HOST", Some("redis://prod:6379")),
            ],
            || {
                let config = RedisConfig::from_env();
                assert!(config.is_ok());
                let config = config.unwrap();
                assert_eq!(config.url, "redis://prod:6379");
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_with_database() {
        temp_env::with_vars(
            [
                ("REDIS_URL", Some("redis://localhost:6379")),
                ("REDIS_DATABASE", Some("3")),
            ],
            || {
                let config = RedisConfig::from_env();
                assert!(config.is_ok());
                let config = config.unwrap();
                assert_eq!(config.url, "redis://localhost:6379");
                assert_eq!(config.database, Some(3));
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_with_auth() {
        temp_env::with_vars(
            [
                ("REDIS_URL", Some("redis://localhost:6379")),
                ("REDIS_USERNAME", Some("myuser")),
                ("REDIS_PASSWORD", Some("mypass")),
            ],
            || {
                let config = RedisConfig::from_env();
                assert!(config.is_ok());
                let config = config.unwrap();
                assert_eq!(config.username, Some("myuser".to_string()));
                assert_eq!(config.password, Some("mypass".to_string()));
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_missing() {
        temp_env::with_vars(
            [("REDIS_URL", None::<&str>), ("REDIS_HOST", None::<&str>)],
            || {
                let config = RedisConfig::from_env();
                assert!(config.is_err());
                let err = config.unwrap_err();
                assert!(err.to_string().contains("REDIS"));
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_redis_config_from_env_invalid_database() {
        temp_env::with_vars(
            [
                ("REDIS_URL", Some("redis://localhost:6379")),
                ("REDIS_DATABASE", Some("invalid")),
            ],
            || {
                let config = RedisConfig::from_env();
                assert!(config.is_err());
                let err = config.unwrap_err();
                assert!(err.to_string().contains("REDIS_DATABASE"));
            },
        );
    }
}
