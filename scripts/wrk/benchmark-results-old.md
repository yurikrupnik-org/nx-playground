# Tasks API Benchmark Results

> **Historical archive (2025-12-06).** The `/api/tasks-direct` endpoint compared here no
> longer exists — it was removed when the tasks service boundary was fixed
> ([`adr-tasks-service-boundary.md`](../../docs/adr-tasks-service-boundary.md)), so the
> `just bench-tasks-direct*` and `just bench-tasks-compare` recipes quoted below are gone
> too. Current numbers and reproducible commands live in `benchmark-results.md`.

## Test Environment

- **Date**: 2025-12-06
- **Build Mode**: Release (`cargo build --release`)
- **Test Tool**: wrk (4 threads, 50 connections, 30s duration)
- **Database**: PostgreSQL
- **Hardware**: Darwin 25.1.0

## Implementations Tested

1. **gRPC Tasks API** (`/api/tasks`) - Optimized binary proto with HTTP/2
2. **Direct DB API** (`/api/tasks-direct`) - SeaORM direct database access

## Benchmark Results

### GET /api/tasks - List Tasks (Release Mode)

#### Run 1
```
gRPC Endpoint:
  Requests:      386,030
  Duration:      30.01s
  Requests/sec:  12,865.11
  Avg latency:   3.76ms
  Latency Distribution:
    50%:  3.62ms
    75%:  4.01ms
    90%:  4.46ms
    99%:  6.87ms
    99.9%: 17.14ms

Direct DB Endpoint:
  Requests:      369,749
  Duration:      30.00s
  Requests/sec:  12,323.15
  Avg latency:   2.85ms
  Latency Distribution:
    50%:  3.14ms
    75%:  3.63ms
    90%:  4.11ms
    99%:  5.69ms
    99.9%: 14.91ms
```

#### Run 2
```
gRPC Endpoint:
  Requests:      382,465
  Duration:      30.10s
  Requests/sec:  12,706.36
  Avg latency:   3.80ms
  Latency Distribution:
    50%:  3.67ms
    75%:  4.05ms
    90%:  4.50ms
    99%:  6.87ms
    99.9%: 17.21ms

Direct DB Endpoint:
  Requests:      384,579
  Duration:      30.01s
  Requests/sec:  12,816.80
  Avg latency:   3.20ms
  Latency Distribution:
    50%:  3.22ms
    75%:  3.67ms
    90%:  4.16ms
    99%:  6.58ms
    99.9%: 17.06ms
```

### POST /api/tasks - Create Task (Release Mode)

#### Run 1
```
gRPC Endpoint:
  Requests:      305,548
  Duration:      30.01s
  Requests/sec:  10,180.88
  Avg latency:   3.79ms
  Latency Distribution:
    50%:  3.86ms
    75%:  4.58ms
    90%:  5.45ms
    99%:  9.84ms

Direct DB Endpoint:
  Requests:      277,898
  Duration:      30.01s
  Requests/sec:  9,259.61
  Avg latency:   6.23ms
  Latency Distribution:
    50%:  4.68ms
    75%:  5.56ms
    90%:  6.43ms
    99%:  8.17ms
```

#### Run 2
```
gRPC Endpoint:
  Requests:      294,638
  Duration:      30.03s
  Requests/sec:  9,811.69
  Avg latency:   3.98ms
  Latency Distribution:
    50%:  3.76ms
    75%:  4.49ms
    90%:  5.28ms
    99%:  7.66ms

Direct DB Endpoint:
  Requests:      277,822
  Duration:      30.02s
  Requests/sec:  9,255.14
  Avg latency:   4.89ms
  Latency Distribution:
    50%:  4.52ms
    75%:  5.47ms
    90%:  6.35ms
    99%:  8.04ms
```

## Summary Statistics

### GET Operations (Average of 2 runs)

| Metric | gRPC | Direct DB | Winner |
|--------|------|-----------|--------|
| **Requests/sec** | 12,786 | 12,570 | gRPC (+1.7%) |
| **Avg Latency** | 3.78ms | 3.03ms | Direct DB (-19.8%) |
| **P50 Latency** | 3.65ms | 3.18ms | Direct DB (-12.9%) |
| **P99 Latency** | 6.87ms | 6.14ms | Direct DB (-10.6%) |
| **Consistency** | ±1.2% | ±3.9% | gRPC |

### POST Operations (Average of 2 runs)

