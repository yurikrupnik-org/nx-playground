use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use std::time::Duration;
use tracing::{info, log::LevelFilter};

use super::PostgresConfig;
use crate::common::{RetryConfig, retry, retry_with_backoff};

/// Connect to a PostgreSQL database with optimized connection pool settings
///
/// # Arguments
/// * `database_url` - PostgreSQL connection string
///
/// # Example
/// ```ignore
/// use database::postgres::connect;
///
/// let db = connect("postgresql://user:pass@localhost/db").await?;
/// ```
pub async fn connect(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut opt = ConnectOptions::new(database_url);
    opt.max_connections(100)
        .min_connections(5)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .max_lifetime(Duration::from_secs(8))
        .sqlx_logging(true)
        .sqlx_logging_level(LevelFilter::Info); // SeaORM requires log::LevelFilter

    let db = Database::connect(opt).await?;

    info!("Successfully connected to PostgreSQL database");

    Ok(db)
}

/// Connect using a PostgresConfig
///
/// This is the recommended way to connect when using configuration.
///
/// # Example
/// ```ignore
/// use database::postgres::{PostgresConfig, connect_from_config};
///
/// let config = PostgresConfig::new("postgresql://user:pass@localhost/db");
/// let db = connect_from_config(config).await?;
/// ```
///
/// With FromEnv (requires `config` feature):
/// ```ignore
/// use database::postgres::connect_from_config;
/// use core_config::FromEnv;
///
/// let config = PostgresConfig::from_env()?;
/// let db = connect_from_config(config).await?;
/// ```
pub async fn connect_from_config(config: PostgresConfig) -> Result<DatabaseConnection, DbErr> {
    let options = config.into_connect_options();
    connect_with_options(options).await
}

/// Connect with custom connection options
///
/// Use this when you need fine-grained control over connection pool settings.
///
/// # Example
/// ```ignore
/// use sea_orm::ConnectOptions;
/// use database::postgres::connect_with_options;
/// use std::time::Duration;
///
/// let mut opt = ConnectOptions::new("postgresql://user:pass@localhost/db");
/// opt.max_connections(50)
///     .connect_timeout(Duration::from_secs(10));
///
/// let db = connect_with_options(opt).await?;
/// ```
pub async fn connect_with_options(options: ConnectOptions) -> Result<DatabaseConnection, DbErr> {
    let db = Database::connect(options).await?;
    info!("Successfully connected to PostgreSQL database with custom options");
    Ok(db)
}

/// Connect to PostgreSQL with automatic retry on failure
///
/// Uses exponential backoff with jitter to retry connection attempts.
/// Useful for handling transient network issues during startup.
///
/// # Example
/// ```ignore
/// use database::postgres::connect_with_retry;
/// use database::common::RetryConfig;
///
/// // Default retry: 3 attempts, 100ms initial delay
/// let db = connect_with_retry("postgresql://user:pass@localhost/db", None).await?;
///
/// // Custom retry: 5 attempts, 500ms initial delay
/// let config = RetryConfig::new()
///     .with_max_retries(5)
///     .with_initial_delay(500);
/// let db = connect_with_retry("postgresql://user:pass@localhost/db", Some(config)).await?;
/// ```
pub async fn connect_with_retry(
    database_url: &str,
    retry_config: Option<RetryConfig>,
) -> Result<DatabaseConnection, DbErr> {
    let url = database_url.to_string();

    match retry_config {
        Some(config) => retry_with_backoff(|| connect(&url), config).await,
        None => retry(|| connect(&url)).await,
    }
}

/// Connect from config with automatic retry on failure
///
/// # Example
/// ```ignore
/// use database::postgres::{PostgresConfig, connect_from_config_with_retry};
/// use database::common::RetryConfig;
///
/// let config = PostgresConfig::from_env()?;
/// let retry_config = RetryConfig::new().with_max_retries(5);
/// let db = connect_from_config_with_retry(config, Some(retry_config)).await?;
/// ```
pub async fn connect_from_config_with_retry(
    config: PostgresConfig,
    retry_config: Option<RetryConfig>,
) -> Result<DatabaseConnection, DbErr> {
    let options = config.into_connect_options();

    match retry_config {
        Some(retry) => {
            retry_with_backoff(
                || {
                    let opts = options.clone();
                    connect_with_options(opts)
                },
                retry,
            )
            .await
        }
        None => {
            retry(|| {
                let opts = options.clone();
                connect_with_options(opts)
            })
            .await
        }
    }
}

// Note: Migrations are now managed by Atlas CLI
// See manifests/db/migrations/ for SQL migration files
// Run `just migrate` to apply migrations

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires actual database
    async fn test_connect() {
        let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5432/test_db".to_string()
        });

        let result = connect(&db_url).await;
        assert!(result.is_ok());
    }
}
