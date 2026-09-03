use sea_orm::ConnectOptions;
use std::time::Duration;
use tracing::log::LevelFilter;

#[cfg(feature = "config")]
use core_config::{ConfigError, FromEnv, env_or_default, env_required};

/// PostgreSQL database configuration
///
/// This struct holds all connection pool settings for PostgreSQL.
/// It can be constructed manually or loaded from environment variables (with `config` feature).
///
/// # Example
///
/// ```ignore
/// use database::postgres::PostgresConfig;
///
/// // Manual construction
/// let config = PostgresConfig::new("postgresql://user:pass@localhost/db");
///
/// // From environment variables (requires `config` feature)
/// let config = PostgresConfig::from_env()?;
///
/// // Convert to ConnectOptions for use with connect_with_options()
/// let options = config.into_connect_options();
/// ```
#[derive(Clone, Debug)]
pub struct PostgresConfig {
    /// Database connection URL (required)
    pub url: String,

    /// Maximum number of connections in the pool
    pub max_connections: u32,

    /// Minimum number of connections in the pool
    pub min_connections: u32,

    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,

    /// Connection acquire timeout in seconds
    pub acquire_timeout_secs: u64,

    /// Connection idle timeout in seconds
    pub idle_timeout_secs: u64,

    /// Connection max lifetime in seconds
    pub max_lifetime_secs: u64,

    /// Enable SQL query logging
    pub sqlx_logging: bool,

    /// SQL logging level
    pub sqlx_logging_level: LevelFilter,
}

impl PostgresConfig {
    /// Create a new PostgresConfig with default pool settings
    ///
    /// # Arguments
    /// * `url` - PostgreSQL connection string
    ///
    /// # Example
    /// ```ignore
    /// let config = PostgresConfig::new("postgresql://user:pass@localhost/db");
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 100,
            min_connections: 5,
            connect_timeout_secs: 8,
            acquire_timeout_secs: 8,
            idle_timeout_secs: 8,
            max_lifetime_secs: 8,
            sqlx_logging: true,
            sqlx_logging_level: LevelFilter::Info,
        }
    }

    /// Create a PostgresConfig with custom pool settings
    ///
    /// # Example
    /// ```ignore
    /// let config = PostgresConfig::with_pool_size(
    ///     "postgresql://user:pass@localhost/db",
    ///     50,  // max connections
    ///     10   // min connections
    /// );
    /// ```
    pub fn with_pool_size(
        url: impl Into<String>,
        max_connections: u32,
        min_connections: u32,
    ) -> Self {
        Self {
            url: url.into(),
            max_connections,
            min_connections,
            connect_timeout_secs: 8,
            acquire_timeout_secs: 8,
            idle_timeout_secs: 8,
            max_lifetime_secs: 8,
            sqlx_logging: true,
            sqlx_logging_level: LevelFilter::Info,
        }
    }

    /// Convert this config into SeaORM ConnectOptions
    ///
    /// This is useful when you need fine-grained control over connection options.
    ///
    /// # Example
    /// ```ignore
    /// use database::postgres::{PostgresConfig, connect_with_options};
    ///
    /// let config = PostgresConfig::new("postgresql://user:pass@localhost/db");
    /// let options = config.into_connect_options();
    /// let db = connect_with_options(options).await?;
    /// ```
    pub fn into_connect_options(self) -> ConnectOptions {
        let mut opt = ConnectOptions::new(&self.url);
        opt.max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs))
            .acquire_timeout(Duration::from_secs(self.acquire_timeout_secs))
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs))
            .max_lifetime(Duration::from_secs(self.max_lifetime_secs))
            .sqlx_logging(self.sqlx_logging)
            .sqlx_logging_level(self.sqlx_logging_level);
        opt
    }

    /// Get a reference to the database URL
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 100,
            min_connections: 5,
            connect_timeout_secs: 8,
            acquire_timeout_secs: 8,
            idle_timeout_secs: 8,
            max_lifetime_secs: 8,
            sqlx_logging: true,
            sqlx_logging_level: LevelFilter::Info,
        }
    }
}

