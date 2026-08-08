# Tasks API Benchmarks

Performance benchmarks comparing gRPC-based and direct database access endpoints for the Tasks API.

## Available Commands

### Quick Benchmarks (10s, light load)
```bash
just bench-tasks-quick
```
- 2 threads, 10 connections
- Good for quick testing during development

### Individual Endpoint Benchmarks (30s, moderate load)

**GET endpoints:**
```bash
just bench-tasks-grpc        # gRPC: GET /api/tasks
```

**POST endpoints:**
```bash
just bench-tasks-grpc-post   # gRPC: POST /api/tasks
```

- 4 threads, 50 connections
- 30 second duration
- Includes detailed latency distribution

### Full Run
```bash
just bench-tasks-all
```
Runs the GET and POST benchmarks sequentially.

## Benchmark Configuration

**Default settings:**
- Threads: 4
- Connections: 50
- Duration: 30s
- Custom Lua reporting script with percentile distribution

**Endpoints tested:**
- `http://localhost:8080/api/tasks` (via the `zerg_tasks` gRPC service)

> The former `/api/tasks-direct` endpoint (in-process SeaORM access to the same table)
> was removed — see [`docs/adr-tasks-service-boundary.md`](../../docs/adr-tasks-service-boundary.md).
> Historical gRPC-vs-direct numbers are kept in `benchmark-results.md`.

## Customizing Benchmarks

### Change load parameters
Edit the justfile recipes to adjust:
- `-t<N>`: Number of threads
- `-c<N>`: Number of connections
- `-d<TIME>`: Duration (e.g., `10s`, `1m`, `2h`)

Example:
```bash
wrk -t8 -c100 -d60s --latency -s scripts/wrk/report.lua http://localhost:8080/api/tasks-direct
```

### Modify POST payload
Edit `scripts/wrk/post-task.lua` to change the request body:
```lua
wrk.body = '{"title":"Custom Task","priority":"urgent"}'
```

## Understanding Results

### Key Metrics

**Throughput:**
- `Requests/sec`: Higher is better
- Direct DB typically 5-10x higher than gRPC

**Latency:**
- `Avg latency`: Mean response time
- `50%/75%/90%/99%`: Percentile distribution
- Direct DB typically 85-90% faster than gRPC

### Example Output
```
Requests:      228017
Duration:      30.02s
Requests/sec:  7594.90
Avg latency:   6.76ms

Latency Distribution:
  50%:  5.81ms    # 50% of requests faster than this
  75%:  6.96ms
  90%:  8.70ms    # 90% under this threshold
  99%:  27.47ms   # 99th percentile (tail latency)
  99.9%: 82.03ms
```

## Performance Comparison Summary

Based on typical results:

| Metric | gRPC Endpoint | Direct DB | Improvement |
|--------|---------------|-----------|-------------|
| Throughput | ~500-700 req/s | ~5000-7500 req/s | **8-10x** |
| Avg Latency | 15-20ms | 2-7ms | **70-90%** |
| P99 Latency | 50-100ms | 20-30ms | **60-70%** |

## Prerequisites

Ensure all services are running:
```bash
# Start database
just _docker-up

# Start tasks gRPC service
cargo run -p zerg_tasks

# Start API service
cargo run -p zerg_api
```

## Notes

- **Warm-up**: First few requests may be slower due to connection pool initialization
- **Database state**: Large datasets will affect read performance
- **System load**: Close other applications for accurate results
- **Network**: Tests assume localhost (no network latency)

## Files

- `post-task.lua`: wrk script for POST requests
- `report.lua`: Enhanced reporting with percentile distribution
- `README.md`: This file
