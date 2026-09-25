# gRPC Serialization Optimization Guide

## Current Serialization Overhead

Based on benchmarks, serialization accounts for ~5ms latency (current optimized: 9.66ms total - 4.66ms network/processing = 5ms serialization).

## Optimization Strategies

### 1. Protocol Buffers Schema Optimization

#### Before (Current)

```protobuf
message CreateResponse {
  string id = 1;              // "550e8400-e29b-41d4-a716-446655440000" (36 bytes)
  string title = 2;
  string description = 3;
  bool completed = 4;
  optional string project_id = 5;  // 36 bytes
  string priority = 6;        // "medium" (6 bytes)
  string status = 7;          // "in_progress" (11 bytes)
  optional string due_date = 8;    // "2024-12-06T15:14:21Z" (24 bytes)
  string created_at = 9;      // 24 bytes
  string updated_at = 10;     // 24 bytes
}
```

**Total overhead per task: ~161 bytes (just metadata)**

#### After (Optimized)

```protobuf
enum Priority {
  PRIORITY_UNSPECIFIED = 0;
  LOW = 1;      // 1 byte
  MEDIUM = 2;
  HIGH = 3;
  URGENT = 4;
}

enum Status {
  STATUS_UNSPECIFIED = 0;
  TODO = 1;           // 1 byte
  IN_PROGRESS = 2;
  DONE = 3;
}

message CreateResponse {
  bytes id = 1;              // Binary UUID (16 bytes)
  string title = 2;
  string description = 3;
  bool completed = 4;
  optional bytes project_id = 5;  // 16 bytes
  Priority priority = 6;     // 1 byte
  Status status = 7;         // 1 byte
  optional int64 due_date = 8;    // 8 bytes
  int64 created_at = 9;      // 8 bytes
  int64 updated_at = 10;     // 8 bytes
}
```

**Total overhead per task: ~58 bytes**

### Byte Savings Breakdown

| Field | Before (bytes) | After (bytes) | Savings |
|-------|----------------|---------------|---------|
| UUID (id) | 36 | 16 | 20 bytes (56%) |
| UUID (project_id) | 36 | 16 | 20 bytes (56%) |
| Priority (enum) | 6 | 1 | 5 bytes (83%) |
| Status (enum) | 11 | 1 | 10 bytes (91%) |
| due_date | 24 | 8 | 16 bytes (67%) |
| created_at | 24 | 8 | 16 bytes (67%) |
| updated_at | 24 | 8 | 16 bytes (67%) |
| **Total** | **161** | **58** | **103 bytes (64%)** |

### 2. gRPC Compression

```rust
// Enable gzip compression (CPU for bandwidth trade-off)
TasksServiceClient::new(channel)
    .accept_compressed(CompressionEncoding::Gzip)
    .send_compressed(CompressionEncoding::Gzip)
```

**Impact:**

- Small messages (<1KB): Minimal benefit, adds CPU overhead
- Large responses (list operations): 60-80% size reduction
- **Recommended for**: List operations with >10 items

### 3. Message Size Limits

```rust
.max_decoding_message_size(8 * 1024 * 1024)  // 8MB
.max_encoding_message_size(8 * 1024 * 1024)
```

Prevents memory exhaustion attacks and OOM errors.

### 4. HTTP/2 Optimizations

```rust
.http2_adaptive_window(true)  // Adaptive flow control
.initial_connection_window_size(1024 * 1024)
.initial_stream_window_size(1024 * 1024)
```

**Impact:** Better throughput for large responses

## Performance Impact Estimates

### List Operation (50 tasks)

**Before optimization:**

```text
50 tasks × 161 bytes metadata = 8,050 bytes
+ Actual data (title, description) ≈ 20KB
= ~28KB total payload
```

**After optimization:**

```text
50 tasks × 58 bytes metadata = 2,900 bytes
+ Actual data (same) ≈ 20KB
= ~23KB total payload (18% smaller)
```

**With compression:**

```text
23KB → ~6KB compressed (gzip ratio ~4:1 for text)
```

