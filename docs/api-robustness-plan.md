# API Robustness Plan

- **Status:** Proposed
- **Date:** 2026-09-18
- **Scope:** the HTTP surface of the demo APIs — `todo_api` (`apps/todo/api`, the
  reference vertical), `zerg_api`, `terran_api`, and the shared
  `libs/core/axum-helpers` they all build on.
- **Related:** [`architecture-backlog.md`](./architecture-backlog.md) (messaging /
  consistency work — outbox, publish idempotency, sagas — **not repeated here**),
  [`communication-and-consistency.md`](./communication-and-consistency.md),
  [`todo-delivery-options.md`](./todo-delivery-options.md),
  [`realtime-todo.md`](./realtime-todo.md)

This is the **request/response contract** counterpart to the architecture backlog:
what an HTTP client can rely on when it retries, edits concurrently, paginates, or
gets an error. Each item gives the **problem** (with the line it lives on), the
**fix**, and an **acceptance test** — an item is not done until the test would fail
if the fix were reverted.

Size: **S** ≤ half a day · **M** 1–2 days · **L** > 2 days.

Sequencing rule: everything in P0/P1 lands in `todo_api` **first** (it is the demo
vertical and it has e2e coverage in `apps/todo/e2e`), then moves into
`axum-helpers` so `zerg_api`/`terran_api` inherit it. A pattern that only exists in
one app is a demo; a pattern in `axum-helpers` is the repo's answer.

---

## What already exists — do not rebuild

Worth stating up front, because several "senior REST patterns" are already here and
the gap is **wiring, not writing**:

| Capability | Where | Status in `todo_api` |
|---|---|---|
| Distributed rate limiter (Redis sliding window, per-tier; emits `Retry-After` + `X-RateLimit-*`, fails **open** when Redis is down) | `libs/core/axum-helpers/src/rate_limit/` | **not wired** (zerg uses it; terran does not either) |
| RED metrics + `/metrics` + pool gauges | `libs/core/axum-helpers/src/metrics.rs` | **not wired** |
| Security headers middleware | `libs/core/axum-helpers/src/http/security.rs` | **not wired** |
| Structured error codes + `ErrorResponse` | `libs/core/axum-helpers/src/errors/` | used via `TodoError` → `AppError` |
| `ValidatedJson` extractor | `libs/core/axum-helpers/src/extractors/validated_json.rs` | **not used** (service calls `validate()`) |
| Graceful shutdown / readiness / cleanup | `libs/core/axum-helpers/src/server/` | only `create_app` (no `/ready`) |
| Served API docs (Swagger UI, Redoc, RapiDoc, Scalar) + strict CORS + compression | `create_router` in `libs/core/axum-helpers/src/server/app.rs:121` | **not used** — the spec is exported to a file and served by nothing |
| Sparse fieldsets + role-gated projection | `libs/core/field-selector` (+ `@native/field-selector`) | used only by the Astro proxy |
| Retry with backoff + jitter | `libs/core/retry` | used by DB/gRPC clients only |
| W3C trace context **injection** (gRPC client side) | `libs/core/grpc/src/interceptors/tracing.rs` | outbound only |
| DB-sourced realtime (NOTIFY → SSE/WS/gRPC `Watch`) | `libs/domains/todo/src/db_events.rs` | done, see `realtime-todo.md` |

`todo_api`'s whole middleware stack today is three lines —
`create_permissive_cors_layer()` + `TraceLayer` + a string `/healthz`
(`apps/todo/api/src/main.rs:123-128`). That is the single biggest lever in this doc.

---

## P0 — Correctness defects (a client can lose data or see a leak today)

### 0.1 Lost updates: no optimistic concurrency · M

