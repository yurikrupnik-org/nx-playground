# gRPC Guide

gRPC is a high-performance RPC framework using HTTP/2 and Protocol Buffers.

## Communication Patterns

### 1. Unary (Request/Response)

Single request → Single response. Like a function call.

```
Client                     Server
  │                          │
  │──── Request ────────────►│
  │                          │ (process)
  │◄─── Response ────────────│
  │                          │
```

**Proto definition:**
```protobuf
rpc GetById(GetByIdRequest) returns (GetByIdResponse);
rpc Create(CreateRequest) returns (CreateResponse);
rpc Delete(DeleteRequest) returns (DeleteResponse);
```

**Use for:**
- CRUD operations
- Simple queries
- Most API calls
- Authentication/authorization checks

**Rust client example:**
```rust
let response = client.get_by_id(GetByIdRequest { id: "123".into() }).await?;
```

**Rust server example:**
```rust
async fn get_by_id(&self, request: Request<GetByIdRequest>) -> Result<Response<GetByIdResponse>, Status> {
    let id = request.into_inner().id;
    // fetch from db...
    Ok(Response::new(GetByIdResponse { id, title, ... }))
}
```

---

### 2. Server Streaming

Single request → Multiple responses. Server pushes data over time.

```
Client                     Server
  │                          │
  │──── Request ────────────►│
  │◄─── Response 1 ──────────│
  │◄─── Response 2 ──────────│
  │◄─── Response 3 ──────────│
  │◄─── (end) ───────────────│
  │                          │
```

**Proto definition:**
```protobuf
rpc ListStream(ListRequest) returns (stream TaskResponse);
rpc Subscribe(SubscribeRequest) returns (stream Event);
rpc DownloadFile(FileRequest) returns (stream ChunkResponse);
```

**Use for:**
- Large result sets (memory efficient)
- Real-time feeds/notifications
- Log streaming
- Progress updates
- File downloads

**Rust client example:**
```rust
let mut stream = client.list_stream(ListRequest {}).await?.into_inner();

while let Some(task) = stream.message().await? {
    println!("Received: {:?}", task);
}
```

**Rust server example:**
```rust
type ListStreamStream = Pin<Box<dyn Stream<Item = Result<TaskResponse, Status>> + Send>>;

async fn list_stream(&self, request: Request<ListRequest>) -> Result<Response<Self::ListStreamStream>, Status> {
    let tasks = fetch_all_tasks().await;

    let stream = tokio_stream::iter(tasks.into_iter().map(|t| Ok(TaskResponse { ... })));

    Ok(Response::new(Box::pin(stream)))
}
```

---

### 3. Client Streaming

Multiple requests → Single response. Client pushes data, server responds once.

```
Client                     Server
  │                          │
  │──── Request 1 ──────────►│
  │──── Request 2 ──────────►│
  │──── Request 3 ──────────►│
  │──── (end) ──────────────►│
  │                          │ (process all)
  │◄─── Response ────────────│
  │                          │
```

**Proto definition:**
```protobuf
rpc UploadFile(stream ChunkRequest) returns (UploadResponse);
rpc BatchCreate(stream CreateRequest) returns (BatchResponse);
rpc RecordMetrics(stream MetricPoint) returns (AckResponse);
```

**Use for:**
- File uploads
- Batch inserts
- Aggregations (avg, sum, count)
- Collecting metrics/telemetry

**Rust client example:**
```rust
let chunks = vec![
    ChunkRequest { data: chunk1 },
    ChunkRequest { data: chunk2 },
    ChunkRequest { data: chunk3 },
];

let response = client.upload_file(tokio_stream::iter(chunks)).await?;
println!("Uploaded {} bytes", response.into_inner().total_bytes);
```

**Rust server example:**
```rust
async fn upload_file(&self, request: Request<tonic::Streaming<ChunkRequest>>) -> Result<Response<UploadResponse>, Status> {
    let mut stream = request.into_inner();
    let mut total_bytes = 0;

    while let Some(chunk) = stream.message().await? {
        total_bytes += chunk.data.len();
        // write to storage...
    }

    Ok(Response::new(UploadResponse { total_bytes }))
}
```

---

### 4. Bidirectional Streaming

Multiple requests ↔ Multiple responses. Full duplex communication.

```
Client                     Server
  │                          │
  │──── Request 1 ──────────►│
  │◄─── Response 1 ──────────│
  │──── Request 2 ──────────►│
  │──── Request 3 ──────────►│
  │◄─── Response 2 ──────────│
  │◄─── Response 3 ──────────│
  │                          │
```

