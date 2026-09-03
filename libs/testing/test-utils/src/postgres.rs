//! PostgreSQL test infrastructure
//!
//! Provides a `TestDatabase` helper that creates a PostgreSQL container for testing.
//! Applies either a declarative `schema.sql` (default: `manifests/db/zerg/schema.sql`)
//! or an ordered `migrations/` directory, via [`TestDatabase::with_migrations_dir`].

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
        Self::with_migrations_dir("manifests/db/zerg/schema.sql").await
    }

    /// Create a test database applying SQL migrations from `rel_dir`
    /// (relative to the workspace root). Lets non-zerg verticals (e.g. todo)
    /// own their schema instead of the hardcoded zerg path.
    pub async fn with_migrations_dir(rel_dir: &str) -> Self {
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

        let connection_string =
            format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

        // Connect to database
        let connection = Database::connect(&connection_string)
            .await
            .expect("Failed to connect to test database");

        // Run migrations using sqlx
        Self::run_migrations_from(&connection, rel_dir).await;

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

    /// Apply a SQL source to the test database.
    ///
    /// `rel_path` may be either a **directory** of ordered `*.sql` migrations
    /// (versioned databases such as `todo`/`terran`) or a **single `schema.sql`
    /// file** (declarative databases such as `zerg`, which has no migrations
    /// directory - Atlas reconciles the desired state instead). See
    /// `manifests/db/README.md` for the two modes.
    async fn run_migrations_from(connection: &DatabaseConnection, rel_path: &str) {
        // Find workspace root by looking for Cargo.toml with [workspace]
        let workspace_root = Self::find_workspace_root();
        let source = workspace_root.join(rel_path);

        assert!(
            source.exists(),
            "SQL source not found: {source:?}. Expected either a migrations \
             directory or a declarative schema.sql."
        );

        // Collect the files to apply, in order.
        let mut migrations: Vec<std::path::PathBuf> = if source.is_dir() {
            let mut files: Vec<_> = std::fs::read_dir(&source)
                .expect("Failed to read migrations directory")
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|ext| ext == "sql").unwrap_or(false))
                .collect();
            files.sort();
            files
        } else {
            vec![source]
        };

        migrations.sort();

        // Execute each migration
        for path in migrations {
            let sql = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Failed to read SQL source: {path:?}"));

            tracing::debug!("Running migration: {:?}", path.file_name());

            // Split on top-level semicolons (comments/strings/$$ blocks respected).
            let statements = Self::split_sql_statements(&sql);

            for statement in statements.iter() {
                let statement = statement.trim();
                // Skip empty statements and pure comment blocks
                let is_comment_only = statement.lines().all(|line| {
                    let trimmed = line.trim();
                    trimmed.is_empty() || trimmed.starts_with("--")
                });
                if !statement.is_empty()
                    && !is_comment_only
                    && let Err(e) = connection.execute_unprepared(statement).await
                {
                    // `already exists` is expected when layering into a pre-created
                    // schema (`with_schema`). Anything else means the harness built a
                    // database the tests will misread - fail now, loudly. A warning
                    // here is invisible: tests install no tracing subscriber.
                    assert!(
                        e.to_string().contains("already exists"),
                        "Failed to apply {:?}\n  statement: {}\n  error: {e}",
                        path.file_name().unwrap_or(path.as_os_str()),
                        statement.lines().next().unwrap_or(statement),
                    );
                }
            }
        }

        tracing::info!("Schema ready");
    }

    /// Split SQL into statements on top-level semicolons.
    ///
    /// A `;` only terminates a statement when it is not inside a `--` line comment,
    /// a single-quoted literal, or a `$$`-quoted block. Getting this wrong is silent
    /// and destructive: a comment such as `-- mirrors the IdP; see docs` would split
    /// mid-comment and glue its tail onto the next `CREATE TABLE`, which then fails.
    fn split_sql_statements(sql: &str) -> Vec<String> {
        let mut statements = Vec::new();
        let mut current = String::new();
        let mut in_dollar_quote = false;
        let mut in_line_comment = false;
        let mut in_string = false;
        let mut chars = sql.chars().peekable();

        while let Some(c) = chars.next() {
            current.push(c);

            if in_line_comment {
                if c == '\n' {
                    in_line_comment = false;
                }
                continue;
            }

            if in_string {
                // '' is an escaped quote, not a terminator.
                if c == '\'' {
                    if chars.peek() == Some(&'\'') {
                        current.push(chars.next().expect("peeked"));
                    } else {
                        in_string = false;
                    }
                }
                continue;
            }

            if c == '$' && chars.peek() == Some(&'$') {
                chars.next(); // consume second $
                current.push('$');
                in_dollar_quote = !in_dollar_quote;
            } else if in_dollar_quote {
                continue;
            } else if c == '-' && chars.peek() == Some(&'-') {
                current.push(chars.next().expect("peeked"));
                in_line_comment = true;
            } else if c == '\'' {
                in_string = true;
            } else if c == ';' {
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

    /// Create a test database whose schema lives under `schema_name` instead of
    /// `public`.
    ///
    /// Each [`TestDatabase`] already owns a container, so this is not required for
    /// isolation; it exists to prove the schema applies cleanly outside `public`.
    ///
    /// `search_path` is pinned in the connection URL rather than issued as a `SET`:
    /// the connection is a *pool*, so a `SET` binds to whichever single connection
    /// served it and later statements silently land back in `public`.
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
        let container = Postgres::default()
            .with_tag("18-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");

        let base = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

        // Bootstrap connection: create the schema before anything targets it.
        let bootstrap = Database::connect(&base)
            .await
            .expect("Failed to connect to test database");
        bootstrap
            .execute_unprepared(&format!("CREATE SCHEMA IF NOT EXISTS {schema_name}"))
            .await
            .expect("Failed to create schema");

        // Every pooled connection resolves unqualified names to `schema_name` first.
        let connection_string = format!("{base}?options=-c%20search_path%3D{schema_name}");
        let connection = Database::connect(&connection_string)
            .await
            .expect("Failed to connect with pinned search_path");

        Self::run_migrations_from(&connection, "manifests/db/zerg/schema.sql").await;

        Self {
            container,
            connection,
            connection_string,
        }
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
            "INSERT INTO users (id, email, name) VALUES ('{user_id}', 'test-{user_id}@example.com', 'Test User {user_id}') ON CONFLICT (id) DO NOTHING"
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