**Problem.** `PgTodoRepository::update` is a read-modify-write with a gap: it loads
the row (`libs/domains/todo/src/postgres.rs:72`), mutates the active model, then
writes (`:94`). Be precise about what is and is not lost: sea-orm `Set`s only the
fields the DTO carried (the comment at `:76-78` is right that two PATCHes to
*different* columns both survive). What **is** lost is (a) two writers on the
**same** column — the second silently wins — and (b) every stale-view edit: user A
saves an edit of a todo that B completed a second ago and A's stale `completed:
false` un-completes it. There is no `version` column, no `ETag` on reads, and no
`If-Match` on writes (`grep -ri etag apps libs` → 0 hits in handler code). The
realtime bus makes (b) **more** likely, not less: every browser has a live list
and an edit box over the same rows.

**Fix.**

1. Migration: `ALTER TABLE todos ADD COLUMN version BIGINT NOT NULL DEFAULT 1;`
   (Atlas versioned mode — use the `db-migration` skill). Bump it in the same
   `UPDATE … WHERE id = $1 AND version = $2` statement so the check is atomic in
   the database, not in Rust.
2. Repository: `update(id, input, expected_version) -> TodoResult<Todo>`; 0 rows
   affected + row exists ⇒ `TodoError::Conflict`.
3. HTTP: emit `ETag: "<version>"` on `GET /{id}` (and on every 200/201 write
   response); accept `If-Match` on `PUT`/`PATCH`/`DELETE`. Missing header ⇒ 428
   Precondition Required (or last-write-wins behind a config flag for the demo);
   stale header ⇒ **412 Precondition Failed**.
4. Add `ErrorCode::PreconditionFailed` to `libs/core/axum-helpers/src/errors/codes.rs`.
5. gRPC parity: `UpdateRequest` has no version field today
   (`manifests/grpc/proto/apps/v1/todo.proto:80-86`) — add `optional int64
   version = 6`, regenerate (`buf` → `libs/rpc/src/generated`, never hand-edit),
   and map a mismatch to `Code::Aborted`. The two transports share one service,
   so one of them silently skipping the check is a worse bug than not having the
   check at all.
6. Evaluate the precondition **in the `UPDATE` statement**, never against the
   row `CachedTodoRepository` hands back (`libs/domains/todo/src/cache.rs`): the
   id key has a 30s TTL and external writes reach it only after the NOTIFY
   listener's `invalidate`, so a cached copy can be a few ms stale. Serving the
   `ETag` from the cache is fine — a stale one simply earns a 412.
7. Free win once the header exists: honour `If-None-Match` on `GET /{id}` and
   answer 304 — the cheapest read-path optimisation in the doc, zero extra state.

**Acceptance test.** An integration test issues two updates from the same `ETag`;
the first returns 200 with a new `ETag`, the second returns 412 and the row still
holds the first writer's value. Same assertion over gRPC (`Code::Aborted`).

---

### 0.2 Error responses are not one shape — and one of them leaks · S

**Problem.** Two distinct holes in the "every error is an `ErrorResponse`" promise:

1. **Leak.** `apps/todo/api/src/stacks.rs:46` returns the raw `sea_orm` error to
   the caller: `.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))`.
   A `DbErr` stringifies to SQL text, column names, and sometimes connection
   details — as `text/plain`.
2. **Shape.** Every handler takes a bare `axum::Json<T>`
   (`libs/domains/todo/src/handlers/direct.rs:54`, `:70`). A malformed body,
   wrong `Content-Type`, or a body that fails to deserialize gets axum's built-in
   rejection: `text/plain`, status 400/415/422, no `code`, no `error`. The
   `AppError::JsonExtractorRejection` arm built for exactly this
   (`libs/core/axum-helpers/src/errors/mod.rs:72`, `:150`) is **unreachable** —
   nothing in the repo uses `#[from_request(via(Json), rejection(AppError))]` or
   `WithRejection` (`grep -rn 'rejection(' apps libs` → 0). `ValidatedJson` and
   `UuidPath` have the same gap one level down: both pass the inner extractor's
   rejection through untouched (`validated_json.rs:51`, `uuid_path.rs:39`).

A generated client (2.2) therefore cannot rely on the error type even for the
most common client error there is.

**Fix.** (1) Return `AppError` from `stacks.rs` (the `TodoError::Database` arm
already logs server-side and answers generically — mirror it) and sweep:
`grep -rn 'e.to_string()' apps libs --include='*.rs' | grep -i 'INTERNAL\|StatusCode'`.
(2) Add an `AppJson<T>` in `axum-helpers::extractors` —
`#[derive(FromRequest)] #[from_request(via(axum::Json), rejection(AppError))]` —
route `ValidatedJson`/`UuidPath` inner rejections through `AppError`, and swap the
handlers. `AppError::JsonExtractorRejection` then does the work it was written for.

**Acceptance test.** With Postgres stopped, `GET /api/stacks` returns
`application/json` matching `ErrorResponse` with no SQL fragment in `message`.
`POST /api/todos` with body `{` returns 400 `application/json` with
`error: "INVALID_JSON"`; `PUT /api/todos/not-a-uuid` returns 400 in the same shape.

