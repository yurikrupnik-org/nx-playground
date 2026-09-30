# Testing Production Shutdown - Zerg API

This guide explains how to test the production-ready shutdown behavior of the Zerg API.

## Prerequisites

- PostgreSQL running (local or configured via env vars)
- Redis running (local or configured via env vars)
- Optional: Tasks gRPC service running

## Quick Test

### 1. Start the API

```bash
cd apps/zerg/api
cargo run
```

You should see:

```text
INFO Starting zerg API with production-ready shutdown (30s timeout)
INFO Server starting on 0.0.0.0:3000
```

### 2. Send a Test Request

In another terminal:

```bash
curl http://localhost:3000/health
```

Should return:

```json
{
  "status": "ok",
  "service": "zerg_api",
  "version": "0.1.0"
}
```

### 3. Test Graceful Shutdown

Send SIGTERM:

```bash
# Find the process
pgrep zerg_api

# Send SIGTERM (or use Ctrl+C in the terminal)
kill -TERM $(pgrep zerg_api)
```

### 4. Verify Shutdown Logs

You should see logs in this order:

```text
✅ INFO Received SIGTERM, initiating graceful shutdown
✅ INFO Starting cleanup tasks (timeout: 30s)
✅ INFO Shutting down: closing database connections
✅ INFO PostgreSQL connection closed successfully
✅ INFO Redis connection closed successfully
✅ INFO Cleanup completed successfully
✅ INFO Zerg API shutdown complete
```

## Testing with In-Flight Requests

### Terminal 1: Start API

```bash
cargo run
```

### Terminal 2: Send Long Request

```bash
# Send request that might take time
curl http://localhost:3000/projects?limit=1000
```

### Terminal 3: Trigger Shutdown While Request Running

```bash
# While the request above is processing
kill -TERM $(pgrep zerg_api)
```

**Expected behavior:**

- Request completes normally (or times out after 30s)
- Shutdown waits for request to finish
- Then runs cleanup

## Testing Timeout Behavior

To test the 30s timeout, you'd need to create a long-running handler. For now, the timeout protects against:

- Hung database queries
- Slow cleanup operations
- Network issues

## Verify Database Connections Closed

### PostgreSQL

```bash
# Before shutdown - you'll see connections
psql -U your_user -d your_db -c "SELECT * FROM pg_stat_activity WHERE datname = 'your_db';"

# Send shutdown signal
kill -TERM $(pgrep zerg_api)

# After shutdown - connections should be gone
psql -U your_user -d your_db -c "SELECT * FROM pg_stat_activity WHERE datname = 'your_db';"
```

### Redis

```bash
# Before shutdown
redis-cli CLIENT LIST

# Send shutdown signal
kill -TERM $(pgrep zerg_api)

# After shutdown - check connections
redis-cli CLIENT LIST
```

## Common Scenarios

### Scenario 1: Normal Shutdown (No Active Requests)

```text
1. SIGTERM received
2. Server stops accepting connections
3. No requests in flight, moves to cleanup
4. PostgreSQL pool closed (< 1s)
5. Redis connection dropped (instant)
6. Process exits
Total: ~1-2s
```

### Scenario 2: Shutdown with Active Requests

```text
1. SIGTERM received
2. Server stops accepting new connections
3. Wait for 2 active requests to complete (5s each)
4. Requests complete after 10s
5. Move to cleanup
6. Close connections (< 1s)
7. Process exits
Total: ~11s
```

### Scenario 3: Hung Request Timeout

```text
1. SIGTERM received
2. Server stops accepting new connections
3. 1 request stuck in database query
4. Wait up to 30s for request
5. After 30s, force shutdown
6. Cleanup runs (best effort)
7. Process exits
Total: ~30s (timeout)
```

## Testing in Kubernetes

### Deploy to K8s

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: zerg-api
spec:
  template:
    spec:
      terminationGracePeriodSeconds: 60  # Must be > 30s shutdown timeout
      containers:
      - name: api
        image: your-registry/zerg-api:latest
        lifecycle:
          preStop:
            exec:
              # Give load balancer time to remove from endpoints
              command: ["/bin/sh", "-c", "sleep 5"]
```

### Test Rolling Update

```bash
# Start rolling update
kubectl rollout restart deployment/zerg-api

# Watch logs during shutdown
kubectl logs -f deployment/zerg-api --previous

# Should see:
# - Received SIGTERM
# - Cleanup tasks
# - Connections closed
# - Clean exit
```

### Monitor During Shutdown

```bash
# Watch pod status
kubectl get pods -w

# Check for 502/503 errors during rollout
# Should be minimal due to:
# 1. preStop hook delay
# 2. Readiness probe removal
# 3. Graceful shutdown
```

## Error Cases to Test

### 1. Database Connection Already Closed

Manually close DB before shutdown:

```sql
-- In psql
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = 'your_db' AND pid <> pg_backend_pid();
```

Then trigger shutdown - should see error log but not crash.

### 2. Redis Connection Lost

Stop Redis:

```bash
# Stop Redis
redis-cli SHUTDOWN

# Then stop API
kill -TERM $(pgrep zerg_api)
```

Should see error but graceful shutdown continues.

### 3. Rapid Successive Signals

```bash
# Send multiple signals quickly
kill -TERM $(pgrep zerg_api) && \
kill -TERM $(pgrep zerg_api) && \
kill -TERM $(pgrep zerg_api)
```

Should only trigger shutdown once (idempotent).

## Metrics to Monitor

During shutdown, watch:

1. **Response Times**: Should not increase significantly
2. **Error Rates**: Should stay low (< 1% 502/503)
3. **Active Connections**: Should drain to 0
4. **Shutdown Duration**: Should complete within 30s typically

## Troubleshooting

### Issue: "Cleanup exceeded timeout"

**Logs:**

```text
WARN Cleanup exceeded timeout of 30s, forcing shutdown
```

**Investigate:**

```sql
-- Check for long queries
SELECT pid, now() - query_start as duration, query
FROM pg_stat_activity
WHERE state = 'active' AND query NOT LIKE '%pg_stat_activity%'
ORDER BY duration DESC;
```

### Issue: Connections not closing

**Verify SeaORM pool settings:**

```rust
// Check in connection code
ConnectOptions::new(url)
    .max_connections(100)
    .max_lifetime(Duration::from_secs(8))
    // ...
```

### Issue: 502 errors during K8s rollout

**Check:**

1. `terminationGracePeriodSeconds` > shutdown timeout
2. `preStop` hook has 5-10s delay
3. Readiness probe fails quickly when shutting down

## Success Criteria

✅ Clean shutdown logs (no errors)
✅ All database connections closed
✅ Redis connection released
✅ No 502/503 during K8s rollouts
✅ Shutdown completes in < 30s normally
✅ In-flight requests complete before shutdown

## Next Steps

After verifying shutdown works:

1. **Add Metrics**: Instrument shutdown duration
2. **Add Alerts**: Alert on shutdown timeouts
3. **Load Test**: Test with realistic traffic
4. **Chaos Engineering**: Test random pod kills
