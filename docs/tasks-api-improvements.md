# Tasks API - Potential Improvements

## Current Performance

- **GET**: 12,786 req/sec @ 3.78ms avg latency
- **POST**: 9,997 req/sec @ 3.89ms avg latency
- **Gap to Direct DB**: Within 2-8% (excellent)

## Proposed Improvements

### 1. Enable gRPC Compression (Quick Win)

**Current State:** Compression code exists but is commented out

**Implementation:**

```toml
# Cargo.toml
[dependencies]
tonic = { version = "0.12", features = ["gzip"] }
```

```rust
// apps/zerg/api/src/grpc_pool.rs
let client = TasksServiceClient::new(channel)
    .accept_compressed(CompressionEncoding::Gzip)
    .send_compressed(CompressionEncoding::Gzip)
    .max_decoding_message_size(8 * 1024 * 1024)
    .max_encoding_message_size(8 * 1024 * 1024);
```

**Expected Impact:**

- List operations (50+ items): 60-80% bandwidth reduction
- Single operations: Minimal benefit (might add CPU overhead)
- Use selectively: Enable only for large responses

**Recommendation:** ⭐⭐⭐ Implement with conditional compression based on response size

---

### 2. Connection Pool for gRPC Clients (Concurrency)

**Current State:** Single client per request (cloned)

**Implementation:**

```rust
// apps/zerg/api/src/state.rs
pub struct AppState {
    pub db: DatabaseConnection,
    pub tasks_client_pool: Arc<Vec<TasksServiceClient<Channel>>>, // Pool of clients
}

// main.rs
let pool_size = num_cpus::get();
let client_pool = create_client_pool(tasks_addr, pool_size).await?;

// Use round-robin or random selection
let client_idx = rand::random::<usize>() % state.tasks_client_pool.len();
let mut client = state.tasks_client_pool[client_idx].clone();
```

**Expected Impact:**

- Better load distribution across connections
- Reduced connection setup overhead
- 5-10% throughput improvement at high concurrency

**Recommendation:** ⭐⭐ Moderate benefit, adds complexity

---

### 3. Redis Caching Layer (Scalability)

**Implementation:**

```rust
// Add caching for list operations
pub async fn list_tasks(State(state): State<AppState>) -> impl IntoResponse {
    let cache_key = "tasks:list:default";

    // Try cache first
    if let Ok(Some(cached)) = state.redis.get::<String>(cache_key).await {
        if let Ok(tasks) = serde_json::from_str(&cached) {
            return (StatusCode::OK, Json(tasks)).into_response();
        }
    }

    // Fetch from gRPC
    let tasks = fetch_from_grpc(&state).await?;

    // Cache for 5 seconds
    let _ = state.redis.setex(cache_key, 5, serde_json::to_string(&tasks)?).await;

    (StatusCode::OK, Json(tasks)).into_response()
}
```

**Expected Impact:**

- Massive improvement for read-heavy workloads (10-100x for cached responses)
- Reduces database load
- Sub-millisecond latency for cached data

**Trade-offs:**

- Added complexity (cache invalidation)
- Stale data risk (need TTL strategy)
- Infrastructure dependency

**Recommendation:** ⭐⭐⭐⭐ High impact for production, but adds operational complexity

---

### 4. Batch Operations (Efficiency)

**Add new endpoints:**

```protobuf
// tasks.proto
message BatchCreateRequest {
  repeated CreateRequest tasks = 1;
}

message BatchCreateResponse {
  repeated CreateResponse tasks = 1;
  int32 created_count = 2;
}

service TasksService {
  rpc BatchCreate(BatchCreateRequest) returns (BatchCreateResponse) {}
  rpc BatchUpdate(BatchUpdateRequest) returns (BatchUpdateResponse) {}
  rpc BatchDelete(BatchDeleteRequest) returns (BatchDeleteResponse) {}
}
```

**Expected Impact:**

- 50-200% improvement for bulk operations
- Reduced round-trip overhead
- Better database transaction efficiency

**Recommendation:** ⭐⭐⭐⭐ Very useful for bulk imports/updates

---

### 5. Database Optimizations

#### A. Add Missing Indexes

```sql
-- Check current indexes
\d tasks

-- Add composite indexes for common queries
CREATE INDEX idx_tasks_status_priority ON tasks(status, priority);
CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_tasks_due_date ON tasks(due_date) WHERE due_date IS NOT NULL;
```

#### B. Optimize List Query