---

### 0.3 Unbounded list limit — one query can pull the table · S

**Problem.** `TodoFilter.limit` is `usize` with a serde default of 50
(`libs/domains/todo/src/models.rs:101-108`) and **no upper bound**.
`GET /api/todos?limit=100000000` reaches `QuerySelect::limit` verbatim
(`postgres.rs:62`). The read cache caps what it *stores* at 1000 rows
(`cache.rs:39`, `LIST_CACHE_CAP`) — but a window past that cap, or any filtered
query, falls straight through to the DB with the limit unclamped (`cache.rs:158-163`),
so the cap protects the KV value, not the query. Same shape in `domain_projects`
(`models.rs:199`) and on the gRPC path (`apps/todo/api/src/grpc.rs:155`). Deep `offset` has the same problem
from the other end: Postgres scans and discards every skipped row.

**Fix.** Clamp in the DTO, not the handler, so every transport inherits it:
`#[validate(range(min = 1, max = 100))] limit`, plus a `MAX_LIMIT` const used by
the gRPC `list` arm. Return the effective limit in the response envelope (0.4) so
a client can tell it was clamped.

**Acceptance test.** `?limit=10000` returns at most 100 items and the envelope
reports `limit: 100`. gRPC `list` with `limit: 10000` returns at most 100.

---

## P1 — Contract gaps (correct today, wrong the moment a client retries)

### 1.1 No idempotency on writes · M

**Problem.** `POST /api/todos` has no `Idempotency-Key`
(`libs/domains/todo/src/handlers/direct.rs:45-58`). A browser on a flaky
connection, a mobile client, or any proxy that retries a timed-out POST creates
duplicate rows — and each duplicate fires a NOTIFY, so every connected UI renders
the duplicate immediately. The repo already treats idempotency as a first-class
concern on the **messaging** side (backlog 0.2, `Nats-Msg-Id` + `duplicate_window`);
the HTTP side has none.

**Fix.** A small `idempotency` module in `axum-helpers` (so zerg's
`POST /api/org` — backlog 5.4 — can reuse it):

- Table `idempotency_keys (key TEXT PRIMARY KEY, request_hash TEXT NOT NULL,
  status_code SMALLINT, response_body JSONB, created_at TIMESTAMPTZ, expires_at
  TIMESTAMPTZ)`. Alternatively the NATS KV bucket already open in `main.rs:81` —
  but prefer Postgres: the key must commit **with** the write, which is the same
  argument the outbox rests on (`communication-and-consistency.md:242`).
- `INSERT … ON CONFLICT DO NOTHING` to claim the key **inside** the write
  transaction. Lost the race + response stored ⇒ replay it with
  `Idempotency-Key-Replayed: true`. Lost the race + still in flight ⇒ 409.
- Same key, **different** body hash ⇒ 422 (Stripe's rule; prevents a key from
  being reused for a different request).
- TTL 24h, swept by the existing worker.

Decide `DELETE` retry semantics at the same time: today a retried `DELETE` of a
row the first attempt already removed gets 404 (`service.rs:90-92`), which a
retrying client reads as failure. Either keep 404 and say so in the OpenAPI doc,
or return 204 for an already-absent row — both are defensible; undocumented is not.

**Acceptance test.** The same `Idempotency-Key` + body POSTed twice yields one row,
two identical 201 bodies, and the second carries the replay header. The same key
with a changed body yields 422.

---

### 1.2 `PUT` has `PATCH` semantics · S

**Problem.** The route is `PUT /{id}` (`libs/domains/todo/src/handlers/mod.rs:53`)
but `UpdateTodo` is all-`Option` and `apply_update` only touches present fields
(`models.rs:85-89`, `:124`) — that is `PATCH`, spelled `PUT`. A client that trusts
the verb and sends a full representation minus one field expects that field
cleared; it stays. The doc-comment on `UpdateTodo` already says "PATCH semantics",
which makes this a naming bug, not a design question.

