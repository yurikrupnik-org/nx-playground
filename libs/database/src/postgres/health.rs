use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use tracing::debug;

use crate::common::DatabaseError;

/// Check PostgreSQL database health
///
/// Executes a simple `SELECT 1` query to verify the database connection is working.
/// This is useful for Kubernetes readiness and liveness probes.
///
/// # Arguments
/// * `db` - Database connection to check
///
/// # Returns
/// * `Ok(())` if the database is healthy
/// * `Err(DatabaseError)` if the health check fails
///
/// # Example
/// ```ignore
/// use database::postgres::{connect, check_health};
///
/// let db = connect(&db_url).await?;
///
/// // In your health endpoint
/// match check_health(&db).await {
///     Ok(_) => HttpResponse::Ok().body("healthy"),
///     Err(e) => HttpResponse::ServiceUnavailable().body(format!("unhealthy: {}", e))
/// }
/// ```
pub async fn check_health(db: &DatabaseConnection) -> Result<(), DatabaseError> {
    debug!("Running PostgreSQL health check");

    // Execute a simple SELECT 1 query using raw SQL
    let stmt = Statement::from_string(DatabaseBackend::Postgres, "SELECT 1".to_owned());
    db.query_one_raw(stmt).await.map_err(|e| {
        DatabaseError::HealthCheckFailed(format!("PostgreSQL health check failed: {e}"))
    })?;

    debug!("PostgreSQL health check passed");
    Ok(())
}

/// Check PostgreSQL database health with custom query
///
/// Allows you to specify a custom query for health checking.
/// Useful when you want to verify specific database state.
///
/// # Arguments
/// * `db` - Database connection to check
/// * `query` - Custom SQL query to execute
///
/// # Example
/// ```ignore
/// use database::postgres::check_health_with_query;
///
/// // Check if a specific table exists
/// check_health_with_query(
///     &db,
///     "SELECT 1 FROM projects LIMIT 1"
/// ).await?;
/// ```
pub async fn check_health_with_query(
    db: &DatabaseConnection,
    query: &str,
) -> Result<(), DatabaseError> {
    debug!(
        "Running PostgreSQL health check with custom query: {}",
        query
    );

    let stmt = Statement::from_string(DatabaseBackend::Postgres, query.to_owned());
    db.query_one_raw(stmt).await.map_err(|e| {
        DatabaseError::HealthCheckFailed(format!(
            "PostgreSQL health check failed with query '{query}': {e}"
        ))
    })?;

    debug!("PostgreSQL health check passed");
    Ok(())
}

/// Health check result for detailed status reporting
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// Whether the database is healthy
    pub healthy: bool,

    /// Optional error message if unhealthy
    pub message: Option<String>,

    /// Response time in milliseconds
    pub response_time_ms: u64,
}

impl HealthStatus {
    /// Create a healthy status
    pub fn healthy(response_time_ms: u64) -> Self {
        Self {
            healthy: true,
            message: None,
            response_time_ms,
        }
    }

    /// Create an unhealthy status
    pub fn unhealthy(message: String, response_time_ms: u64) -> Self {
        Self {
            healthy: false,
            message: Some(message),
            response_time_ms,
        }
    }
}

/// Check PostgreSQL database health with detailed status
///
/// Returns detailed health status including response time.
/// Useful for monitoring and observability.
///
/// # Example
/// ```ignore
/// use database::postgres::check_health_detailed;
///
/// let status = check_health_detailed(&db).await;
/// println!("Database healthy: {}, response time: {}ms",
///     status.healthy,
///     status.response_time_ms
/// );
/// ```
pub async fn check_health_detailed(db: &DatabaseConnection) -> HealthStatus {
    let start = std::time::Instant::now();

    match check_health(db).await {
        Ok(_) => {
            let elapsed = start.elapsed().as_millis() as u64;
            HealthStatus::healthy(elapsed)
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as u64;
            HealthStatus::unhealthy(e.to_string(), elapsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_healthy() {
        let status = HealthStatus::healthy(42);
        assert!(status.healthy);
        assert_eq!(status.response_time_ms, 42);
        assert!(status.message.is_none());
    }

    #[test]
    fn test_health_status_unhealthy() {
        let status = HealthStatus::unhealthy("connection failed".to_string(), 100);
        assert!(!status.healthy);
        assert_eq!(status.response_time_ms, 100);
        assert_eq!(status.message, Some("connection failed".to_string()));
    }

    // Note: Actual database health check tests require a running database
    // and should be integration tests, not unit tests
}
