---
name: service-transport
description: Choose and wire a communication surface between processes — NATS JetStream topics/event architecture, gRPC, or plain HTTP/REST (plus Postgres NOTIFY for browser realtime). Use when adding a stream/subject/consumer/worker, a proto service or gRPC client, a REST endpoint or outbound HTTP call, or when asked how service A should talk to service B.
---

# Service transport: events, gRPC, HTTP

One broker (NATS JetStream), one proto module (`manifests/grpc`), one HTTP toolkit
(`libs/core/axum-helpers`). Never introduce a second of any — Kafka and RabbitMQ are
`status = "rejected"` in `docs/tooling/registry.toml`, and adding a tool needs
`skill://cncf-manager`.

## 0. Pick the shape first

| Need | Transport | Reference |
|---|---|---|
| Caller must not wait; work must survive a crash; a second reader may want the fact later | **NATS JetStream** | §1 |
| Typed call between two of OUR processes, low latency, streaming | **gRPC** | §2 |
| Browsers, third parties, anything with a URL a human types | **HTTP/REST** | §3 |
| "Every committed write reaches every open browser, whoever wrote it" | **Postgres NOTIFY** | §1b |
| Both sides in the SAME process | none — call the function |

Decision prose: `docs/communication-and-consistency.md` §2/§4, `docs/grpc.md`
("gRPC is a wire contract, not a boundary"), `docs/todo-delivery-options.md`
(measured JSON 247 B vs protobuf 110 B per todo).

Two transports over the same data is fine (`todo_api` publishes `todos.>` for the
worker AND NOTIFY→SSE for browsers) — but **one source per consumer**. Never add a
second event source for the same data (`docs/realtime-todo.md`).

Crossing a service boundary means the payload type lives in its own `scope:shared`
crate (`libs/contracts/<name>`), never in the other vertical's domain crate:
`just boundaries` rejects `scope:tasks → scope:zerg` (`tools/nx/scope-tags.ts`,
`tools/nx/check-boundaries.ts`).

## 1. Event topic — NATS JetStream

Everything goes through `libs/core/messaging` (`messaging`, non-default feature):
`messaging = { workspace = true, features = ["nats"] }`. Never `async_nats::connect`
directly; never hand-write a `jetstream::stream::Config`.

**Decide `StreamKind` before there is traffic — retention cannot be changed in place**
(`libs/core/messaging/src/nats/config.rs`):

- `JobQueue` → `WorkQueue` retention. One consumer group; NATS *refuses* a second, so
  duplicate processing is impossible. Work that must happen once (send an email).
- `EventLog` → `Limits` retention. Independent groups each read everything; a group
  added tomorrow replays today. Domain facts (`TodoCreated`, `ProjectDeleted`).
  `Interest` retention is NOT the event-log answer — it drops messages published while
  no consumer exists.

Naming, as actually used (`EMAILS`/`TODOS`/`PROJECTS`):
stream `SCREAMING_PLURAL`, subject space `plural.>`, concrete subject `plural.<fact>`,
DLQ `<STREAM>_DLQ`, consumer group kebab-case service role (`email-worker`,
`todo-worker`, `tasks-project-refs`).

### Publisher

1. Payload + stream constants. Same vertical → `libs/domains/<x>/src/events.rs`;
   crossing processes → `libs/contracts/<x>/src/lib.rs` (copy
   `libs/contracts/projects`: `X_STREAM`, `X_SUBJECT`, `X_DLQ`, `X_KIND`, and
   `impl Event { const SUBJECT }`). **The consumer group name does not belong in the
   contract** — it belongs to the reading service.
2. `impl messaging::Job` with a stable `job_id()` (`Uuid::now_v7()` at construction)
   and a `job_type()`. There is deliberately no payload retry counter — the server's
   `delivery_count` is authoritative (`libs/core/messaging/src/job.rs`).
