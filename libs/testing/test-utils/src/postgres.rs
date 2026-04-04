//! PostgreSQL test infrastructure
//!
//! Provides a `TestDatabase` helper that creates a PostgreSQL container for testing.
//! Uses sqlx to run migrations from the manifests/migrations directory.

use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use std::path::PathBuf;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Test database wrapper that ensures proper cleanup
///
/// The container is automatically stopped and removed when this struct is dropped.
pub struct TestDatabase {
    #[allow(dead_code)]
    container: ContainerAsync<Postgres>,
    pub connection: DatabaseConnection,
    pub connection_string: String,
}

impl TestDatabase {
    /// Create a new test database with migrations applied
    ///
    /// # Example
    ///
    /// ```no_run
    /// use test_utils::TestDatabase;
    ///
    /// # async fn example() {
    /// let db = TestDatabase::new().await;
    /// // Use db.connection() to create your repository
    /// # }
    /// ```
    pub async fn new() -> Self {
        // Use Postgres 18 to match production
        let postgres = Postgres::default().with_tag("18-alpine");

        let container = postgres
            .start()
            .await
            .expect("Failed to start Postgres container");

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");

        let connection_string = format!(
            "postgres://postgres:postgres@127.0.0.1:{}/postgres",
            host_port
        );

        // Connect to database
        let connection = Database::connect(&connection_string)
            .await
            .expect("Failed to connect to test database");

        // Run migrations using sqlx
        Self::run_migrations(&connection).await;

        tracing::info!(port = host_port, "Test database ready (Postgres 18)");

        Self {
            container,
            connection,
            connection_string,
        }
    }

    /// Find the workspace root by looking for Cargo.toml with [workspace]
    fn find_workspace_root() -> PathBuf {
        let mut current = std::env::current_dir().expect("Failed to get current directory");

        loop {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                let content =
                    std::fs::read_to_string(&cargo_toml).expect("Failed to read Cargo.toml");
                if content.contains("[workspace]") {
                    return current;
                }
            }

            if !current.pop() {
                // Fallback to CARGO_MANIFEST_DIR parent chain
                let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                return manifest_dir
                    .ancestors()
                    .find(|p| {
                        p.join("Cargo.toml").exists() && {
                            std::fs::read_to_string(p.join("Cargo.toml"))
                                .map(|c| c.contains("[workspace]"))
                                .unwrap_or(false)
                        }
                    })
                    .unwrap_or(&manifest_dir)
                    .to_path_buf();
            }
        }
    }

    /// Run migrations from SQL files in manifests/db/migrations/
    async fn run_migrations(connection: &DatabaseConnection) {
        // Find workspace root by looking for Cargo.toml with [workspace]
        let workspace_root = Self::find_workspace_root();
        let migrations_dir = workspace_root.join("manifests/db/migrations");

        if !migrations_dir.exists() {
            tracing::warn!(
                "Migrations directory not found: {:?}. Run 'just migrate-diff mydatabase initial' first.",
                migrations_dir
            );
            return;
        }

        // Read and sort migration files
        let mut migrations: Vec<_> = std::fs::read_dir(migrations_dir)
            .expect("Failed to read migrations directory")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "sql")
                    .unwrap_or(false)
            })
            .collect();

        migrations.sort_by_key(|e| e.path());

        // Execute each migration
        for entry in migrations {
            let path = entry.path();
            let sql = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Failed to read migration: {:?}", path));

            tracing::debug!("Running migration: {:?}", path.file_name());

            // Split by semicolons, but respect dollar-quoted strings ($$...$$)
            let statements = Self::split_sql_statements(&sql);

            for statement in statements.iter() {
                let statement = statement.trim();
                // Skip empty statements and pure comment blocks
                let is_comment_only = statement.lines().all(|line| {
                    let trimmed = line.trim();
                    trimmed.is_empty() || trimmed.starts_with("--")
                });
                if !statement.is_empty() && !is_comment_only {
                    if let Err(e) = connection.execute_unprepared(statement).await {
                        // Log but don't fail for certain expected errors
                        if !e.to_string().contains("already exists") {
                            tracing::warn!("Migration statement failed: {}", e);
                        }
                    }
                }
            }
        }

        tracing::info!("Migrations complete");
    }

    /// Split SQL into statements, respecting dollar-quoted strings
    fn split_sql_statements(sql: &str) -> Vec<String> {
        let mut statements = Vec::new();
        let mut current = String::new();
        let mut in_dollar_quote = false;
        let mut chars = sql.chars().peekable();

        while let Some(c) = chars.next() {
            current.push(c);

            // Check for $$ (dollar quote)
            if c == '$' && chars.peek() == Some(&'$') {
                chars.next(); // consume second $
                current.push('$');
                in_dollar_quote = !in_dollar_quote;
            } else if c == ';' && !in_dollar_quote {
                // End of statement
                let stmt = current.trim().to_string();
                if !stmt.is_empty() {
                    statements.push(stmt);
                }
                current = String::new();
            }
        }

        // Add any remaining content
        let stmt = current.trim().to_string();
        if !stmt.is_empty() {
            statements.push(stmt);
        }

        statements
    }

    /// Create a test database with a specific schema (for parallel test isolation)
    ///
    /// # Example
    ///
    /// ```no_run
    /// use test_utils::TestDatabase;
    ///
    /// # async fn example() {
    /// let db = TestDatabase::with_schema("test_create_project").await;
    /// # }
    /// ```
    pub async fn with_schema(schema_name: &str) -> Self {
        let db = Self::new().await;

        // Create schema for isolation
        let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name);
        db.connection
            .execute_unprepared(&create_schema)
            .await
            .expect("Failed to create schema");

        // Set search path to use this schema
        let set_path = format!("SET search_path TO {}", schema_name);
        db.connection
            .execute_unprepared(&set_path)
            .await
            .expect("Failed to set search path");

        // Run migrations in this schema
        Self::run_migrations(&db.connection).await;

        db
    }

    /// Get a cloned connection (useful for passing to repositories)
    pub fn connection(&self) -> DatabaseConnection {
        self.connection.clone()
    }

    /// Create a test user and return their UUID
    ///
    /// This is useful for tests that need to create entities with foreign key
    /// references to the users table.
    pub async fn create_test_user(&self, user_id: uuid::Uuid) -> uuid::Uuid {
        let query = format!(
            "INSERT INTO users (id, email, name, password_hash) VALUES ('{}', 'test-{}@example.com', 'Test User {}', '$argon2id$v=19$m=19456,t=2,p=1$test$test') ON CONFLICT (id) DO NOTHING",
            user_id, user_id, user_id
        );
        self.connection
            .execute_unprepared(&query)
            .await
            .expect("Failed to create test user");
        user_id
    }
}

// Container is automatically cleaned up when TestDatabase is dropped
impl Drop for TestDatabase {
    fn drop(&mut self) {
        tracing::debug!("Cleaning up test database container");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_creation() {
        let db = TestDatabase::new().await;
        assert!(db.connection_string.contains("postgres://"));
    }

    #[tokio::test]
    async fn test_schema_isolation() {
        let db1 = TestDatabase::with_schema("schema1").await;
        let db2 = TestDatabase::with_schema("schema2").await;

        // Both databases should be functional
        assert!(db1.connection_string.contains("postgres://"));
        assert!(db2.connection_string.contains("postgres://"));
    }
}