/// Load PostgresConfig from environment variables
///
/// Environment variables:
/// - `DATABASE_URL` (required) - PostgreSQL connection string
/// - `DB_MAX_CONNECTIONS` (optional, default: 100)
/// - `DB_MIN_CONNECTIONS` (optional, default: 5)
/// - `DB_CONNECT_TIMEOUT_SECS` (optional, default: 8)
/// - `DB_ACQUIRE_TIMEOUT_SECS` (optional, default: 8)
/// - `DB_IDLE_TIMEOUT_SECS` (optional, default: 8)
/// - `DB_MAX_LIFETIME_SECS` (optional, default: 8)
/// - `DB_SQLX_LOGGING` (optional, default: true)
///
/// # Example
/// ```ignore
/// use database::postgres::PostgresConfig;
/// use core_config::FromEnv;
///
/// let config = PostgresConfig::from_env()?;
/// ```
#[cfg(feature = "config")]
impl FromEnv for PostgresConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let url = env_required("DATABASE_URL")?;

        let max_connections = env_or_default("DB_MAX_CONNECTIONS", "100")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_MAX_CONNECTIONS".to_string(),
                details: format!("{e}"),
            })?;

        let min_connections = env_or_default("DB_MIN_CONNECTIONS", "5")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_MIN_CONNECTIONS".to_string(),
                details: format!("{e}"),
            })?;

        let connect_timeout_secs = env_or_default("DB_CONNECT_TIMEOUT_SECS", "8")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_CONNECT_TIMEOUT_SECS".to_string(),
                details: format!("{e}"),
            })?;

        let acquire_timeout_secs = env_or_default("DB_ACQUIRE_TIMEOUT_SECS", "8")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_ACQUIRE_TIMEOUT_SECS".to_string(),
                details: format!("{e}"),
            })?;

        let idle_timeout_secs = env_or_default("DB_IDLE_TIMEOUT_SECS", "8")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_IDLE_TIMEOUT_SECS".to_string(),
                details: format!("{e}"),
            })?;

        let max_lifetime_secs = env_or_default("DB_MAX_LIFETIME_SECS", "8")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_MAX_LIFETIME_SECS".to_string(),
                details: format!("{e}"),
            })?;

        let sqlx_logging = env_or_default("DB_SQLX_LOGGING", "true")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "DB_SQLX_LOGGING".to_string(),
                details: format!("{e}"),
            })?;

        Ok(Self {
            url,
            max_connections,
            min_connections,
            connect_timeout_secs,
            acquire_timeout_secs,
            idle_timeout_secs,
            max_lifetime_secs,
            sqlx_logging,
            sqlx_logging_level: LevelFilter::Info,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgres_config_new() {
        let config = PostgresConfig::new("postgresql://localhost/test");
        assert_eq!(config.url, "postgresql://localhost/test");
        assert_eq!(config.max_connections, 100);
        assert_eq!(config.min_connections, 5);
    }

    #[test]
    fn test_postgres_config_with_pool_size() {
        let config = PostgresConfig::with_pool_size("postgresql://localhost/test", 50, 10);
        assert_eq!(config.url, "postgresql://localhost/test");
        assert_eq!(config.max_connections, 50);
        assert_eq!(config.min_connections, 10);
    }

    #[test]
    fn test_postgres_config_into_connect_options() {
        let config = PostgresConfig::new("postgresql://localhost/test");
        let _options = config.into_connect_options();
        // Can't easily assert on ConnectOptions internals, but verify it compiles
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_postgres_config_from_env_minimal() {
        // Unset optional vars to ensure defaults are used
        temp_env::with_vars(
            [
                ("DATABASE_URL", Some("postgresql://localhost/testdb")),
                ("DB_MAX_CONNECTIONS", None),
                ("DB_MIN_CONNECTIONS", None),
                ("DB_CONNECT_TIMEOUT_SECS", None),
                ("DB_IDLE_TIMEOUT_SECS", None),
                ("DB_ACQUIRE_TIMEOUT_SECS", None),
            ],
            || {
                let config = PostgresConfig::from_env();
                assert!(config.is_ok());
                let config = config.unwrap();
                assert_eq!(config.url, "postgresql://localhost/testdb");
                assert_eq!(config.max_connections, 100); // default
                assert_eq!(config.min_connections, 5); // default
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_postgres_config_from_env_custom() {
        temp_env::with_vars(
            [
                ("DATABASE_URL", Some("postgresql://localhost/testdb")),
                ("DB_MAX_CONNECTIONS", Some("50")),
                ("DB_MIN_CONNECTIONS", Some("10")),
                ("DB_CONNECT_TIMEOUT_SECS", Some("15")),
            ],
            || {
                let config = PostgresConfig::from_env();
                assert!(config.is_ok());
                let config = config.unwrap();
                assert_eq!(config.url, "postgresql://localhost/testdb");
                assert_eq!(config.max_connections, 50);
                assert_eq!(config.min_connections, 10);
                assert_eq!(config.connect_timeout_secs, 15);
            },
        );
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_postgres_config_from_env_missing_url() {
        temp_env::with_var_unset("DATABASE_URL", || {
            let config = PostgresConfig::from_env();
            assert!(config.is_err());
            let err = config.unwrap_err();
            assert!(err.to_string().contains("DATABASE_URL"));
        });
    }

    #[cfg(feature = "config")]
    #[test]
    fn test_postgres_config_from_env_invalid_number() {
        temp_env::with_vars(
            [
                ("DATABASE_URL", Some("postgresql://localhost/testdb")),
                ("DB_MAX_CONNECTIONS", Some("invalid")),
            ],
            || {
                let config = PostgresConfig::from_env();
                assert!(config.is_err());
                let err = config.unwrap_err();
                assert!(err.to_string().contains("DB_MAX_CONNECTIONS"));
            },
        );
    }
}