3. Publisher: copy `libs/domains/projects/src/nats.rs` — `get_or_create_stream(
   stream_config_for(STREAM, SUBJECT, KIND))` then `producer.send_to(Event::SUBJECT,
   &event)`. `stream_config_for` is the ONLY stream builder (`nats/consumer.rs`);
   `get_or_create_stream` does not reconcile, so whoever creates it first wins —
   a hand-rolled config silently mismatches the consumer.
4. Keep a publisher trait + `Noop*` impl in the domain (`TodoEventPublisher` /
   `NoopTodoPublisher`) as a *required* constructor argument, so NATS-less binaries
   and tests have a real substitute.
5. Binary: one shared `messaging::nats::jetstream_with_retry(&config.nats_url, None)`;
   degrade to the Noop publisher with `warn!` if unreachable. Publish failure inside a
   service method is `warn!`-and-continue unless the message must not be lost — there
   is no outbox (commit-then-publish can lose a message; `docs/architecture-backlog.md`).

### Consumer / worker

1. Declare *this service's* view: `impl StreamConfig` reusing the contract's
   stream/subject/DLQ/kind with your own `CONSUMER_NAME`
   (`apps/zerg/tasks/src/project_events.rs:36-47`). **Never suffix the durable name
   per process** — that silently turns a work queue into fan-out and the symptom is
   duplicated side effects in prod. Explicit opt-in for genuine broadcast:
   `WorkerConfig::with_per_instance_broadcast()` (cache invalidation only, never
   side-effecting work).
2. `impl Processor<J>`: classify failures deliberately —
   `ProcessingError::transient` (DB unreachable; a DLQ'd correction would leave data
   broken forever), `permanent` (bad data), `rate_limited` (upstream 429). Budgets in
   `libs/core/messaging/src/error.rs`: 3×1s→30s / 0 / 5×5s→120s.
3. Make processing idempotent: at-least-once is the contract and nothing sets
   `Nats-Msg-Id` / `duplicate_window` today.
4. Bootstrap — copy `apps/todo/worker/src/lib.rs::run` (standalone) or
   `apps/zerg/tasks/src/server.rs:125-169` (embedded in an API binary):
   `init_metrics()` → `jetstream_with_retry` → `WorkerConfig::from_stream::<S>()
   .with_health_port(p)` → `HealthServer::new(p).with_metrics(handle)` spawned →
   `NatsWorker::<J,_>::new(js, processor, cfg).await?.with_health_state(state)
   .run(shutdown_rx)`. Stream + durable consumer are created by
   `ensure_stream`/`ensure_consumer`. Shutdown: `watch::channel(false)` + SIGINT/SIGTERM.
5. Never ack an undeserializable message — on a WorkQueue stream ack deletes it. The
   worker's poison path (`move_poison_to_dlq` then `term`) already does the right thing.
6. Deploy: `[workload] service = false` for a pure consumer (`apps/todo/worker/butler.toml`),
   health port + `prometheus = true`, `NATS_URL` env, and for prod queue-depth scaling
   `[env.prod.workload.scaler]` with a `nats-jetstream` trigger naming stream+consumer
   (`apps/zerg/email-nats/butler.toml`). Then §4.
7. Test: `test-utils = { workspace = true, features = ["nats"] }` → `TestNats::new()`
   (testcontainers, docker required). Publish through the real producer, run the real
   `NatsWorker` with a deadline, assert the side effect —
   `apps/zerg/tasks/tests/project_refs_it.rs`; kind semantics in
   `libs/core/messaging/tests/stream_kind_it.rs`.

Broker locations: compose `nats:2.10-alpine --jetstream -m 8222` → `nats://localhost:4222`;
zerg in kind → `zerg-shared-config` ConfigMap `nats://nats.dbs.svc.cluster.local:4222`;
todo in kind → `manifests/kustomize/todo/nats.yaml` (Service `nats:4222`). There is **no
`[[tilt.portForward]]` for nats**, so a host `nats` CLI talks to compose, not the cluster.
`just email-*` (`apps/zerg/email-nats/email.just`) are the live gates: `email-replica-check`
(one delivery per job across replicas), `email-scale-check` (no dupes/loss at 2000 jobs),
`email-consumers-clean` (durables leak across restarts).