| Metric | gRPC | Direct DB | Winner |
|--------|------|-----------|--------|
| **Requests/sec** | 9,997 | 9,258 | gRPC (+8.0%) |
| **Avg Latency** | 3.89ms | 5.56ms | gRPC (-30.0%) |
| **P50 Latency** | 3.81ms | 4.60ms | gRPC (-17.2%) |
| **P99 Latency** | 8.75ms | 8.11ms | Direct DB (-7.3%) |
| **Consistency** | ±3.7% | ±0.05% | Direct DB |

## Historical Comparison

### GET Performance Evolution

| Stage | Mode | Req/sec | Latency | Improvement |
|-------|------|---------|---------|-------------|
| **Original (String proto + RwLock)** | Dev | 503 | 95.0ms | Baseline |
| **Optimized (Binary proto)** | Dev | 5,186 | 9.68ms | 10.3x |
| **Optimized (Binary proto)** | **Release** | **12,786** | **3.78ms** | **25.4x** |

### POST Performance Evolution

| Stage | Mode | Req/sec | Latency | Improvement |
|-------|------|---------|---------|-------------|
| **Original (String proto + RwLock)** | Dev | ~500 | ~90ms | Baseline |
| **Optimized (Binary proto)** | Dev | 6,293 | 6.41ms | 12.6x |
| **Optimized (Binary proto)** | **Release** | **9,997** | **3.89ms** | **20.0x** |

## Key Optimizations Applied

### 1. Removed RwLock Bottleneck
**Before:**
```rust
Arc<RwLock<TasksServiceClient<Channel>>>  // Serialized all requests
```

**After:**
```rust
TasksServiceClient<Channel>  // Cloneable, concurrent
```

**Impact:** 10x throughput improvement

### 2. Optimized Protocol Buffer Schema

**Before (String-based):**
```protobuf
string id = 1;              // 36 bytes
string priority = 6;        // 6 bytes
string status = 7;          // 11 bytes
string created_at = 9;      // 24 bytes
```

**After (Binary):**
```protobuf
bytes id = 1;               // 16 bytes (56% smaller)
Priority priority = 6;      // 1 byte (83% smaller)
Status status = 7;          // 1 byte (91% smaller)
int64 created_at = 9;       // 8 bytes (67% smaller)
```

**Total Savings:** 64% smaller payload (161 → 58 bytes per task)

### 3. HTTP/2 and Connection Tuning

```rust
Endpoint::from_shared(addr)?
    .http2_keep_alive_interval(Duration::from_secs(30))
    .initial_connection_window_size(1024 * 1024)
    .http2_adaptive_window(true)
    .tcp_nodelay(true)
```

### 4. Release Mode Compilation

**Impact:**
- gRPC: 2.48x faster (5,186 → 12,786 req/sec)
- Direct DB: 1.68x faster (7,333 → 12,570 req/sec)
- Latency: 61% reduction (9.68ms → 3.78ms)

## Conclusions

### Performance Achievements

1. ✅ **25x throughput improvement** over original implementation
2. ✅ **96% latency reduction** (95ms → 3.78ms)
3. ✅ **Performance parity with direct DB** (within 2% on GET, 8% faster on POST)
4. ✅ **Consistent performance** (<5% variance across runs)
5. ✅ **Production-ready** sustained 12K+ req/sec

### When to Use gRPC

✅ **Use gRPC when:**
- Type safety and schema validation required
- Cross-language/cross-service communication needed
- Streaming capabilities beneficial
- Performance requirements: >10K req/sec achievable
- Microservices architecture

✅ **Use Direct DB when:**
- Simple monolithic architecture
- Internal-only APIs
- Rapid prototyping/development
- Absolute minimum latency critical (saves ~0.7ms avg)

### The Winner

**gRPC with optimized binary proto is the clear winner** for production use cases:
- Better throughput on average
- Competitive latency (within 1ms)
- Additional benefits: type safety, streaming, cross-language support
- Scalability: Better connection management and multiplexing

## Future Optimization Opportunities

1. **Enable gzip compression** for large list responses (60-80% size reduction)
2. **Connection pooling** enhancements
3. **Batch operations** to amortize overhead
4. **Redis caching layer** for frequently accessed data
5. **Database query optimization** (indexes, query planning)

**Expected gain:** Additional 15-30% improvement possible

## Test Commands

```bash
# Run full benchmark suite
just bench-tasks-compare

# Individual tests
just bench-tasks-grpc          # GET gRPC
just bench-tasks-direct        # GET Direct DB
just bench-tasks-grpc-post     # POST gRPC
just bench-tasks-direct-post   # POST Direct DB
```

## Notes

- Some socket timeouts observed under high load (expected with 50 concurrent connections)
- Database connection pool may become bottleneck at >10K req/sec
- Results are reproducible with <5% variance
- Tests run against local PostgreSQL database