```rust
// Use SELECT only needed columns instead of SELECT *
// Current: Fetches all columns
// Optimized: SELECT id, title, status, priority, created_at

// Add query planning
let tasks = Task::find()
    .select_only()
    .columns([
        task::Column::Id,
        task::Column::Title,
        task::Column::Status,
        task::Column::Priority,
        task::Column::CreatedAt,
    ])
    .filter(filter_conditions)
    .limit(limit)
    .offset(offset)
    .all(&db)
    .await?;
```

#### C. Connection Pool Tuning

```rust
// config/database.rs
let pool_options = DatabaseOptions {
    max_connections: 100,
    min_connections: 10,
    connect_timeout: Duration::from_secs(5),
    idle_timeout: Some(Duration::from_secs(60)),
    acquire_timeout: Duration::from_secs(5),
};
```

**Expected Impact:**

- Indexes: 20-50% improvement for filtered queries
- Column selection: 10-20% bandwidth reduction
- Pool tuning: 5-15% improvement under load

**Recommendation:** ⭐⭐⭐⭐⭐ Essential for production

---

### 6. Cursor-Based Pagination (Large Datasets)

**Current:** Offset-based (inefficient for large offsets)

**Improved:**

```protobuf
message ListRequest {
  optional string cursor = 1;  // Base64-encoded cursor
  int32 page_size = 2;
}

message ListResponse {
  repeated Task tasks = 1;
  string next_cursor = 2;
  bool has_more = 3;
}
```

```rust
// Implementation
pub async fn list_tasks_cursor(cursor: Option<String>, limit: usize) -> Result<(Vec<Task>, String)> {
    let (id_cursor, created_at_cursor) = decode_cursor(cursor)?;

    let tasks = Task::find()
        .filter(
            task::Column::CreatedAt.lt(created_at_cursor)
                .or(
                    task::Column::CreatedAt.eq(created_at_cursor)
                        .and(task::Column::Id.lt(id_cursor))
                )
        )
        .order_by_desc(task::Column::CreatedAt)
        .order_by_desc(task::Column::Id)
        .limit(limit + 1)  // Fetch one extra to check has_more
        .all(&db)
        .await?;

    let has_more = tasks.len() > limit;
    let tasks = tasks.into_iter().take(limit).collect();
    let next_cursor = encode_cursor(&tasks.last())?;

    Ok((tasks, next_cursor))
}
```

**Expected Impact:**

- Constant-time pagination regardless of offset
- 10-100x faster for deep pagination (offset > 10000)

**Recommendation:** ⭐⭐⭐⭐ Critical for applications with many records

---

### 7. Request Validation & Sanitization

```rust
// Add validation
#[derive(Debug, Deserialize, Validate)]
pub struct CreateTaskDto {
    #[validate(length(min = 1, max = 255))]
    pub title: String,

    #[validate(length(max = 5000))]
    pub description: String,

    #[validate(custom = "validate_future_date")]
    pub due_date: Option<String>,
}

fn validate_future_date(date: &str) -> Result<(), ValidationError> {
    let parsed = chrono::DateTime::parse_from_rfc3339(date)
        .map_err(|_| ValidationError::new("invalid_date"))?;

    if parsed < Utc::now() {
        return Err(ValidationError::new("past_date"));
    }

    Ok(())
}
```

**Expected Impact:**

- Better error messages
- Prevent invalid data from reaching database
- Security: Prevent injection attacks

**Recommendation:** ⭐⭐⭐⭐⭐ Essential for production

---

### 8. Observability & Metrics

```rust
// Add Prometheus metrics
use prometheus::{IntCounter, Histogram, Registry};

#[derive(Clone)]
pub struct Metrics {
    pub requests_total: IntCounter,
    pub request_duration: Histogram,
    pub grpc_errors: IntCounter,
}

pub async fn list_tasks(State(state): State<AppState>) -> impl IntoResponse {
    let start = Instant::now();
    state.metrics.requests_total.inc();

    let result = fetch_tasks(&state).await;

    state.metrics.request_duration.observe(start.elapsed().as_secs_f64());

    if result.is_err() {
        state.metrics.grpc_errors.inc();
    }

    result
}
```

**Expected Impact:**

- Better visibility into performance
- Easier debugging of issues
- Proactive monitoring

**Recommendation:** ⭐⭐⭐⭐⭐ Critical for production

---

### 9. Streaming Optimizations

**Current:** ListStream loads all data, then streams