## 1b. Postgres NOTIFY (browser realtime only)

No broker, no backlog, no replay, no cross-service delivery. Path:
trigger `todos_notify` (`manifests/db/todo/migrations/20260828133022_todo_notify.up.sql`)
→ `libs/domains/todo/src/db_events.rs` → `tokio::broadcast` → SSE/WS
(`apps/todo/api/src/events.rs`) and gRPC `Watch` (`grpc.rs`).

- Payload is `{kind,id}` only — NOTIFY caps at 8000 bytes; the listener re-reads the row.
- Invalidate the cache *before* hydrating; the notification IS the invalidation signal.
- NOTIFY is transactional (fires on COMMIT) and row-level (N rows ⇒ N events).
- A malformed payload must `warn!` and continue, never kill the listener.
- No backlog ⇒ clients refetch the list on every (re)connect and apply events idempotently.
- Migration goes through `skill://db-migration`; test by writing **raw SQL** in the test
  so it proves the DB is the source (`libs/domains/todo/tests/db_events_it.rs`).

## 2. gRPC

Contract in `manifests/grpc/proto/apps/v1/<name>.proto` (all files live in `apps/v1/`
regardless of package; `buf.yaml` excepts `PACKAGE_DIRECTORY_MATCH`). Generated code is
committed to `libs/rpc/src/generated` by two **remote** buf plugins (`buf` on PATH +
network; no `build.rs`, no local protoc) — never hand-edit it.

1. `package <name>.v1;`, `option go_package = "grpc/<name>/v1";`. Wire policy is
   additive-only: `breaking: use: [FILE]` — never renumber or remove a field, `reserved`
   both the tag number and the name (`tasks.proto` is the worked example).
2. `just proto-fmt && just proto-lint && just proto-build`, then `just proto-gen`.
3. **Hand-write the module chain buf does not emit** (there is no `prost-crate` plugin,
   and the files lie with a `// @generated` header): `libs/rpc/src/generated/<name>/mod.rs`
   = `pub mod v1;`, `<name>/v1/mod.rs` = `include!("<name>.v1.rs");` (the prost file
   already includes the `.tonic.rs`), plus `pub mod <name>;` in `generated/mod.rs`.
   Skip this and `cargo check -p rpc` still passes while your service is unreachable —
   `agent/v1` and `users/v1` are orphaned in the tree right now.
4. `just proto-check` (`cargo check -p rpc`); confirm
   `rpc::<name>::v1::<name>_service_server::SERVICE_NAME` resolves.
5. Server — pick a shape:
   - **Merged into an existing axum app** (no new port/binary, h2c on `:8080`): copy
     `apps/todo/api/src/grpc.rs`. `Routes::from(axum::Router::new())` — NOT
     `Routes::default()`, whose `UNIMPLEMENTED` fallback swallows unknown HTTP paths —
     `.add_service(health_service).add_service(svc).into_axum_router()`, and `.merge(grpc)`
     **after** CORS/`TraceLayer::new_for_http` (a preflight is meaningless for h2c and an
     HTTP layer reshapes tonic status codes). Register
     `tonic_health` + `set_service_status(SERVICE_NAME, Serving)`.
   - **Standalone binary**: copy `apps/zerg/vector/src/server.rs` (57 lines) —
     `grpc-client = { workspace = true, features = ["server"] }`,
     `ServerConfig::from_env()` (`GRPC_HOST` default `::1`, `GRPC_PORT` 50051,
     `GRPC_COMPRESSION`, `GRPC_MAX_MESSAGE_SIZE`; unparseable = hard error),
     `GrpcServer::serve_with_shutdown`. Set `GRPC_HOST = "0.0.0.0"` / `GRPC_PORT` in
     `[config]` like `apps/zerg/tasks/butler.toml`.