**Fix.** Register the handler on **`PATCH`** with
`Content-Type: application/merge-patch+json` (RFC 7396) accepted alongside
`application/json`, and keep `PUT` as a deprecated alias for one release
(`Deprecation` + `Sunset` headers, RFC 8594). The blast radius is small: the one
in-repo REST caller of `PUT` is `apps/todo/web/src/lib/todo-api.ts:59`;
`web-astro`'s proxy forwards `request.method` verbatim
(`apps/todo/web-astro/src/pages/api/todos/[...rest].ts:29`) and inherits whatever
the SPA sends; `web-htmx`, the CLI (NATS) and gRPC never issue a `PUT`.

Two things to get right that the article skips:

- **`null` today is a silent no-op, not an error.** `{"description": null}`
  deserializes to `None` and is dropped by the merge, so a client following
  RFC 7396 ("null removes the member") gets 200 and no change. `description` is
  `NOT NULL DEFAULT ''` in the schema, so the honest rule is `null → ""` for
  non-nullable columns (and `null → NULL` for any future nullable one) — not a
  three-state wrapper. Proto3 `optional` on the gRPC side has exactly the same
  presence-only semantics (`todo.proto:82-85`), so the rule belongs in the DTO,
  where both transports meet.
- **The merge rule is spelled in three places** — `Todo::apply_update`
  (`models.rs:124`), the sea-orm `Set` chain in `postgres.rs:80-92`, and a
  reimplementation flagged as such in `apps/todo/temporal/src/lib.rs:108`.
  Collapse to one (`apply_update` over the model, then `into_active_model`)
  **before** changing the semantics, or the three will disagree about `null`.

**Acceptance test.** `PATCH` with `{"title":"x"}` leaves `description` intact;
`PATCH` with `{"description":null}` sets it to `""` over REST **and** an
`UpdateRequest` with an empty `description` does the same over gRPC; `PUT` still
works and returns a `Deprecation` header.

---

### 1.3 List responses are bare arrays — no total, no cursor · M

**Problem.** `list_todos` returns `Json<Vec<Todo>>` (`handlers/direct.rs:25-30`).
A client paginating has no way to know how many pages exist (`count_todos` exists
in the service, `service.rs:131`, and is never exposed), and `offset` pagination
skews whenever a row is inserted between pages — which, with a realtime insert
feed, is constantly.

**Fix.** One shared envelope in `axum-helpers` (every domain list is currently a
bare array, so do this once):

```rust
pub struct Page<T> { pub items: Vec<T>, pub total: Option<u64>, pub limit: usize, pub next_cursor: Option<String> }
```

Keep `offset` working for the demo pages; add a keyset cursor over
`(created_at DESC, id DESC)` — the index `idx_todos_created_at` already exists —
and make the cursor an opaque base64 of that tuple so it can change shape later.
This is a **breaking response change**: do it behind `/api/v2/todos`, or ship it
with the `Deprecation` window from 1.2 and regenerate the ts-rs DTOs so the three
web apps break at compile time rather than at runtime.

**Acceptance test.** `GET /api/todos?limit=2` returns `items.len() == 2`, a
`next_cursor`, and `total`; following the cursor after inserting a new row does
not repeat or skip an item (the offset version does).

---

### 1.4 No request timeout, body limit, or concurrency cap · S

**Problem.** The stack (`main.rs:123-129`) has no `TimeoutLayer`, no
`RequestBodyLimitLayer`, no `ConcurrencyLimitLayer`. `description` is unbounded
`TEXT` with no `#[validate(length)]` (`models.rs:77`, `:87`), so a single POST can carry
an arbitrarily large body into Postgres — and then into the NOTIFY hydration path.
This is the article's "timeouts and bulkheads" pattern, and it is the cheapest
item in the doc.

**Fix.** Add to `axum-helpers::create_app` (so all three APIs inherit):
`TimeoutLayer::new(30s)` (a 408/503 mapping in `AppError`),
`RequestBodyLimitLayer::new(1 MiB)` (413 with the `ErrorResponse` shape),
`ConcurrencyLimitLayer` sized from config. Add
`#[validate(length(max = 10_000))]` to `description` on both DTOs.

**Acceptance test.** A 2 MiB body returns 413 in `ErrorResponse` shape; a handler
that sleeps 60s returns within ~30s.

---

### 1.5 Rate limiting, metrics, security headers, `/ready` are not wired into `todo_api` · S