**Improved:**

```rust
pub async fn list_stream(&self, request: Request<ListStreamRequest>)
    -> Result<Response<Self::ListStreamStream>, Status> {

    let req = request.into_inner();
    let filter = build_filter(&req)?;

    // Stream directly from database cursor (don't load all in memory)
    let stream = self.service
        .stream_tasks(filter)  // Returns Stream<Item = Task>
        .await?
        .map(|task| {
            Ok(ListStreamResponse {
                id: uuid_to_bytes(task.id),
                title: task.title,
                // ... other fields
            })
        });

    Ok(Response::new(Box::pin(stream)))
}
```

**Expected Impact:**

- Constant memory usage regardless of dataset size
- Start streaming immediately (don't wait for full query)
- Better for large datasets

**Recommendation:** ⭐⭐⭐ Good for large result sets

---

### 10. Rate Limiting

```rust
use tower::ServiceBuilder;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

let governor_conf = Box::new(
    GovernorConfigBuilder::default()
        .per_second(100)
        .burst_size(20)
        .finish()
        .unwrap(),
);

let app = Router::new()
    .route("/tasks", get(list_tasks))
    .layer(ServiceBuilder::new().layer(GovernorLayer { config: governor_conf }))
    .with_state(state);
```

**Expected Impact:**

- Protect against abuse
- Prevent resource exhaustion
- Fairer resource allocation

**Recommendation:** ⭐⭐⭐⭐ Important for public APIs

---

## Priority Implementation Order

### Phase 1: Essential (Do Now)

1. ✅ **Database Indexes** - Immediate query performance boost
2. ✅ **Request Validation** - Security and data quality
3. ✅ **Observability/Metrics** - Visibility into production

### Phase 2: High Impact (Do Soon)

4. ⭐⭐⭐⭐ **Batch Operations** - Support bulk use cases
5. ⭐⭐⭐⭐ **Cursor Pagination** - Scale to large datasets
6. ⭐⭐⭐⭐ **Redis Caching** - Massive read performance gains

### Phase 3: Optimization (Do Later)

7. ⭐⭐⭐ **gRPC Compression** - Bandwidth savings
8. ⭐⭐⭐ **Streaming Optimizations** - Memory efficiency
9. ⭐⭐⭐ **Rate Limiting** - API protection

### Phase 4: Advanced (Nice to Have)

10. ⭐⭐ **Connection Pooling** - Minor concurrency boost

---

## Expected Overall Impact

**If all Phase 1-2 improvements are implemented:**

| Metric | Current | Projected | Improvement |
|--------|---------|-----------|-------------|
| **Cold Cache GET** | 12,786 req/sec | 15,000-18,000 req/sec | +20-40% |
| **Hot Cache GET** | 12,786 req/sec | 50,000-100,000 req/sec | +300-700% |
| **Bulk Operations** | N/A | 20,000-30,000 items/sec | New capability |
| **Deep Pagination** | Slow (offset > 10k) | Fast (constant time) | 10-100x |
| **Database Load** | 100% | 20-40% | -60-80% |

---

## Estimated Development Time

| Improvement | Effort | Impact | ROI |
|-------------|--------|--------|-----|
| Database Indexes | 1 hour | High | ⭐⭐⭐⭐⭐ |
| Request Validation | 4 hours | High | ⭐⭐⭐⭐⭐ |
| Metrics | 4 hours | High | ⭐⭐⭐⭐⭐ |
| Batch Operations | 8 hours | High | ⭐⭐⭐⭐ |
| Cursor Pagination | 6 hours | High | ⭐⭐⭐⭐ |
| Redis Caching | 12 hours | Very High | ⭐⭐⭐⭐⭐ |
| gRPC Compression | 2 hours | Medium | ⭐⭐⭐ |
| Streaming | 6 hours | Medium | ⭐⭐⭐ |
| Rate Limiting | 3 hours | Medium | ⭐⭐⭐ |
| Connection Pool | 4 hours | Low | ⭐⭐ |

**Total Phase 1-2 Time:** ~35 hours (1 week)
**Expected Performance Gain:** 2-5x for typical workloads, 10-100x for cached workloads

---

## Recommendation

**Start with Phase 1 (Database + Validation + Metrics)** as these provide immediate value with minimal complexity. Then evaluate based on actual production usage patterns before implementing Phase 2.

The current performance (12K+ req/sec) is already excellent. Additional optimizations should be driven by real-world requirements rather than premature optimization.