6. Map domain errors → `Status` explicitly (`to_status` in `apps/todo/api/src/grpc.rs`):
   log internals, never forward them. Streams map `Lagged(n)` →
   `Status::resource_exhausted` and end; the client re-lists and re-watches.
7. Identity comes from the verified bearer token in `authorization` metadata
   (`oidc_auth::OidcVerifier`), never from a request field.
8. Client: copy `apps/zerg/api/src/grpc_pool.rs` —
   `grpc_client::create_channel_lazy(addr)` (no I/O at construction, auto-reconnect),
   `with_interceptor(channel.clone(), TracingInterceptor::new())` + Zstd + 8 MB limits,
   a bare `HealthClient::new(channel)` for readiness. The interceptor changes the type:
   store it as `Client<TracedChannel>`. Address from env with an
   `http://[::1]:<port>` default, in-cluster value in `[config]`
   (`TASKS_SERVICE_ADDR = "http://zerg-tasks:50051"`). Never gate your own readiness on
   the callee — degrade just those routes (`apps/zerg/api/src/api/health.rs`). Map back
   with `impl From<tonic::Status> for ApiError` (`apps/zerg/api/src/error.rs`).
9. Probes: `type = "grpc"` in `butler.toml` renders kubelet's native `grpc:` probe and
   requires `grpc.health.v1.Health` registered. There is no `GRPCRoute` anywhere —
   gRPC is east–west ClusterIP only.
10. Prove it by serving REST+gRPC on an ephemeral port and connecting a real client
    (`apps/todo/api/src/grpc.rs` `serve()` + `crud_round_trip_over_grpc_is_visible_over_rest`).
    Gate: `just proto` then `just proto-gen && git diff --exit-code`; `just proto-breaking`
    before pushing (fetch `main` first — it is a no-op while standing on `main`, and CI
    runs it PR-only).

`just check` contains NO proto step. CI never runs `proto-gen` or diffs generated output.

## 3. HTTP / REST

Two mutually exclusive router shapes — use the vertical's existing one:

- `apps/todo/api` — hand-built `Router`, `create_permissive_cors_layer()` (dev-only),
  `create_app(app, &config.server)` (binds + `shutdown_signal()`), hand-rolled
  `/healthz`.
- `apps/zerg/api` / terran — `create_router::<ApiDoc>(api::routes(&state))`, which
  **requires `CORS_ALLOWED_ORIGIN`** (startup error otherwise), force-nests under
  `/api`, mounts swagger/redoc/rapidoc/scalar, plus `health_router` (`/health`),
  app-owned `ready_router` (`/ready`), and `create_production_app(..., 30s, cleanup)`
  when the service owns pooled connections.

Endpoint on an existing domain (canonical path, `libs/domains/todo`):

1. DTO in `models.rs`: `Serialize, Deserialize, Validate, ToSchema, TS` + `#[ts(export)]`,
   `#[ts(as = "String")]` for `Uuid`/`DateTime<Utc>`.
2. Service method in `service.rs` returning `TodoResult<T>`; repository if it stores.
3. New failure mode → variant in `error.rs` + arm in `impl From<X> for AppError`.
   `impl_into_response_via_app_error!` stays as-is.
4. Handler in `handlers/direct.rs` with `#[utoipa::path(...)]`, `State<Arc<Service>>`,
   `UuidPath`/`ValidatedJson<T>` extractors (structured 400 before the body runs).
5. Register in `handlers/mod.rs`: `pub use`, `paths(...)`, `components(schemas(...))`,
   and the `.route()` line — axum 0.8 path syntax is `{id}`, not `:id`.
6. Regenerate: `cargo test -p domain_todo export_openapi` (→ `docs/openapi/todos.v1.json`)
   and `export_bindings` (→ `libs/domains/todo/types/*.ts`), then **add the new type to
   `types/index.ts` by hand** — the barrel is not generated. Gates:
   `bun nx run domain_todo:ts-gate`, `bun nx run zerg_api:openapi-gate`
   (`todos.v1.json` has no gate; regenerate it deliberately).

