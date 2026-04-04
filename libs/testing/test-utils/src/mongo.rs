//! MongoDB test infrastructure
//!
//! Provides a `TestMongo` helper that creates a MongoDB container for testing.

use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::mongo::Mongo;

/// Test MongoDB wrapper that ensures proper cleanup
pub struct TestMongo {
    #[allow(dead_code)]
    container: ContainerAsync<Mongo>,
    pub connection_string: String,
    pub database_name: String,
}

impl TestMongo {
    /// Create a new test MongoDB instance
    pub async fn new() -> Self {
        Self::with_database("test_db").await
    }

    /// Create a new test MongoDB instance with a specific database name
    pub async fn with_database(database_name: &str) -> Self {
        let container = Mongo::default()
            .start()
            .await
            .expect("Failed to start MongoDB container");

        let host_port = container
            .get_host_port_ipv4(27017)
            .await
            .expect("Failed to get host port");

        let connection_string = format!("mongodb://127.0.0.1:{}", host_port);

        tracing::info!(port = host_port, db = database_name, "Test MongoDB ready");

        Self {
            container,
            connection_string,
            database_name: database_name.to_string(),
        }
    }
}

impl Drop for TestMongo {
    fn drop(&mut self) {
        tracing::debug!("Cleaning up test MongoDB container");
    }
}