**Proto definition:**
```protobuf
rpc Chat(stream ChatMessage) returns (stream ChatMessage);
rpc Sync(stream SyncRequest) returns (stream SyncResponse);
rpc GameLoop(stream PlayerInput) returns (stream GameState);
```

**Use for:**
- Chat applications
- Real-time gaming
- Collaborative editing
- Live data synchronization
- Interactive sessions

**Rust client example:**
```rust
let outbound = async_stream::stream! {
    yield ChatMessage { text: "Hello".into() };
    tokio::time::sleep(Duration::from_secs(1)).await;
    yield ChatMessage { text: "World".into() };
};

let response = client.chat(outbound).await?;
let mut inbound = response.into_inner();

while let Some(msg) = inbound.message().await? {
    println!("Server: {}", msg.text);
}
```

**Rust server example:**
```rust
type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatMessage, Status>> + Send>>;

async fn chat(&self, request: Request<tonic::Streaming<ChatMessage>>) -> Result<Response<Self::ChatStream>, Status> {
    let mut inbound = request.into_inner();

    let outbound = async_stream::try_stream! {
        while let Some(msg) = inbound.message().await? {
            // Echo back with modification
            yield ChatMessage { text: format!("Echo: {}", msg.text) };
        }
    };

    Ok(Response::new(Box::pin(outbound)))
}
```

---

## Comparison Table

| Pattern | Request | Response | Connection | Latency | Use Case |
|---------|---------|----------|------------|---------|----------|
| Unary | 1 | 1 | Short | Lowest | CRUD, queries |
| Server stream | 1 | N | Long | Low | Feeds, downloads |
| Client stream | N | 1 | Long | Medium | Uploads, batching |
| Bidirectional | N | N | Long | Varies | Chat, real-time |

---

## When to Use Each Pattern

```
Need immediate single response?
├── Yes → Unary
└── No → Who sends multiple messages?
         ├── Server only → Server Streaming
         ├── Client only → Client Streaming
         └── Both → Bidirectional Streaming
```

### Decision Guide

| Scenario | Pattern |
|----------|---------|
| Get user by ID | Unary |
| Create/Update/Delete | Unary |
| List 10 items with pagination | Unary |
| List 10,000 items | Server Streaming |
| Real-time notifications | Server Streaming |
| Upload large file | Client Streaming |
| Batch insert 1000 records | Client Streaming |
| Chat application | Bidirectional |
| Live collaboration | Bidirectional |
| Game state sync | Bidirectional |

---

## Error Handling

### Status Codes

| Code | When to Use |
|------|-------------|
| `OK` | Success |
| `INVALID_ARGUMENT` | Bad request data |
| `NOT_FOUND` | Resource doesn't exist |
| `ALREADY_EXISTS` | Duplicate creation |
| `PERMISSION_DENIED` | Auth failed |
| `UNAUTHENTICATED` | No/invalid credentials |
| `RESOURCE_EXHAUSTED` | Rate limit, quota |
| `INTERNAL` | Server bug |
| `UNAVAILABLE` | Service down (retry) |
| `DEADLINE_EXCEEDED` | Timeout |

### Rust Example

```rust
use tonic::Status;

// Return errors
Err(Status::not_found(format!("Task {} not found", id)))
Err(Status::invalid_argument("Title cannot be empty"))
Err(Status::internal("Database connection failed"))

// Handle errors (client)
match client.get_by_id(request).await {
    Ok(response) => { /* success */ }
    Err(status) => match status.code() {
        tonic::Code::NotFound => { /* handle 404 */ }
        tonic::Code::Unauthenticated => { /* redirect to login */ }
        _ => { /* generic error */ }
    }
}
```

---

## Metadata (Headers)

### Setting Metadata (Client)

```rust
use tonic::metadata::MetadataValue;
use tonic::Request;

let mut request = Request::new(GetByIdRequest { id: "123".into() });
request.metadata_mut().insert("authorization", "Bearer token".parse()?);
request.metadata_mut().insert("x-request-id", "req-456".parse()?);

let response = client.get_by_id(request).await?;
```

### Reading Metadata (Server)

```rust
async fn get_by_id(&self, request: Request<GetByIdRequest>) -> Result<Response<GetByIdResponse>, Status> {
    let auth = request.metadata().get("authorization")
        .ok_or_else(|| Status::unauthenticated("Missing auth header"))?
        .to_str()
        .map_err(|_| Status::invalid_argument("Invalid auth header"))?;

    // validate auth...

    Ok(Response::new(GetByIdResponse { ... }))
}
```