**Problem.** All four exist and are used by `zerg_api` (`apps/zerg/api/src/lib.rs:36,
137-142, 172-180`), none are used by `todo_api` — which is the app the e2e suite,
the Tilt dev loop, and the demo actually exercise. `/healthz` is a string literal
(`main.rs:123`): it is a liveness probe answering "the process is up", with no
readiness signal for Postgres or NATS, so Kubernetes routes traffic to a pod whose
DB is still connecting.

**Fix.** The cheapest route is to stop hand-rolling the stack and call
`create_router::<TodoApiDoc>` like terran and zerg do (`apps/terran/api/src/lib.rs:141`):
that alone brings security headers, compression, strict CORS (1.6), a JSON 404
fallback, and **serves the OpenAPI spec** at `/swagger-ui`, `/redoc`, `/scalar`
— today `docs/openapi/todos.v1.json` is written by a test and served by nothing.
The gRPC merge order rule from `main.rs:119-122` still holds: merge tonic routes
after the HTTP layers.

Then: `init_metrics()` before the first metric, layer `track_metrics`, merge
`metrics_router` **after** (so `/metrics` is not tracked), merge `health_router`

- a `/ready` that checks the pool (model: `apps/zerg/api/src/api/mod.rs:156`),
and apply `rate_limit_middleware` with an `Extension(RateLimitTier)` on the
sub-router (`api/mod.rs:27-46`). The limiter already emits `Retry-After` and
`X-RateLimit-Limit/Remaining/Reset` and **fails open** when Redis errors
(`rate_limit/middleware.rs:110-135`, `:117-124`) — the only wiring decision is
what to do when Redis is not configured at all, since `todo_api` has no Redis
today: skip the layer with a warning, never fail boot. Prometheus scraping also
needs `prometheus = true` in the app's `butler.toml` `[workload]` (the default is
`false` precisely because most apps 404 on `/metrics` — see CLAUDE.md).

**Acceptance test.** `GET /metrics` renders `http_request_duration_seconds`;
`/ready` returns 503 while Postgres is down; the N+1st request in a window returns
429 with `Retry-After`; `X-Content-Type-Options: nosniff` is on every response.

---

### 1.6 CORS is permissive on `todo_api` · S

**Problem.** `create_permissive_cors_layer()` (`main.rs:127`) is `Any` origin. Fine
for the dev loop, wrong for the deployed dev/prod environments where the same
binary runs. The strict version exists twice and neither is what it looks like:
`create_router` parses `CORS_ALLOWED_ORIGIN` into an origin **list** inline
(`server/app.rs:134-181`), while `create_cors_layer` (`http/cors.rs:17`) is a
single-origin near-duplicate of that block that nothing calls.

**Cross-cutting, and the article never mentions it:** the strict layer's
`allow_headers` is `Content-Type, Authorization, Accept, Cookie, x-csrf-token`
(`app.rs:173-179`) and there is **no `expose_headers` at all**. Every header this
plan introduces — `If-Match`, `Idempotency-Key` (request side), `ETag`,
`X-Request-Id`, `Retry-After`, `X-RateLimit-*`, `Deprecation`, `Sunset` (response
side) — is invisible to a cross-origin browser client until it is listed. In the
Java articles this never bites because Spring apps are same-origin; here the SPAs
are cross-origin in every dev setup.

**Fix.** Delete `create_cors_layer`, lift the inline block into it as the one
list-parsing implementation, add the request headers to `allow_headers` and the
response headers to `expose_headers`, and have `create_router` and `todo_api` both
call it. Select on `config.environment`: permissive in `local`, strict everywhere
else; fail boot if `CORS_ALLOWED_ORIGIN` is unset outside `local`. Add the var to
the app's `butler.toml` `[config]`, since manifests are generated.

**Acceptance test.** With `ENVIRONMENT=production` and no `CORS_ALLOWED_ORIGIN`,
boot fails with a clear error; with it set, a preflight from a foreign origin is
refused, and a preflight naming `If-Match` is accepted; `ETag` appears in
`Access-Control-Expose-Headers`.

---

## P2 — Observability and evolution

### 2.1 Inbound trace context is dropped · S

**Problem.** `init_tracing` installs the W3C `TraceContextPropagator`
(`libs/core/config/src/tracing.rs:60`) and the gRPC client interceptor **injects**
`traceparent` (`libs/core/grpc/src/interceptors/tracing.rs:49`) — but no HTTP
server layer **extracts** it. An inbound `traceparent` from a browser, gateway, or
sibling service starts a fresh trace instead of joining the caller's, so a
cross-service trace is split in two at every HTTP hop. `TraceLayer::new_for_http`
(`main.rs:128`) does not do this on its own.

