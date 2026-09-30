# Production-Ready Shutdown Guide

This guide explains how to implement proper graceful shutdown for your Axum applications with database connection cleanup.

## The Problem

Simple shutdown implementations have several issues:

1. **No timeout** - Server may hang forever waiting for requests to complete
2. **No cleanup** - Database connections, file handles, and other resources aren't closed
3. **No coordination** - Background tasks don't know when to stop
4. **Data loss** - In-flight operations may not complete

## What Happens During Shutdown

### With Axum's `with_graceful_shutdown()`

1. **Stop accepting new connections** - No new TCP connections accepted
2. **Wait for current requests** - Existing HTTP requests complete
3. **BUT**: No timeout, no cleanup, connections left open

### With Our Production Solution

1. **Signal received** (SIGTERM/SIGINT)
2. **Shutdown coordinator notifies all subsystems**
3. **Server stops accepting new requests**
4. **In-flight requests complete** (with timeout)
5. **Cleanup tasks run** (close DB connections, flush buffers)
6. **Application exits cleanly**

## Shutdown Sequence Details

```text
┌─────────────────────────────────────────────────────┐
│  1. SIGTERM/SIGINT received                         │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
┌─────────────────────────────────────────────────────┐
│  2. ShutdownCoordinator broadcasts to all tasks     │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
┌─────────────────────────────────────────────────────┐
│  3. HTTP server stops accepting new connections     │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
┌─────────────────────────────────────────────────────┐
│  4. Wait for in-flight requests (max: timeout)      │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
┌─────────────────────────────────────────────────────┐
│  5. Run cleanup tasks concurrently:                 │
│     - Close PostgreSQL connection pool              │
│     - Send QUIT to Redis                            │
│     - Flush any buffers                             │
│     - Cancel background tasks                       │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
┌─────────────────────────────────────────────────────┐
│  6. Application exits (code 0)                      │
└─────────────────────────────────────────────────────┘
```

## Database Connection Cleanup

### PostgreSQL (SeaORM)

```rust
// SeaORM's DatabaseConnection has internal connection pool (sqlx)
// Explicit close drains pool and closes all connections
db.close().await?;
```

**What happens if you don't close:**

- Connections left in ESTABLISHED state
- PostgreSQL may keep connections in `pg_stat_activity`
- Connection pool resources not freed
- May hit connection limits on next startup

### Redis

```rust
// ConnectionManager maintains single multiplexed connection
// Closes automatically on drop (no explicit quit() needed)
drop(redis);
```

**What happens if you don't close:**

- Connection left open in Redis
- Redis may eventually timeout (default: 300s)
- ConnectionManager will close on drop, but explicit drop in cleanup is cleaner

**Note:** Redis `ConnectionManager` doesn't expose a `quit()` method. The underlying connection is closed automatically when the ConnectionManager is dropped. For proper observability, we explicitly drop it in the cleanup closure and log the operation.

## Production Usage

### Basic Usage (Recommended)

```rust
use axum_helpers::server::create_production_app;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = Config::from_env()?;
    let db = postgres::connect_from_config(config.database).await?;
    let redis = redis::connect_from_config(config.redis).await?;

    let router = create_router(my_routes).await?;

    // Clone for cleanup closure
    let db_clone = db.clone();
    let redis_clone = redis.clone();

    create_production_app(
        router,
        &config.server,
        Duration::from_secs(30), // 30s graceful shutdown timeout
        async move {
            info!("Closing database connections");

            // Close connections concurrently
            tokio::join!(
                async {
                    match db_clone.close().await {
                        Ok(_) => info!("PostgreSQL closed successfully"),
                        Err(e) => error!("Error closing PostgreSQL: {}", e),
                    }
                },
                async {
                    drop(redis_clone);
                    info!("Redis closed successfully");
                }
            );
        }
    ).await
}
```

### Advanced Usage with CleanupCoordinator

```rust
use axum_helpers::{CleanupCoordinator, create_production_app};
use std::time::Duration;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let db = postgres::connect("...").await?;
    let redis = redis::connect("...").await?;
    let router = create_router(my_routes).await?;

    let db_clone = db.clone();
    let redis_clone = redis.clone();

    create_production_app(
        router,
        &config,
        Duration::from_secs(30),
        async move {
            let mut cleanup = CleanupCoordinator::new();

            cleanup.add_task("postgres", async move {
                db_clone.close().await.ok();
            });

            cleanup.add_task("redis", async move {
                drop(redis_clone);
                info!("Redis connection closed");
            });

            cleanup.add_task("background_jobs", async move {
                // Cancel background tasks
            });

            cleanup.run().await;
        }
    ).await
}
```

### With Shutdown Coordinator (Maximum Control)

