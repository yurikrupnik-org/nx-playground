//! Redis test infrastructure
//!
//! Provides a `TestRedis` helper that creates a Redis container for testing.

use redis::Client;
use redis::aio::MultiplexedConnection;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::redis::Redis;

/// Test Redis wrapper that ensures proper cleanup
///
/// The container is automatically stopped and removed when this struct is dropped.
///
/// # Example
///
/// ```no_run
/// use test_utils::TestRedis;
/// use redis::AsyncCommands;
///
/// # async fn example() {
/// let redis = TestRedis::new().await;
/// let mut conn = redis.connection();
///
/// // Use Redis in your tests
/// conn.set::<_, _, ()>("key", "value").await.unwrap();
/// let value: String = conn.get("key").await.unwrap();
/// assert_eq!(value, "value");
/// # }
/// ```
pub struct TestRedis {
    #[allow(dead_code)]
    container: ContainerAsync<Redis>,
    connection: MultiplexedConnection,
    pub connection_string: String,
}

impl TestRedis {
    /// Create a new test Redis instance
    ///
    /// Uses Redis 8 Alpine image by default.
    pub async fn new() -> Self {
        // Use Redis 8 Alpine (latest stable, lightweight)
        let redis_image = Redis::default().with_tag("8-alpine");

        let container = redis_image
            .start()
            .await
            .expect("Failed to start Redis container");

        let host_port = container
            .get_host_port_ipv4(6379)
            .await
            .expect("Failed to get Redis port");

        let connection_string = format!("redis://127.0.0.1:{host_port}");

        let client =
            Client::open(connection_string.clone()).expect("Failed to create Redis client");

        let connection = client
            .get_multiplexed_async_connection()
            .await
            .expect("Failed to connect to Redis");

        tracing::info!(port = host_port, "Test Redis ready (Redis 8-alpine)");

        Self {
            container,
            connection,
            connection_string,
        }
    }

    /// Get a cloned connection (useful for passing to services)
    ///
    /// # Example
    ///
    /// ```no_run
    /// use test_utils::TestRedis;
    ///
    /// # async fn example() {
    /// let redis = TestRedis::new().await;
    /// let conn = redis.connection();
    /// // Pass conn to your service/repository
    /// # }
    /// ```
    pub fn connection(&self) -> MultiplexedConnection {
        self.connection.clone()
    }

    /// Get the connection string for manual client creation
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
}

// Container is automatically cleaned up when TestRedis is dropped
impl Drop for TestRedis {
    fn drop(&mut self) {
        tracing::debug!("Cleaning up test Redis container");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::AsyncCommands;

    #[tokio::test]
    async fn test_redis_set_get() {
        let redis = TestRedis::new().await;
        let mut conn = redis.connection();

        // Set a value
        conn.set::<_, _, ()>("test_key", "test_value")
            .await
            .unwrap();

        // Get it back
        let value: String = conn.get("test_key").await.unwrap();
        assert_eq!(value, "test_value");
    }

    #[tokio::test]
    async fn test_redis_delete() {
        let redis = TestRedis::new().await;
        let mut conn = redis.connection();

        // Set and delete
        conn.set::<_, _, ()>("temp_key", "temp_value")
            .await
            .unwrap();
        conn.del::<_, ()>("temp_key").await.unwrap();

        // Should not exist
        let exists: bool = conn.exists("temp_key").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_redis_expiry() {
        let redis = TestRedis::new().await;
        let mut conn = redis.connection();

        // Set with expiry (1 second)
        conn.set_ex::<_, _, ()>("expiring_key", "value", 1)
            .await
            .unwrap();

        // Should exist immediately
        let exists: bool = conn.exists("expiring_key").await.unwrap();
        assert!(exists);

        // Wait for expiry
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Should not exist after expiry
        let exists: bool = conn.exists("expiring_key").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_redis_increment() {
        let redis = TestRedis::new().await;
        let mut conn = redis.connection();

        // Increment counter
        let count: i64 = conn.incr("counter", 1).await.unwrap();
        assert_eq!(count, 1);

        let count: i64 = conn.incr("counter", 5).await.unwrap();
        assert_eq!(count, 6);
    }

    #[tokio::test]
    async fn test_redis_list_operations() {
        let redis = TestRedis::new().await;
        let mut conn = redis.connection();

        // Push to list
        conn.rpush::<_, _, ()>("my_list", "item1").await.unwrap();
        conn.rpush::<_, _, ()>("my_list", "item2").await.unwrap();
        conn.rpush::<_, _, ()>("my_list", "item3").await.unwrap();

        // Get list length
        let len: usize = conn.llen("my_list").await.unwrap();
        assert_eq!(len, 3);

        // Pop from list
        let item: String = conn.lpop("my_list", None).await.unwrap();
        assert_eq!(item, "item1");
    }
}