**Fix.** A `trace_context` middleware in `axum-helpers`: extract the propagator
context from request headers, set it as the parent of the request span, and
generate an `X-Request-Id` when absent. Echo `X-Request-Id` on the response (and
in the `ErrorResponse`, as an optional `request_id` field) so a user can paste the
id from a failed call into a support ticket and it resolves to a trace. Include
`trace_id` in the log format, matching what the gRPC interceptor already sends.

**Acceptance test.** A request carrying a `traceparent` produces a span whose
trace id equals the inbound one; the response carries the same `X-Request-Id` the
request sent, or a generated one when it did not.

---

### 2.2 OpenAPI documents only success responses · S

**Problem.** `docs/openapi/todos.v1.json` contains 5×200, 1×201, 1×204 and
**no error responses at all** — the `#[utoipa::path]` blocks
(`handlers/direct.rs:17-113`) never list 400/404/409/500, and the `ToResponse`
types built exactly for this (`libs/core/axum-helpers/src/errors/responses.rs`) are
not referenced. A generated client therefore has no error type, and anything added
in P0/P1 (412, 422, 429, 413) will be invisible to it.

**Fix.** Add the error responses to every path and register them in
`components(responses(...))` on `TodoApiDoc` (`handlers/mod.rs:23-41`), and
actually serve the document (1.5 — `create_router` mounts Swagger/Scalar for free). The
`export_openapi_todos_v1` test already regenerates the committed document, so this
is a one-file change that self-verifies. Also document the new headers
(`ETag`, `If-Match`, `Idempotency-Key`, `Retry-After`) as parameters.

**Acceptance test.** `cargo nextest run -p domain_todo` regenerates the spec and
`git diff --exit-code docs/openapi` fails if it was not committed (this is already
how the repo gates generated files); the spec lists 404 and 500 on `GET /{id}`.

---

### 2.3 No versioning or deprecation mechanism · M

**Problem.** Routes mount at `/api/todos` with no version segment
(`apps/todo/api/src/main.rs:124`), while the OpenAPI doc calls itself `1.0.0` and
the file is `todos.v1.json`. Three of the items above (1.2, 1.3, and `ETag`
becoming mandatory) are breaking changes with three in-repo web clients plus a
gRPC client. Without a version path and a sunset convention, each one is a
flag-day.

**Fix.** Adopt a repo convention now, before P1 lands: mount under `/api/v1/…`
with `/api/…` as an alias, and on a breaking change add `/api/v2/…` while the v1
route answers with `Deprecation: true` and `Sunset: <http-date>` (RFC 8594).
gRPC already has this for free via the `todo.v1` package — `manifests/grpc/proto/apps/v1/todo.proto`
— so the REST side is the one out of step.

**Acceptance test.** `/api/v1/todos` and `/api/todos` return the same body; a route
marked deprecated carries both headers and a `Link` to the successor.

---

### 2.4 SSE has no resume, no connection cap · M

**Problem.** `sse_handler` (`apps/todo/api/src/events.rs:60-81`) subscribes to a
256-slot broadcast and sends a `lagged` event when a consumer falls behind. A
client that reconnects (laptop sleep, proxy timeout) sends `Last-Event-ID`, which
is ignored — it silently misses every event in the gap, and the only repair is a
full list refetch the client does not know it needs. The WebSocket transport is
worse: on `Lagged` it logs at `debug!` and continues (`events.rs:104-106`), so a
WS client that fell behind is never told — SSE at least emits a `lagged` event.
Nothing caps the number of concurrent subscribers either, so each connection is
an unbounded slot in a broadcast the DB listener must keep feeding.

**Fix.** Give each event a monotonic id (the NOTIFY sequence, or a
`todo_events` append-only table keyed by `(id, occurred_at)`), set `Event::id`,
and on reconnect replay from `Last-Event-ID` — or, when the gap exceeds the
retained window, send a `resync` event telling the client to refetch. Cap
concurrent SSE/WS connections per process and return 503 past the cap. Keep the
DB as the only event source (CLAUDE.md: never add a second source for the same
data). See `realtime-todo.md` for the current design.

**Acceptance test.** A client disconnects, three todos are created, it reconnects
with `Last-Event-ID`; it receives the three missed events (or exactly one
`resync`), never silence. A WS subscriber forced past the channel capacity
receives a `lagged` frame.