```rust
use axum_helpers::{ShutdownCoordinator, create_app};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let (coordinator, mut shutdown_rx) = ShutdownCoordinator::new();

    // Spawn background task that listens for shutdown
    tokio::spawn(async move {
        shutdown_rx.recv().await.ok();
        info!("Background task shutting down");
        // Clean shutdown of background work
    });

    // Your server with coordinator
    // ... implement custom shutdown logic
}
```

## Kubernetes Integration

For Kubernetes deployments, configure proper termination grace period:

```yaml
apiVersion: apps/v1
kind: Deployment
spec:
  template:
    spec:
      terminationGracePeriodSeconds: 60  # Must be > shutdown_timeout
      containers:
      - name: api
        lifecycle:
          preStop:
            exec:
              command: ["/bin/sh", "-c", "sleep 5"]  # Give time for load balancer
        livenessProbe:
          httpGet:
            path: /health
            port: 3000
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /ready
            port: 3000
          initialDelaySeconds: 5
          periodSeconds: 5
```

**Shutdown sequence in Kubernetes:**

1. Pod marked for termination
2. Removed from Service endpoints (takes ~1-2s)
3. `preStop` hook runs (5s sleep)
4. SIGTERM sent to container
5. Application graceful shutdown (30s)
6. If not stopped, SIGKILL after `terminationGracePeriodSeconds`

## Configuration Recommendations

| Environment | Shutdown Timeout | Termination Grace Period |
|-------------|------------------|--------------------------|
| Development | 10s              | N/A                      |
| Staging     | 30s              | 60s                      |
| Production  | 30-60s           | 90-120s                  |

**Why 30-60s for production?**

- Most HTTP requests complete in < 5s
- Database transactions complete in < 10s
- Long-running queries should use background jobs
- Gives ample time for cleanup without hanging

## Monitoring Shutdown

Log what to watch for:

```text
✓ Good shutdown:
  INFO Received SIGTERM, initiating graceful shutdown
  INFO Starting cleanup tasks (timeout: 30s)
  INFO PostgreSQL connection closed successfully
  INFO Redis connection closed successfully
  INFO Cleanup completed successfully

✗ Problem shutdown:
  WARN Cleanup exceeded timeout of 30s, forcing shutdown
  ERROR Error closing PostgreSQL connection: ...

  # Check for:
  # - Long-running queries
  # - Deadlocks
  # - Network issues
  # - Connection pool exhaustion
```

## Testing Shutdown

```bash
# Start your server
cargo run --bin zerg_api

# In another terminal, send SIGTERM
kill -TERM $(pgrep zerg_api)

# Watch logs for:
# 1. "Received SIGTERM"
# 2. "Starting cleanup tasks"
# 3. "closed successfully" for each resource
# 4. "Cleanup completed successfully"
# 5. Clean exit (no panic/error)

# Verify no lingering connections
psql -c "SELECT * FROM pg_stat_activity WHERE datname = 'your_db';"
redis-cli CLIENT LIST
```

## Common Issues

### Issue: Cleanup exceeds timeout

**Symptoms:**

```text
WARN Cleanup exceeded timeout of 30s, forcing shutdown
```

**Causes:**

- Long-running database queries
- Slow network to database
- Deadlocked transaction

**Solutions:**

- Increase timeout
- Cancel long-running queries on shutdown signal
- Use shorter statement timeouts

### Issue: Database "too many connections"

**Symptoms:**

- Next startup fails with "too many connections"
- Old connections visible in `pg_stat_activity`

**Causes:**

- Application crashed without cleanup
- Connections not properly closed

**Solutions:**

- Always use `create_production_app` in production
- Set reasonable `max_connections` in PostgreSQL
- Configure connection pool properly

### Issue: Requests fail during shutdown

**Symptoms:**

- 502/503 errors during deployment

**Causes:**

- Load balancer sends requests after SIGTERM
- No `preStop` hook delay

**Solutions:**

- Add `preStop` sleep (5-10s)
- Ensure `terminationGracePeriodSeconds` > shutdown timeout
- Use readiness probe to stop traffic before shutdown

## Migration from Simple Shutdown

```rust
// BEFORE (not production-ready)
axum::serve(listener, router)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
// Issues: No timeout, no cleanup, may hang

// AFTER (production-ready)
use std::time::Duration;

let db_clone = db.clone();
let redis_clone = redis.clone();

create_production_app(
    router,
    &config.server,
    Duration::from_secs(30),
    async move {
        tokio::join!(
            db_clone.close(),
            async {
                let mut redis = redis_clone;
                redis.quit::<()>().await.ok();
            }
        );
    }
).await?;
```

## Further Reading

- [SeaORM Connection Pooling](https://www.sea-ql.org/SeaORM/docs/install-and-config/connection/)
- [Redis Connection Management](https://redis.io/docs/reference/clients/)
- [Kubernetes Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/)
- [Graceful Shutdown Best Practices](https://cloud.google.com/blog/products/containers-kubernetes/kubernetes-best-practices-terminating-with-grace)
