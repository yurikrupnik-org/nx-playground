# Test Utils

Shared test infrastructure for all domain crates in the monorepo.

## Features

- **`postgres`** (default): PostgreSQL test infrastructure with automatic container management
- **`redis`**: Redis test infrastructure with automatic container management
- **`all`**: All database test infrastructure

## Components

### TestDatabase (PostgreSQL)

Provides a PostgreSQL 18 container for integration testing with automatic cleanup.

**Features required:** `postgres` (enabled by default)

```toml
[dev-dependencies]
test-utils = { workspace = true }
```

**Usage:**

```rust
use test_utils::TestDatabase;

#[tokio::test]
async fn test_with_postgres() {
    let db = TestDatabase::new().await;
    let repo = MyRepository::new(db.connection());

    // Your test logic here
}
```

### TestRedis

Provides a Redis 8 container for caching/session testing with automatic cleanup.

**Features required:** `redis`

```toml
[dev-dependencies]
test-utils = { workspace = true, features = ["redis"] }
```

**Usage:**

```rust
use test_utils::TestRedis;
use redis::AsyncCommands;

#[tokio::test]
async fn test_with_redis() {
    let redis = TestRedis::new().await;
    let mut conn = redis.connection();

    // Set and get values
    conn.set::<_, _, ()>("session:123", "user_data").await.unwrap();
    let value: String = conn.get("session:123").await.unwrap();

    assert_eq!(value, "user_data");
}
```

### TestDataBuilder

Generates deterministic test data for reproducible tests. **Always available** (no feature required).

**Usage:**

```rust
use test_utils::TestDataBuilder;

#[tokio::test]
async fn test_with_deterministic_data() {
    let builder = TestDataBuilder::from_test_name("my_test");

    // These values will be the same every time this test runs
    let user_id = builder.user_id();
    let project_name = builder.name("project", "main");
    // => "test-project-12345678-main"
}
```

### Assertions

Custom assertion helpers for better error messages.

```rust
use test_utils::assertions::{assert_uuid_eq, assert_some};

let user_id = assert_some(result, "user should exist");
assert_uuid_eq(actual_id, expected_id, "user ID mismatch");
```

## Usage Examples

### PostgreSQL Only (Default)

```toml
# libs/domains/projects/Cargo.toml
[dev-dependencies]
test-utils = { workspace = true }
```

```rust
// libs/domains/projects/tests/integration_test.rs
use test_utils::{TestDatabase, TestDataBuilder};

#[tokio::test]
async fn test_create_project() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("create_project");

    let repo = ProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));

    // Test logic...
}
```

### Redis Only

```toml
# libs/domains/sessions/Cargo.toml
[dev-dependencies]
test-utils = { workspace = true, features = ["redis"] }
redis = { workspace = true }
```

```rust
// libs/domains/sessions/tests/integration_test.rs
use test_utils::{TestRedis, TestDataBuilder};
use redis::AsyncCommands;

#[tokio::test]
async fn test_session_storage() {
    let redis = TestRedis::new().await;
    let builder = TestDataBuilder::from_test_name("session_test");

    let session_id = builder.user_id().to_string();
    let mut conn = redis.connection();

    // Store session
    conn.set_ex::<_, _, ()>(
        format!("session:{}", session_id),
        "user_data",
        3600  // 1 hour TTL
    ).await.unwrap();

    // Retrieve session
    let exists: bool = conn.exists(format!("session:{}", session_id)).await.unwrap();
    assert!(exists);
}
```

### Both PostgreSQL and Redis

```toml
# libs/domains/hybrid/Cargo.toml
[dev-dependencies]
test-utils = { workspace = true, features = ["postgres", "redis"] }
redis = { workspace = true }
```

```rust
// libs/domains/hybrid/tests/integration_test.rs
use test_utils::{TestDatabase, TestRedis, TestDataBuilder};
use redis::AsyncCommands;

#[tokio::test]
async fn test_with_db_and_cache() {
    let db = TestDatabase::new().await;
    let cache = TestRedis::new().await;
    let builder = TestDataBuilder::from_test_name("hybrid_test");

    // Use both database and cache
    let repo = MyRepository::new(db.connection());
    let mut redis_conn = cache.connection();

    // Write to database
    let project = repo.create(input).await.unwrap();

    // Cache in Redis
    let cache_key = format!("project:{}", project.id);
    redis_conn.set_ex::<_, _, ()>(
        &cache_key,
        serde_json::to_string(&project).unwrap(),
        300  // 5 minutes
    ).await.unwrap();

    // Read from cache
    let cached: String = redis_conn.get(&cache_key).await.unwrap();
    let cached_project: Project = serde_json::from_str(&cached).unwrap();

    assert_eq!(cached_project.id, project.id);
}
```

## Advanced Usage

### Schema Isolation for Parallel Tests

When running tests in parallel, use separate schemas to avoid conflicts:

```rust
#[tokio::test]
async fn test_with_isolated_schema() {
    let db = TestDatabase::with_schema("test_create_project").await;
    // This test runs in its own schema
}
```

### Custom Redis Configuration

```rust
#[tokio::test]
async fn test_with_redis_client() {
    let redis = TestRedis::new().await;

    // Get connection string for custom client
    let conn_str = redis.connection_string();
    let client = redis::Client::open(conn_str).unwrap();

    // Or use the provided connection
    let mut conn = redis.connection();
}
```

## Container Management

All test containers are **automatically cleaned up** when the test ends:

- `TestDatabase` drops the PostgreSQL container on `Drop`
- `TestRedis` drops the Redis container on `Drop`
- No manual cleanup needed!

## Running Tests

```bash
# Test with PostgreSQL only
cargo test -p domain_projects

# Test with Redis only
cargo test -p test-utils --features redis

# Test with all features
cargo test -p test-utils --all-features

# Test specific integration test
cargo test -p domain_projects --test integration_test
```

## Performance

| Infrastructure | Startup Time | Typical Test Time |
|----------------|--------------|-------------------|
| TestDatabase   | ~1-2s        | ~3s (12 tests)    |
| TestRedis      | ~0.5-1s      | ~2s (5 tests)     |

## Troubleshooting

### Container Won't Start

```bash
# Check Docker is running
docker ps

# Clean up old containers
docker rm -f $(docker ps -aq)
```

### Port Conflicts

Testcontainers automatically assigns random ports, so conflicts are rare. If you encounter issues, check for orphaned containers.

### Slow Tests

Tests using real databases are slower than unit tests with mocks:
- Unit tests (mocked): 0.00s
- Integration tests (real DB): ~3s
- Handler tests (real DB + HTTP): ~2-5s

Use unit tests with mocks for fast feedback, integration tests for data layer verification.
