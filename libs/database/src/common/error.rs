/// Unified database error type for all database operations
///
/// This provides a consistent error interface across PostgreSQL, Redis, and other databases.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    /// PostgreSQL-specific errors (SeaORM)
    #[cfg(feature = "postgres")]
    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] sea_orm::DbErr),

    /// Redis-specific errors
    #[cfg(feature = "redis")]
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// Mongo-specific errors
    #[cfg(feature = "mongo")]
    #[error("Mongodb error: {0}")]
    Mongo(String),

    /// Connection failed after retries
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Health check failed
    #[error("Health check failed: {0}")]
    HealthCheckFailed(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Migration error
    #[error("Migration error: {0}")]
    MigrationError(String),

    /// Generic database error
    #[error("Database error: {0}")]
    Generic(String),
}

/// Result type alias for database operations
pub type DatabaseResult<T> = Result<T, DatabaseError>;
