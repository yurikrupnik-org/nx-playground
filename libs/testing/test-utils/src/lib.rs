//! Shared test utilities for domain testing
//!
//! This crate provides reusable test infrastructure for all domain crates:
//! - `TestDatabase`: PostgreSQL container with automatic cleanup (feature: "postgres")
//! - `TestRedis`: Redis container with automatic cleanup (feature: "redis")
//! - `TestNats`: NATS container with JetStream for stream testing (feature: "nats")
//! - `TestDataBuilder`: Deterministic test data generation (always available)
//! - `assertions`: Custom assertion helpers (always available)
//!
//! # Features
//!
//! - `postgres` (default): Enables PostgreSQL test infrastructure
//! - `redis`: Enables Redis test infrastructure
//! - `nats`: Enables NATS JetStream test infrastructure
//! - `all`: Enables all test infrastructure
//!
//! # Usage
//!
//! ## PostgreSQL Testing
//!
//! ```rust,no_run
//! use test_utils::{TestDatabase, TestDataBuilder};
//!
//! #[tokio::test]
//! async fn my_postgres_test() {
//!     let db = TestDatabase::new().await;
//!     let builder = TestDataBuilder::from_test_name("my_test");
//!
//!     let user_id = builder.user_id();
//!     let resource_name = builder.name("resource", "main");
//! }
//! ```
//!
//! ## Redis Testing
//!
//! Add `features = ["redis"]` to your dev-dependencies:
//!
//! ```toml
//! [dev-dependencies]
//! test-utils = { workspace = true, features = ["redis"] }
//! ```
//!
//! Then in your tests:
//!
//! ```rust,ignore
//! use test_utils::TestRedis;
//! use redis::AsyncCommands;
//!
//! #[tokio::test]
//! async fn my_redis_test() {
//!     let redis = TestRedis::new().await;
//!     let mut conn = redis.connection();
//!
//!     conn.set::<_, _, ()>("key", "value").await.unwrap();
//!     let value: String = conn.get("key").await.unwrap();
//!     assert_eq!(value, "value");
//! }
//! ```
//!
//! ## NATS JetStream Testing
//!
//! Add `features = ["nats"]` to your dev-dependencies:
//!
//! ```toml
//! [dev-dependencies]
//! test-utils = { workspace = true, features = ["nats"] }
//! ```
//!
//! Then in your tests:
//!
//! ```rust,ignore
//! use test_utils::TestNats;
//!
//! #[tokio::test]
//! async fn my_nats_test() {
//!     let nats = TestNats::new().await;
//!     let jetstream = nats.jetstream();
//!
//!     // Create streams, publish messages, etc.
//! }
//! ```

use uuid::Uuid;

// Conditionally compile database modules based on features
#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "mongo")]
mod mongo;

#[cfg(feature = "redis")]
mod redis;

#[cfg(feature = "nats")]
mod nats;

// Re-export based on enabled features
#[cfg(feature = "postgres")]
pub use postgres::TestDatabase;

#[cfg(feature = "mongo")]
pub use mongo::TestMongo;

#[cfg(feature = "redis")]
pub use redis::TestRedis;

#[cfg(feature = "nats")]
pub use nats::TestNats;

/// Builder for test data with deterministic randomization
///
/// This ensures tests are reproducible by using seeded random data.
pub struct TestDataBuilder {
    seed: u64,
}

impl TestDataBuilder {
    /// Create a new builder with a seed (for deterministic tests)
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Create from test name (generates seed from test name hash)
    ///
    /// This is the recommended way to create a builder for consistent test data.
    ///
    /// # Example
    ///
    /// ```
    /// use test_utils::TestDataBuilder;
    ///
    /// let builder = TestDataBuilder::from_test_name("test_create_resource");
    /// ```
    pub fn from_test_name(name: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        Self::new(hasher.finish())
    }

    /// Generate a unique user ID for testing
    pub fn user_id(&self) -> Uuid {
        // Use seed to generate deterministic UUID
        let bytes = self.seed.to_le_bytes();
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes[..8].copy_from_slice(&bytes);
        uuid_bytes[8..16].copy_from_slice(&bytes);
        Uuid::from_bytes(uuid_bytes)
    }

    /// Generate a unique name for testing
    ///
    /// # Arguments
    ///
    /// * `prefix` - The type of resource (e.g., "project", "cloud-resource")
    /// * `suffix` - A unique identifier within the test (e.g., "main", "backup")
    ///
    /// # Example
    ///
    /// ```
    /// use test_utils::TestDataBuilder;
    ///
    /// let builder = TestDataBuilder::from_test_name("my_test");
    /// let name = builder.name("project", "main");
    /// // Returns: "test-project-12345-main"
    /// ```
    pub fn name(&self, prefix: &str, suffix: &str) -> String {
        format!("test-{}-{}-{}", prefix, self.seed, suffix)
    }
}

/// Test assertion helpers
pub mod assertions {
    use uuid::Uuid;

    /// Assert that two UUIDs are equal with a nice error message
    pub fn assert_uuid_eq(actual: Uuid, expected: Uuid, context: &str) {
        assert_eq!(
            actual, expected,
            "{}: expected UUID {}, got {}",
            context, expected, actual
        );
    }

    /// Assert that an optional value is Some
    pub fn assert_some<T>(value: Option<T>, context: &str) -> T {
        value.unwrap_or_else(|| panic!("{}: expected Some, got None", context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_builder_deterministic() {
        let builder1 = TestDataBuilder::new(42);
        let builder2 = TestDataBuilder::new(42);

        assert_eq!(builder1.user_id(), builder2.user_id());
        assert_eq!(
            builder1.name("project", "test"),
            builder2.name("project", "test")
        );
    }

    #[test]
    fn test_data_builder_from_name() {
        let builder1 = TestDataBuilder::from_test_name("my_test");
        let builder2 = TestDataBuilder::from_test_name("my_test");

        assert_eq!(builder1.user_id(), builder2.user_id());
    }

    #[test]
    fn test_data_builder_different_names() {
        let builder1 = TestDataBuilder::from_test_name("test1");
        let builder2 = TestDataBuilder::from_test_name("test2");

        // Different test names should generate different data
        assert_ne!(builder1.user_id(), builder2.user_id());
    }
}
