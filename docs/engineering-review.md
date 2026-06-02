# Engineering Review — `nx-playground` (zerg)

> **Scope:** Architecture, design, security, observability, reliability, maintainability.
> **Method:** Source-grounded review of the Rust modular monolith (Axum REST + Tonic gRPC), NATS
> messaging, data layer, proc-macros, Kubernetes/Tilt deployment, and CI/CD. Every finding cites
> `path:line`. Subagent claims were independently verified; two were wrong and are corrected at the
> end of this document.

---

## Overall assessment

This is a genuinely well-architected Rust modular monolith with infrastructure maturity well above
the median: static-binary `scratch` images, hardened pod security contexts, graceful shutdown, OTLP
tracing scaffolding, testcontainers-based integration tests, parameterized SQL throughout, and a
clean `Models → Repository → Service → Handlers` layering with proc-macro-driven boilerplate
reduction. The bones are excellent.

It also has **one critical, deployment-blocking security gap and a cluster of latent issues.** The
most important finding: **the entire data plane is effectively unauthenticated.** A complete
JWT/Redis auth implementation exists — it is simply never enforced. This is consistent with the
project being an active work-in-progress (`apps/zerg/api/TODO.md:98` still lists "Protect all
non-public endpoints"), but it must be the #1 priority before any non-local exposure.

---

## Severity legend

| Level | Meaning |
|-------|---------|
| 🔴 CRITICAL | Exploitable now; blocks any non-local deployment |
| 🟠 HIGH | Serious risk; fix before production |
| 🟡 MEDIUM | Real risk / correctness gap; schedule soon |
| ⚪ LOW | Hygiene, dead code, doc drift |

## Findings at a glance

| # | Sev | Finding | Primary location |
|---|-----|---------|------------------|
| 1 | 🔴 | No auth enforced on any business endpoint (unauth CRUD, account takeover, priv-esc) | `apps/zerg/api/src/api/mod.rs:101-104` |
| 2 | 🟠 | Handlers ignore caller identity (IDOR) despite ownership-aware service methods | `libs/domains/projects/src/handlers.rs:150,175,199` |
| 3 | 🟠 | CI uses long-lived `GCP_SA_KEY` where keyless WIF is available | `.github/workflows/ci-optimized.yml:54,125,196` |
| 4 | 🟠 | Container vuln scan never blocks (`continue-on-error: true`) | `.github/workflows/ci-optimized.yml:271-273` |
| 5 | 🟠 | `panic = 'abort'` + no `CatchPanicLayer` → single request can crash the pod | `Cargo.toml:126` |
| 6 | 🟡 | No per-request timeout layer | `libs/core/axum-helpers/src/server/app.rs:183-199` |
| 7 | 🟡 | Unbounded list queries (no max `limit`) | `libs/domains/projects/src/models.rs:193-221` |
| 8 | 🟡 | CSRF middleware is a no-op while cookie auth is accepted | `libs/core/axum-helpers/src/http/csrf.rs:10-16` |
| 9 | 🟡 | `/metrics` scrape annotations point at non-existent endpoints (API & tasks) | `apps/zerg/api/k8s/kustomize/base/deployment.yaml:26-27` |
| 10 | 🟡 | No gRPC TLS/mTLS, no NetworkPolicy/strict mTLS, no DB statement timeout, SQL logging on | `libs/core/grpc/src/channel/mod.rs`, `libs/database/src/postgres/connector.rs:19-35` |
| 11 | 🟡 | Internal error strings leak to gRPC clients | `apps/zerg/tasks/src/service.rs:63,76` |
| 12 | 🟡 | Rate limiter fails open, including for `/auth` | `libs/core/axum-helpers/src/rate_limit/middleware.rs:115-121` |
| 13 | ⚪ | Dead `field-selector`/`selectable_fields` subsystem with header-trust footgun | `libs/core/field-selector/src/lib.rs:343-358` |
| 14 | ⚪ | Doc UIs (Swagger/ReDoc/RapiDoc/Scalar) always public | `libs/core/axum-helpers/src/server/app.rs:184-187` |
| 15 | ⚪ | Missing CSP/HSTS; deprecated `X-XSS-Protection` | `libs/core/axum-helpers/src/http/security.rs:20-33` |
| 16 | ⚪ | Dead duplicate CORS builder | `libs/core/axum-helpers/src/http/cors.rs` |
| 17 | ⚪ | OAuth account creation is not transactional | `libs/domains/users/src/oauth/account_linking.rs:40-129` |
| 18 | ⚪ | Generated protobuf committed with no regen-diff CI check | `libs/rpc/src/generated/` |
| 19 | ⚪ | Circuit-breaker flag set but never implemented | `libs/core/messaging/src/config.rs` |
| 20 | ⚪ | Stale `TODO.md` / doc drift | `apps/zerg/api/TODO.md` |

---

## 🔴 CRITICAL

### 1. No authentication is enforced on any business endpoint

`apps/zerg/api/src/api/mod.rs:101-104` applies only `optional_jwt_auth_middleware` globally — it
inserts `JwtClaims` *if a valid token is present* and otherwise lets the request through. The
mandatory guard `jwt_auth_middleware` (which exists and is correct,
`libs/core/axum-helpers/src/auth/middleware.rs:56`) is **never wired anywhere** — a repo-wide search
finds it only in doc comments and re-exports.

Consequence: every route under `/api/{projects,tasks,tasks-direct,cloud-resources,users,vector}` is
reachable with no credentials. Concretely, against `libs/domains/users/src/handlers.rs:63-73` +
`models.rs:122-129,201-212`:

- `GET /api/users` → dumps all users' PII (`UserResponse`: email, name, roles, verification status).
- `DELETE /api/users/{id}` → delete any user, unauthenticated.
- `PUT /api/users/{id}` accepts `password`, `roles`, and `email_verified` (`UpdateUser`).
  `apply_update` (`models.rs:201-212`) writes them directly. **An unauthenticated attacker can reset
  any user's password, set `email_verified: true`, and grant themselves `["admin"]` — full account
  takeover + privilege escalation.** (`change_password` *does* require the current password —
  `handlers.rs:254-261` — but `update_user` bypasses that entirely.)

Same exposure applies to projects/tasks/cloud-resources CRUD.

**Fix:** apply `jwt_auth_middleware` (mandatory) to all routers except `/auth/*`, `/health`,
`/ready`, and the doc UIs; remove `password`/`roles`/`email_verified` from the public `UpdateUser`
surface and move them behind an admin-only path.

---

## 🟠 HIGH

### 2. Handlers ignore caller identity even when authenticated (IDOR)

The service layer already has ownership-aware methods
(`libs/domains/projects/src/service.rs:42-50,87-95,115-120` — `get_project_for_user`,
`update_project_for_user`, `delete_project_for_user`), but the handlers call the **unguarded**
variants and never read `JwtClaims`:

- `projects/handlers.rs:150` `service.get_project(id)`, `:175` `update_project(id, …)`,
  `:199` `delete_project(id)`.
- `tasks/handlers/direct.rs:28-30` hardcodes `filter.project_id = None` → lists all tasks
  system-wide.
- `cloud_resources/handlers.rs:168-200` performs no project-ownership check.

So fixing #1 alone yields "any logged-in user can act on any object." The wiring to close this
already exists — handlers must extract `JwtClaims`, pass `user_id`, and call the `_for_user`
methods. The stale `// TODO: Add user_id when authentication is implemented`
(`cloud_resources/handlers.rs:112-113,258-259`, `projects/handlers.rs:203`) shows the audit log is
already losing the actor.

### 3. CI uses long-lived cloud credentials where keyless is available

`.github/workflows/ci-optimized.yml:54,125,196` — the `lint`, `test`, and `build` jobs authenticate
to GCP with `credentials_json: ${{ secrets.GCP_SA_KEY }}` (a long-lived service-account key). The
`container` job (`:258-262`) already uses Workload Identity Federation, and `id-token: write` is
granted globally (`:13`). Migrate all three jobs to WIF and delete the static key.

### 4. Container vulnerability scan never blocks

`.github/workflows/ci-optimized.yml:271-273` runs the Trivy scan with `continue-on-error: true`, so
CRITICAL/HIGH CVEs never fail CI. Trivy is also installed off the unpinned `main` branch (`:269`).
Make the scan blocking on CRITICAL/HIGH and pin the installer to a release tag.

### 5. `panic = 'abort'` + no panic-catch layer = single-request DoS

`Cargo.toml:126` sets `panic = 'abort'` for release. There is no
`tower_http::catch_panic::CatchPanicLayer` anywhere. In release builds, **any panic in a handler or
middleware aborts the whole process**, dropping all in-flight requests on that pod. The reachable
panic surface is small but real — e.g. `libs/domains/users/src/auth_handlers.rs:775` unwraps
`HeaderValue::from_str(&redirect_url)` on user-influenced input in the OAuth callback. Add
`CatchPanicLayer` and convert request-path `unwrap()`s to typed errors.
(`projects/postgres.rs:142` `serde_json::to_value(tags).unwrap()` is effectively unfailable but
should still be `map_err`'d.)

---

## 🟡 MEDIUM

### 6. No per-request timeout; missing resource ceilings

The `tower-http` `timeout` feature is enabled but no `TimeoutLayer` is applied
(`server/app.rs:183-199`). A slow/hung handler or slowloris client ties up a worker indefinitely.
(Axum's default 2 MB body limit does bound payload size, so that aspect is covered.) Add a global
request timeout layer.

### 7. Unbounded list queries

`ProjectFilter`/`TaskFilter`/cloud-resource filters default `limit=50` but enforce no maximum
(`projects/models.rs:193-221`, `tasks/models.rs:145-150`, `cloud_resources/models.rs:110-121`).
`?limit=100000000` flows straight into `LIMIT` (`projects/postgres.rs:88-90`). Clamp to a hard max
(e.g. ≤ 1000) at the DTO/service boundary; prefer keyset pagination over `OFFSET` for deep lists.

### 8. CSRF middleware is a no-op while cookie auth is accepted

`libs/core/axum-helpers/src/http/csrf.rs:10-16` is a pass-through stub, yet
`extract_token_from_request` accepts the JWT from an `access_token` cookie
(`auth/middleware.rs:16-31`) and CORS allows credentials (`server/app.rs:180`). Cookie-borne auth +
no CSRF protection = CSRF on state-changing routes. Either implement the CSRF check, or scope cookie
auth to `SameSite=Strict` and require the `Authorization` header for mutations.

### 9. Observability: scrape targets point at endpoints that don't exist

- The API Deployment annotates Prometheus to scrape `/metrics` on `:8080`
  (`apps/zerg/api/k8s/kustomize/base/deployment.yaml:26-27`), but the API exposes **no** `/metrics`
  endpoint — it is an open TODO (`apps/zerg/api/TODO.md:90-94`). Scrapes 404.
- The tasks Deployment annotates `/metrics` on `:50051`
  (`apps/zerg/tasks/k8s/kustomize/base/deployment.yaml:26-27`), which is the **gRPC (h2c) port** —
  there is no HTTP metrics server there.
- email-nats is correct: `HealthServer::with_metrics(...)` serves a real Prometheus render at
  `:8081/metrics` (`libs/core/messaging/src/nats/health.rs:162-170`, wired at
  `apps/zerg/email-nats/src/lib.rs:153`).

Also, the gRPC tracing interceptor injects only a fresh `x-request-id` and does **not** propagate
W3C trace context (`libs/core/grpc/src/interceptors/tracing.rs`), so OTLP traces won't stitch across
the API→tasks hop. Add a `/metrics` endpoint (the `metrics` / `metrics-exporter-prometheus` deps are
already present) and propagate `traceparent`.

### 10. Transport & DB hardening gaps

- gRPC channels and server have **no TLS/mTLS** option (`libs/core/grpc/src/channel/mod.rs`,
  `libs/core/grpc/src/server/config.rs`). At cluster level there is no `PeerAuthentication: STRICT`
  and no `NetworkPolicy` (`manifests/k8s/base/namespace.yaml`), so "internal" traffic is neither
  enforced-encrypted nor segmented. Add strict mTLS + default-deny NetworkPolicies.
- Postgres pool sets no `statement_timeout` and SQL logging is on by default at INFO
  (`libs/database/src/postgres/connector.rs:19-35`). Disable logging in prod (query/param leakage)
  and add a statement timeout.

### 11. Internal error strings leak to clients

The tasks gRPC service maps domain errors with `Status::internal(e.to_string())` /
`Status::not_found(e.to_string())` (`apps/zerg/tasks/src/service.rs:63,76`), surfacing DB-driver
detail to callers and mis-categorizing (a DB outage in `get_by_id` returns `NotFound`). Map to
opaque codes; log detail server-side.

### 12. Rate limiter fails open — including on auth

`libs/core/axum-helpers/src/rate_limit/middleware.rs:115-121` allows the request when Redis errors.
Defensible for general traffic, but it means brute-force protection on `/auth/*` evaporates during a
Redis blip. Consider fail-closed (or a local fallback bucket) for the `auth` tier specifically.

---

## ⚪ LOW / hygiene

- **13. Dead subsystem.** The whole `field-selector` crate + `selectable_fields` proc macro + tests
  are unused by any handler or model (`#[derive(SelectableFields)]` appears only in the macro's own
  tests). Its `AuthContext` extractor trusts client headers `x-user-role`/`x-user-id`
  (`libs/core/field-selector/src/lib.rs:343-358`) — a privilege-escalation footgun if anyone wires
  it up. Delete it, or gate it behind a trusted-gateway contract and remove the header trust.
- **14. Doc UIs always public.** Swagger/ReDoc/RapiDoc/Scalar are mounted unconditionally
  (`server/app.rs:184-187`). Gate behind env/auth in production.
- **15. Security headers** lack `Content-Security-Policy` and `Strict-Transport-Security`;
  `X-XSS-Protection: 1; mode=block` is deprecated (set `0`) — `http/security.rs:20-33`.
- **16. CORS duplication.** `http/cors.rs::create_cors_layer` is dead — `create_router` inlines its
  own identical CORS (`server/app.rs:163-181`).
- **17. OAuth account creation isn't transactional** (`oauth/account_linking.rs:40-129`): user
  insert and OAuth-link are separate writes; a failure between them orphans a user. Wrap in a
  transaction.
- **18. Generated protobuf is committed** (`libs/rpc/src/generated/`) with no CI
  regenerate-and-diff step → silent drift between `.proto` and Rust. Add a CI check that regen
  produces no diff.
- **19. Unimplemented circuit breaker** — `enable_circuit_breaker: true` is set but never read
  (`libs/core/messaging/src/config.rs`); implement it or drop the flag.
- **20. Stale tracking docs.** `apps/zerg/api/TODO.md` lists CORS, rate limiting, HPA, resource
  limits, and OpenAPI integration as not-done, but all are implemented and wired — while the auth
  gap it *does* list is the real one. Misleading drift; prune it.
- **Tracked placeholder credential** `password=staging-password-change-me`
  (`manifests/cnpg/overlays/staging/kustomization.yaml:46`) — ensure external-secrets overrides it
  before staging is reachable. (Local `compose.yaml` / `mprocs` creds are standard dev defaults —
  acceptable.)

---

## What's genuinely strong

- **Pod/container hardening** is exemplary across all apps: `runAsNonRoot`, `readOnlyRootFilesystem`,
  `cap drop ALL`, `seccompProfile: RuntimeDefault`, distroless `scratch` static binaries,
  requests/limits, three-tier probes, HPA, prod PDB.
- **Data access is injection-safe** — SeaORM typed queries + parameterized `Statement` raw SQL
  (`users/postgres_repository_impl.rs:76-120`); generic `BaseRepository<E>` for reuse.
- **Auth crypto done right where used**: HS256-only `Validation` (no `alg:none` confusion), JWT
  secret required ≥ 32 chars from env (`auth/config.rs:46-61`), argon2, OAuth with PKCE + Redis
  `GETDEL` state + origin validation.
- **Resilience plumbing**: graceful shutdown with coordinated cleanup (`main.rs:148-172`), connect
  retries with jittered backoff, NATS worker with explicit ack, DLQ, bounded concurrency, K8s health
  endpoints.
- **OTLP tracing** is correctly wired and degrades gracefully when no collector is configured
  (`libs/core/config/src/tracing.rs`).

---

## Recommended order of work

1. **Enforce auth** — wire `jwt_auth_middleware` on all non-public routers; remove
   `roles`/`password`/`email_verified` from the public `UpdateUser` (#1).
2. **Close IDOR** — handlers extract `JwtClaims`, call the existing `_for_user` service methods, fix
   audit `user_id` (#2).
3. **CI** — move lint/test/build to WIF; make Trivy blocking + pinned (#3, #4).
4. **Resilience** — `CatchPanicLayer`, request `TimeoutLayer`, pagination caps, statement timeout,
   prod SQL logging off (#5, #6, #7, #10).
5. **CSRF + headers** (#8); **observability**: real `/metrics` + trace propagation (#9).
6. **Hardening**: strict mTLS + NetworkPolicies, gRPC TLS (#10); prune dead `field-selector` and
   stale docs (#13, #20).

---

## Corrections to automated scan claims

During review, two intermediate findings were investigated and found **inaccurate** — recorded here
for transparency:

- **`.env` is *not* committed.** It is gitignored and untracked (`git check-ignore .env` → match;
  `git ls-files` → no match). There is no committed-secrets incident. (If the local file holds real
  keys, rotate them as a precaution, but the repo is clean.)
- **The tasks gRPC service is Postgres-backed, not in-memory.** `TasksServiceImpl` wraps
  `TaskService<PgTaskRepository>` (`apps/zerg/tasks/src/service.rs:30-46`,
  `apps/zerg/tasks/src/server.rs:15`). The only `Mutex<HashMap>` / `lock().unwrap()` lives in
  `#[cfg(test)]` mock code (`service.rs:164-290`) and is not a production path.