Outbound call (only in-repo example: `apps/todo/web-htmx/src/api.rs` → todo-api):

1. Base URL as an `AppConfig` field via `env_or_default("<X>_API_URL", "http://127.0.0.1:<port>")`
   — all env reads live in `from_env`, nowhere else (`libs/core/config`, `FromEnv`).
   In-cluster value as `[[workload.env]] value = "http://<service>:8080"`.
2. `#[derive(Clone)] struct XApi { http: Client, origin: String }` with
   `origin.trim_end_matches('/')`, a private `url()`, an `expect_ok` folding non-2xx into
   `eyre!("<svc> {status}: {body}")`, `.wrap_err("<svc> unreachable")` on `send()`, and
   `.timeout(..)` on anything on a render path. Construct once in `main`, `.with_state`.
3. Type payloads against the publisher's contract/domain crate, not a re-declared struct.
4. Decide explicitly whether the call may degrade (`flags_or_defaults`) or must fail —
   upstream failure maps to `502` (`upstream_error` in `web-htmx/src/main.rs`).
5. Retry only idempotent GETs, via `core_retry::retry_with_backoff` (no HTTP caller uses
   it today).

North–south exposure: prod `HTTPRoute` as `[[env.prod.workload.extraManifests]]` parented
to `main-gateway`/`gateway` (`apps/zerg/web/butler.toml`) — that Gateway is owned by
gitops-v1, never defined here. Dev routes in `manifests/k8s/base/gateway/*` are
hand-written and ungated.

## 4. Wiring any new surface (all three transports)

`<app>/butler.toml` is the only hand-written deploy fact; Tiltfiles and manifests are
generated (AGENTS.md).

1. `[workload]` presence is the deployability predicate — no `[workload]`, no
   `tilt-gen`/`k8s-gen`/`container`/`scan` targets. State only what differs from
   `[workloadDefaults.<service|web|node>]`.
2. **One port per workload.** `Workload` has a single `port` + `portName`; a second
   listener means a second app or a hand-written Service in `[[workload.extraManifests]]`.
   gRPC on one port with HTTP is the merged-h2c trick (§2), not two ports.
3. Never write `image` in any `[workload]` — hard error; butler injects it.
   `[config]` is `{string:string}` — `FLAG = true` fails, quote it. Env overlays
   **replace arrays wholesale**: changing one `envFrom` entry means restating the list.
4. `[tilt] hostPort` only (container side derived from `port`); must not collide —
   in use: 5206, 5221, 5230, 5231, 5252, 5253, 50051, infra 5432/6379/8025.
5. Probes must match the code (`/healthz` vs `/health` vs `type = "grpc"`), and
   `prometheus = true` only if the process really serves `/metrics`.
6. Caller addresses callee by same-namespace short DNS in `[config]`
   (`http://<workload-name>:<port>`).
7. Regenerate and commit: `just tilt-gen`, `just k8s-gen` (per-app:
   `just tilt-gen-app <project>` / `just k8s-gen-app <project>`, then one full run for the
   root artifacts). Rendered manifests land in `[k8s] outDir` — read the key, do not
   assume `manifests/k8s/apps`.
8. New crate → `skill://new-rust-crate` (workspace members list, `{ workspace = true }`
   deps).

## 5. Gates

```
cargo check -p <crate>            # fastest loop
cargo nextest run -p <crate>      # docker up for testcontainers
just check                        # fmt-check lint test audit — no proto, no drift gates
just tilt-check k8s-check container-check boundaries   # drift; CI runs only `boundaries`
just proto                        # gRPC only: fmt→lint→build→gen→cargo check -p rpc
just verify                       # pre-push: check + proto + all drift gates + scan + e2e
```

Never run whole-workspace Rust through `nx run-many` (AGENTS.md). CI does not run
`tilt-check`/`k8s-check`/`container-check` — catch drift locally before pushing.