---

## Interceptors (Middleware)

### Client Interceptor

```rust
use tonic::service::Interceptor;

#[derive(Clone)]
struct AuthInterceptor {
    token: String,
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        Ok(request)
    }
}

// Usage
let channel = Channel::from_static("http://[::1]:50051").connect().await?;
let client = TasksServiceClient::with_interceptor(channel, AuthInterceptor { token: "xxx".into() });
```

### Server Interceptor

```rust
use tower::ServiceBuilder;
use tonic::transport::Server;

Server::builder()
    .layer(
        ServiceBuilder::new()
            .layer(tonic::service::interceptor(|req: Request<()>| {
                // logging, auth, etc.
                Ok(req)
            }))
    )
    .add_service(TasksServiceServer::new(service))
    .serve(addr)
    .await?;
```

---

## Service Boundary Rules (required)

gRPC gives you a *wire contract*, not a *boundary*. A tonic server that shares a crate
and a database with its caller is a monolith with extra latency. Before shipping any
service on this transport, all of these must hold — see the full checklist in
[`modular-monolith-architecture.md`](./modular-monolith-architecture.md#then-the-boundary-checklist).

- **Contract-only client dependency.** The caller depends on the generated `rpc::*`
  types and a contract crate — never on the service's domain/entity/repository crate.
  The caller's HTTP→gRPC handlers live in the caller's app.
- **The service owns its tables.** The caller holds no DB grants on them. Cross-service
  references are plain ID columns, never foreign keys.
- **Authenticate the hop; never trust identity in the payload.** Tenant/user scope must
  be derived from a verified token, not read from a request field. Forward the caller's
  access token as `authorization` metadata and verify it with `oidc_auth::OidcVerifier`
  (JWKS/RS256). A `bytes org_id` field the server trusts means anyone who can reach the
  port can read any tenant's data.
- **Additive-only proto evolution.** Never renumber or remove a field, never rename a
  package — both force lockstep deploys. Deprecate, ship, then remove in a later release.
  When a field genuinely must go, `reserved` its number **and** its name so the tag can
  never be silently reused with a different meaning:

  ```protobuf
  message GetByIdRequest {
    bytes id = 1;
    reserved 2;              // was org_id - identity is no longer a request field
    reserved "org_id";
  }
  ```

  `tasks.proto` is the worked example: the Phase 4 release removed the identity fields
  from every request and reserved all of them. That was the last breaking change; from
  there the policy is enforced by the reservations themselves.
- **Don't gate the caller's readiness on the callee.** Degrade that service's routes to
  503; keep everything else serving.
- **Set per-call deadlines.** Channel-level timeouts are a backstop, not a policy.

> Worked example, including the mistakes we made:
> [`adr-tasks-service-boundary.md`](./adr-tasks-service-boundary.md).

## This Project

### Proto Location
```
manifests/grpc/proto/apps/v1/tasks.proto
```

### Generated Code
```
libs/rpc/src/generated/tasks/v1/tasks.v1.rs        # Message types
libs/rpc/src/generated/tasks/v1/tasks.v1.tonic.rs  # Client & Server
```
Regenerate with `just proto` (buf: format → lint → build → generate → `cargo check -p rpc`).
Hand-maintained `mod.rs` files wire the generated files into the crate.

### Services

**tasks.v1.TasksService** (`manifests/grpc/proto/apps/v1/tasks.proto`):
```protobuf
service TasksService {
  rpc Create(CreateRequest) returns (CreateResponse);           // Unary
  rpc GetById(GetByIdRequest) returns (GetByIdResponse);        // Unary
  rpc DeleteById(DeleteByIdRequest) returns (DeleteByIdResponse); // Unary
  rpc UpdateById(UpdateByIdRequest) returns (UpdateByIdResponse); // Unary
  rpc List(ListRequest) returns (ListResponse);                 // Unary
  rpc ListStream(ListStreamRequest) returns (stream ListStreamResponse); // Server Streaming
}
```

Note: the package is `tasks.v1`. An earlier unversioned `tasks` package was generated
into the crate but never regenerated from a checked-in proto — always confirm the code
you import comes from `manifests/grpc/proto/`.

### Running

```bash
# Start gRPC server
cargo run -p zerg_tasks

# Start HTTP API (gRPC client)
cargo run -p zerg_api
```
