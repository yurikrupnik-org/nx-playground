use redis::aio::ConnectionManager;
use tracing::debug;

use crate::common::DatabaseError;

/// Check Redis health
///
/// Executes a `PING` command to verify the Redis connection is working.
/// This is useful for Kubernetes readiness and liveness probes.
///
/// # Arguments
/// * `conn` - Redis connection manager to check
///
/// # Returns
/// * `Ok(())` if Redis is healthy
/// * `Err(DatabaseError)` if the health check fails
///
/// # Example
/// ```ignore
/// use database::redis::{connect, check_health};
///
/// let conn = connect(&redis_url).await?;
///
/// // In your health endpoint
/// match check_health(&mut conn.clone()).await {
///     Ok(_) => HttpResponse::Ok().body("healthy"),
///     Err(e) => HttpResponse::ServiceUnavailable().body(format!("unhealthy: {}", e))
/// }
/// ```
pub async fn check_health(conn: &mut ConnectionManager) -> Result<(), DatabaseError> {
    debug!("Running Redis health check");

    // Execute PING command
    let response: String = redis::cmd("PING")
        .query_async(conn)
        .await
        .map_err(|e| DatabaseError::HealthCheckFailed(format!("Redis health check failed: {e}")))?;

    if response != "PONG" {
        return Err(DatabaseError::HealthCheckFailed(format!(
            "Redis PING returned unexpected response: {response}"
        )));
    }

    debug!("Redis health check passed");
    Ok(())
}

/// Check Redis health with a custom command
///
/// Allows you to specify a custom Redis command for health checking.
///
/// # Arguments
/// * `conn` - Redis connection manager to check
/// * `command` - Custom Redis command to execute
///
/// # Example
/// ```ignore
/// use database::redis::check_health_with_command;
///
/// // Check if a specific key exists
/// check_health_with_command(
///     &mut conn.clone(),
///     redis::cmd("EXISTS").arg("healthcheck_key")
/// ).await?;
/// ```
pub async fn check_health_with_command(
    conn: &mut ConnectionManager,
    command: &mut redis::Cmd,
) -> Result<(), DatabaseError> {
    debug!("Running Redis health check with custom command");

    command
        .query_async::<String>(conn)
        .await
        .map_err(|e| DatabaseError::HealthCheckFailed(format!("Redis health check failed: {e}")))?;

    debug!("Redis health check passed");
    Ok(())
}

/// Health check result for detailed status reporting
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// Whether Redis is healthy
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

/// Check Redis health with detailed status
///
/// Returns detailed health status including response time.
/// Useful for monitoring and observability.
///
/// # Example
/// ```ignore
/// use database::redis::check_health_detailed;
///
/// let status = check_health_detailed(&mut conn.clone()).await;
/// println!("Redis healthy: {}, response time: {}ms",
///     status.healthy,
///     status.response_time_ms
/// );
/// ```
pub async fn check_health_detailed(conn: &mut ConnectionManager) -> HealthStatus {
    let start = std::time::Instant::now();

    match check_health(conn).await {
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
        let status = HealthStatus::healthy(15);
        assert!(status.healthy);
        assert_eq!(status.response_time_ms, 15);
        assert!(status.message.is_none());
    }

    #[test]
    fn test_health_status_unhealthy() {
        let status = HealthStatus::unhealthy("connection timeout".to_string(), 5000);
        assert!(!status.healthy);
        assert_eq!(status.response_time_ms, 5000);
        assert_eq!(status.message, Some("connection timeout".to_string()));
    }

    // Note: Actual Redis health check tests require a running Redis instance
    // and should be integration tests, not unit tests
}