### Latency Impact

| Optimization | Before | After | Improvement |
|--------------|--------|-------|-------------|
| Schema optimization | 5ms | 4ms | 20% faster |
| + Compression (large) | 5ms | 3ms | 40% faster |
| + Compression (small) | 5ms | 5.5ms | -10% (overhead) |

### Expected Results

**Current:**

- Requests/sec: 5,175
- Avg latency: 9.66ms

**With schema optimization:**

- Requests/sec: ~5,500-6,000 (6-16% gain)
- Avg latency: ~8.5-9ms

**With compression (list operations):**

- Requests/sec: ~6,500-7,000 (for large responses)
- Avg latency: ~7-8ms (for list ops)

## Implementation Steps

### 1. Migrate to Optimized Proto

```bash
# Backup current proto
cp manifests/grpc/proto/apps/v1/tasks.proto manifests/grpc/proto/apps/v1/tasks_legacy.proto

# Replace with optimized version
cp manifests/grpc/proto/apps/v1/tasks_optimized.proto manifests/grpc/proto/apps/v1/tasks.proto

# Regenerate Rust code
task proto-gen
```

### 2. Update Conversion Logic

**Convert UUIDs:**

```rust
// String → Bytes
uuid.as_bytes().to_vec()

// Bytes → UUID
Uuid::from_slice(&bytes)?
```

**Convert Enums:**

```rust
// String → Enum
match priority.as_str() {
    "low" => Priority::Low as i32,
    "medium" => Priority::Medium as i32,
    ...
}

// Enum → String
match Priority::from_i32(value) {
    Some(Priority::Low) => "low",
    ...
}
```

**Convert Timestamps:**

```rust
// DateTime → Unix timestamp
datetime.timestamp()

// Unix timestamp → DateTime
DateTime::from_timestamp(timestamp, 0)
```

### 3. Enable Compression

Already added in `grpc_pool.rs` - no additional work needed!

### 4. Update Tasks Service

Similar changes needed in `apps/zerg/tasks/src/main.rs`.

## Trade-offs

| Optimization | Pros | Cons |
|--------------|------|------|
| **Binary UUIDs** | 56% smaller | More conversion code |
| **Enum integers** | 83-91% smaller | Less readable in debugging |
| **Unix timestamps** | 67% smaller | Need timezone handling |
| **Compression** | 60-80% smaller payloads | CPU overhead (5-10%) |

## When to Use What

### Use Optimized Proto When

✅ High-throughput operations (>1000 req/sec)
✅ Large response payloads (lists)
✅ Network bandwidth is constrained
✅ Binary efficiency matters

### Stick with String Proto When

✅ Development/debugging (human-readable)
✅ Low traffic (<100 req/sec)
✅ Simplicity preferred
✅ Multiple language clients need easy integration

### Enable Compression When

✅ List operations (>10 items)
✅ Responses >5KB
✅ Network is bottleneck, not CPU

### Skip Compression When

❌ Small messages (<1KB)
❌ CPU constrained
❌ Single-item operations

## Benchmark Plan

```bash
# Test current implementation
task bench-tasks-grpc

# After proto optimization
task proto-gen && cargo build -p zerg_api
task bench-tasks-grpc

# Compare results
# Expected: 8-15% improvement in throughput
# Expected: 10-15% reduction in latency
```

## Migration Strategy

### Phase 1: Prepare

- Create optimized proto as `tasks_optimized.proto`
- Add conversion utilities
- Test in development

### Phase 2: Dual Protocol

- Support both v1 (string) and v2 (binary)
- Use different service paths
- Gradual client migration

### Phase 3: Full Migration

- Switch default to optimized
- Deprecate old proto
- Remove after grace period

## Summary

**Realistic expectations:**

- Schema optimization: 10-15% improvement
- Compression (for lists): Additional 20-30%
- Combined: ~30-45% improvement on list operations

**Won't close the gap to direct DB** (still ~1.4x slower), but every bit helps!

The real win is **bandwidth savings** (64% less data) which matters at scale.