---

### 2.5 Sparse fieldsets are not on the REST path · S

**Problem.** `libs/core/field-selector` implements exactly the article's pattern —
`?fields=` projection with role-based gating: a deserializable `FieldSelector`
query type (`lib.rs:186`), `filter_secure`/`filter_list_secure` (`:234`, `:266`),
and an `AuthContext` extractor behind the `axum` feature (`:329`). The only consumer is the Astro proxy via
`@native/field-selector` (`apps/todo/web-astro/src/lib/projection.ts`). The Rust
REST handlers return every column.

**Fix.** Take `Query<FieldSelector>` in `list_todos`/`get_todo` and project through
`filter_list_secure` with the `AuthContext` from request extensions before
serializing. Low priority on payload grounds (a `Todo` is 247 B of JSON —
`todo-delivery-options.md`), but it is the one pattern where the library exists,
is tested, and simply is not plugged in — and it makes the JS/Rust parity story in
CLAUDE.md true on both sides.

**Acceptance test.** `GET /api/todos?fields=id,title` returns objects with exactly
those two keys; a field gated to `Admin` is absent for an anonymous caller.

---

## P3 — Resilience of outbound calls

### 3.1 No circuit breaker on outbound dependencies · M

**Problem.** `libs/core/retry` gives backoff + jitter but no breaker: a dependency
that is *slow* rather than *down* gets retried, which multiplies load on the thing
already struggling. `todo_api`'s dependencies degrade gracefully by construction
(NATS unreachable ⇒ no-op publisher, `main.rs:63-99`; KV absent ⇒ passthrough),
so this is **not urgent for the todo vertical**. It bites `zerg_api`, which calls
the tasks gRPC service, WorkOS, Qdrant and OpenAI on the request path.

**Fix.** Wrap the outbound clients in a breaker (`failsafe-rs`, or a small
half-open state machine over the existing `retry` config — `libs/patterns/state`
has the shape). Expose breaker state as a gauge through the existing metrics
recorder so "which integration is open" is a dashboard query, not a log grep.
Config-driven thresholds, no redeploy to tune.

**Acceptance test.** With the tasks service returning errors, the breaker opens
after the threshold and subsequent calls fail fast (measurably faster than the
timeout) without reaching the network; it half-opens and recovers.

---

### 3.2 Event publish can still be lost (tracked elsewhere) · M

`TodoService::emit` logs and swallows publish failures by design
(`libs/domains/todo/src/service.rs:44-49`) — correct for the browser feed, which is
DB-sourced, but it means `todo-worker` can miss a lifecycle event. This is the
**transactional outbox** item and it is already specified in
[`architecture-backlog.md`](./architecture-backlog.md) §5.2 with
[`communication-and-consistency.md`](./communication-and-consistency.md):242 as the
design. Listed here only so the HTTP plan does not look like it forgot it — **do
not duplicate the work item**.

---

## Suggested order

| Week | Items | Why first |
|---|---|---|
| 1 | 0.2, 0.3, 1.4, 1.5, 1.6 | Small, no contract break, all in `main.rs` / `axum-helpers` / DTOs; closes the leak, the shape gap and the DoS shapes. 1.5 + 1.6 are one change (adopt `create_router`) |
| 2 | 0.1 (ETag + version) | Highest-value correctness fix; needs a migration, so it wants its own week |
| 3 | 1.1 (idempotency), 2.1 (trace context) | Both land in `axum-helpers` and unblock zerg backlog 5.4 |
| 4 | 2.3 (versioning) then 1.2, 1.3 | Versioning convention must exist **before** the breaking verb/envelope changes |
| 5 | 2.2, 2.5, 2.4 | Docs + polish; 2.4 is the largest of the three |
| later | 3.1 | Belongs to zerg, not the todo demo |

Two rules that make this stick:

1. **Every fix lands in `axum-helpers`, applied in `todo_api` first.** The demo
   vertical is the proving ground; the library is the deliverable.
2. **Keep the four transports in lockstep.** `todo_api` is one backend with REST,
   SSE, WebSocket and gRPC on one port. A concurrency check, a limit clamp, or an
   idempotency guard enforced on only one of them is a bug with a bigger blast
   radius than the gap it closed — put the rule in the service/DTO layer, not the
   handler.
