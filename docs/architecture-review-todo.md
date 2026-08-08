# Architecture Review — 5 Enterprise Software Killers
# based on: https://www.youtube.com/watch?v=aGgmY0kgJTA

> **Source:** "5 Terrible Decisions That Kill Enterprise Software" — CodeOpinion (Derek Comartin), YouTube `aGgmY0kgJTA`.
>
> **⚠️ Accuracy caveat:** The exact transcript could not be extracted (browser extension offline, transcript proxies blocked). The 5 issues below are **reconstructed from CodeOpinion's consistent, well-documented positions** on what kills enterprise systems — not a verbatim transcript of this specific video. Watch the video and correct any of the 5 headings that differ; the mapping to our code and the recommended patterns stay valid regardless of exact wording.
>
> **Good news up front:** Our system is already healthy on several of these. Domains are independent crates composed only at the app layer, there's no god-repository, and `libs/core` holds infra (not shared business entities). This doc marks what's ✅ already good, ⚠️ partial, and ❌ a real smell — so effort goes where it matters.

**Status legend:** `[ ]` todo · `[~]` partial/in-progress · `[x]` already satisfied

---

## Issue 1 — Organizing by technical concern instead of business capability

**The argument:** Layered/n-tier apps grouped by "what a file *is*" (all controllers, all services, all repos) instead of "what it *does*" (a feature) produce low cohesion and high coupling. A change to one feature touches every layer folder.

**In our system — ✅ mostly good, worth protecting.**
- We slice **vertically at the crate boundary**: each domain (`projects`, `users`, `tasks`, `todo`, `vector`, `cloud_resources`) is its own crate owning its full stack. There is *no* global `handlers/` or `services/` directory. See `libs/domains/projects/src/lib.rs`.
- Internally each crate is layered (`handlers → service → repository → models`), which is fine — the anti-pattern is layering *across the whole app*, not within a slice.

**Recommended patterns:** Vertical Slice Architecture (keep it), Bounded Context per crate, Screaming Architecture (folder names reveal the domain, not the framework).

**Todos**
- [x] Domains are vertical slices at the crate boundary — no cross-app technical layering.
- [ ] Add a lint/CI guard (or `deny.toml` / import rule) so no new global `handlers/`-style technical grouping creeps in.
- [ ] Document the "slice = crate, layers live inside the slice" convention in `docs/modular-monolith-architecture.md` so new domains follow it by default.

---

## Issue 2 — Anemic Domain Model (logic in the wrong place)

**The argument:** When entities are just public-field data bags and all rules live in fat service classes, you have a procedural transaction-script wearing an OO costume. Invariants aren't enforced by the type, so they get duplicated, skipped, or drift.

**In our system — ❌ the most notable smell.**
- `Project` = 13 public fields, no invariants on the type (`libs/domains/projects/src/models.rs`). `User` = 17 public fields (`libs/domains/users/src/models.rs`).
- Real domain rules live in services, not the model: the 3-project free-tier limit in `ProjectService::can_user_create_project` (`libs/domains/projects/src/service.rs`), and status-transition guards in `activate_project` / `suspend_project`.
- There *is* a thin veneer (`Project::new`, `Project::apply_update`) — so it's not purely anemic, but the important rules are outside the model.

**Recommended patterns:** Rich Domain Model / Aggregates, encapsulated invariants (private fields + intent methods), value objects for validated primitives (e.g. `ProjectName`, `EmailAddress`), state as an explicit state machine on the aggregate.

**Todos**
- [ ] Pick **one** reference aggregate (`Project`) and move its rules onto the type: status transitions become `project.activate()? / .suspend()?` returning `Result`, not service branches.
- [ ] Introduce value objects for the most-validated fields (`ProjectName`, `Email`) so invalid states are unrepresentable rather than regex-checked at the edge.
- [ ] Encapsulate fields where practical (constructor + intent methods instead of all-`pub`), starting with `Project`, then `User`.
- [ ] Leave cross-aggregate / policy logic (free-tier quota needs a repo count) in the service — document the split: **aggregate = invariants, service = orchestration + policy needing external data.**
- [ ] Treat this as incremental: one aggregate per PR, no big-bang rewrite.

---

## Issue 3 — Data/CRUD-centric design instead of behavior (task-based)

**The argument:** Exposing entities as raw Create/Update/Delete pushes business decisions up to the caller and loses intent. "Set status = 'suspended'" tells you nothing about *why*; `SuspendProject(reason)` does. CRUD APIs breed anemic models (Issue 2) and put orchestration in the UI/client.

**In our system — [~] partially addressed.**
- Base routers are CRUD: `GET/POST /`, `GET/PUT/DELETE /{id}` (`libs/domains/projects/src/handlers.rs`, same in `tasks`, `todo`).
- **But** the richer domains already add intent-revealing endpoints: `POST /{id}/activate|suspend|archive` on projects, and login/register/OAuth actions in `users/src/auth_handlers.rs`. This is the right direction.

**Recommended patterns:** Task-based UI / intent-revealing endpoints, Commands over CRUD, (optionally) CQRS to split write-intent from read-shapes. Keep generic CRUD only for genuinely CRUD-shaped reference data.

**Todos**
- [ ] Audit each domain's `PUT /{id}` — where an update carries business meaning, add a named command endpoint alongside/instead of the generic update.
- [ ] Define command DTOs that carry *intent + reason*, not just the new field values, for state-changing operations.
- [ ] Decide per-domain: is this entity genuinely CRUD (config/reference data → keep CRUD) or behavioral (has a lifecycle → task-based)? Record the decision.
- [ ] Ensure audit events (already emitted in `handlers.rs`) capture the *command name*, not just "updated".

---

## Issue 4 — Coupling: the synchronous distributed monolith

**The argument:** Splitting a system but wiring the pieces with synchronous request/response chains gives you the worst of both — distribution's failure modes plus the monolith's temporal coupling. A → B → C sync calls fail together, scale together, and deploy together.

**In our system — [!] REVISED 2026-07-25. The resilience infra is excellent; the *boundary* is not one. See `docs/adr-tasks-service-boundary.md`.**
- Sync in-process for `projects`, `users`, `cloud_resources` — fine, they're in one process.
- **What holds up (unchanged):** `apps/zerg/tasks` is a standalone gRPC server binary; `apps/zerg/api` holds a pooled `TasksServiceClient` (`grpc_pool.rs`) that `connect_lazy()`s so the api boots independently, with a tracing interceptor, HTTP/2 keep-alive, 5s connect / 30s request timeouts, and `create_channel_with_retry` (`libs/core/grpc/src/channel/mod.rs`). Contract is versioned (`tasks.v1`) and generated for both sides by buf. This is genuinely good plumbing.
- **What does not hold up:** the original verdict graded *resilience infrastructure* and never audited *boundary hygiene*. On the three tests that define a service boundary, it fails:
  - **Dependency direction.** `apps/zerg/api` and `apps/zerg/tasks` both depend on `domain_tasks`; the client imports `PgTaskRepository, TaskService, handlers` (`api/tasks_direct.rs:2`), and the caller's HTTP→gRPC handlers live inside the callee's crate (`domain_tasks/handlers/grpc.rs`). A domain model change recompiles and redeploys both — the independence the split was meant to buy.
  - **Data ownership.** Two processes write one table: `api/tasks_direct.rs:5` and `tasks/src/server.rs:71` both construct `PgTaskRepository`. `/api/tasks-direct` is a live path that bypasses the service entirely. This *is* "a domain reaching synchronously into another's data mid-request" — the thing Issue 4 says is not happening. Adding tenant scoping had to be applied twice or isolation would have been bypassable.
  - **Authentication.** `org_id`/`user_id` cross the wire as plaintext proto fields the server trusts. `required_uuid()` validates shape, not entitlement — anything that can reach `:50051` can read any org's tasks.
- **Also:** `/ready` gates on `tasks_grpc` (`api/health.rs:40`), so tasks being down takes projects, users, and org endpoints down with it — *negative* fault isolation, two processes that must both be up. And `tasks/src/server.rs:101-102` hosts `VectorServiceServer` alongside tasks, so "scale tasks independently" is already false.
- **Async events** for `todo`: publishes `TodoEvent` to NATS JetStream, consumed by `apps/todo/worker`, hidden behind a `TodoEventPublisher` trait (`libs/domains/todo/src/events.rs`). Healthy loosely-coupled pattern — and notably our *cleanest* boundary, because a queue forces a message contract and forbids shared state, while a synchronous call lets you keep both and feel separated.
- Note commit `aaf4c85` pruned legacy messaging helpers from `libs/core` but `libs/core/messaging` is still live — worth confirming the messaging story is coherent, not half-removed.

**The corrected nuance:** synchronous-on-the-request-path was never the real problem — with lazy connect and health gating that is a reasonable trade. The real problem is that we pay network cost for module-level coupling: shared crate, shared table, unauthenticated hop. That is the textbook distributed monolith, arrived at from the opposite direction than expected.

**Recommended patterns:** Execute `docs/adr-tasks-service-boundary.md` (contract-only client dep, own database with ID references instead of cross-service FKs, verified-token identity at the hop, decoupled readiness). For *new* cross-boundary flows, prefer events over synchronous hops.

**Todos**
- [x] `tasks` split has independent boot (`connect_lazy`), health gating, pooled client, timeouts, retry helper — resilience infra is in place.
- [x] Audit boundary hygiene, not just resilience → recorded in `docs/adr-tasks-service-boundary.md`.
- [x] **Phase 1:** delete `/tasks-direct` and `direct_router` — one door to the data. *(2026-07-25: route returns 404, OpenAPI lists only `/tasks`; `bench-*-direct` recipes removed.)*
- [x] **Phase 2:** extract `libs/contracts/tasks`; `zerg_api` depends on the contract, not `domain_tasks`. *(2026-07-25: `grep -rn "domain_tasks" apps/zerg/api/` is empty; `domain_tasks` shed 16 deps incl. axum/utoipa/tonic/rpc/ts-rs.)*
- [x] **Phase 3:** give tasks its own database; `org_ref`/`user_ref` IDs replace cross-service FKs. *(2026-07-25: `manifests/db/tasks/` + `tasks_app` role; `tasks` absent from the zerg schema.)*
- [x] **Phase 4:** forward the user token as gRPC metadata, verify via JWKS in `zerg_tasks`, delete `org_id`/`user_id` from proto requests. *(2026-07-25: direct unauthenticated and forged-token calls to `:50051` both return `Unauthenticated` — see `apps/zerg/tasks/tests/boundary_smoke.rs`.)*
- [x] **Phase 5:** ungate `/ready`, add per-call deadlines, split `VectorService` into its own binary, adopt additive-only proto policy. *(2026-07-25: with tasks killed, `/ready` stays 200 and only `/api/tasks` returns 503.)*
- [ ] Guardrail: before any *future* extraction, run the boundary checklist in `docs/modular-monolith-architecture.md` — resilience infra alone is not a boundary.
- [ ] Resolve the messaging-lib ambiguity: document what `libs/core/messaging` is for post-cleanup, or finish removing it. Cross-link `docs/messaging-patterns.md`.
- [ ] Prefer publishing an event over a synchronous cross-domain call when adding new inter-domain flows.

---

## Issue 5 — Wrong boundaries / premature decomposition (split by entity or layer, not capability)

**The argument:** Microservices split along entity lines or technical layers (a "Customer service", an "Order service") force chatty sync calls and shared models. Boundaries should follow **business capabilities**; distribution should be deferred until a real scaling/deployment reason exists. A modular monolith with clean seams beats a premature distributed system.

**In our system — ✅ largely right, one thing to watch.**
- We're a **modular monolith**: domains are independent crates, composed only at `apps/zerg/api` — the recommended starting point. See `docs/modular-monolith-architecture.md`.
- No shared god-entity; `libs/core` is infra only.
- **The one cross-domain link:** `cloud_resources` `belongs_to` `projects` via a SeaORM FK (`libs/domains/cloud_resources/src/entity.rs`). It's a persistence-level FK, not a runtime call — mild, but it's the seam that would hurt most if these ever became separate services (shared DB / cross-context FK).

**Recommended patterns:** Modular Monolith first (keep it), boundaries by business capability, contexts communicate via IDs + events (not shared FKs across contexts), extract a service only when a concrete scaling/deployment/ownership driver appears — the "monolith-first" rule.

**Todos**
- [ ] Decide whether `cloud_resources → projects` is *one* bounded context (then the FK is fine) or *two* (then replace the cross-context FK with an ID reference + validation/event). Document the call.
- [ ] Write down the explicit rule: **new domains reference others by ID, not by importing entities or FK-ing across contexts.**
- [ ] Add a "when do we extract a service?" checklist (scaling, independent deploy, team ownership) so distribution stays a deliberate decision, not a default.
- [ ] Keep the app layer as the only composition point — no domain-imports-domain code coupling.

---

## Summary scorecard

| # | Issue | Our status | Priority |
|---|-------|-----------|----------|
| 1 | Layer-by-technical-concern | ✅ Vertical slices already | Low — guard it |
| 2 | Anemic domain model | ❌ Real smell (logic in services) | **High** |
| 3 | CRUD instead of behavior | [~] Partial (some action endpoints) | Medium |
| 4 | Synchronous distributed monolith | ✅ `tasks` is a deliberate microservice split w/ resilience; minor verify | Low |
| 5 | Wrong/premature boundaries | ✅ Modular monolith; FK seam to decide | Low–Med |

**Suggested order of attack:** Issue 2 (highest leverage, unblocks 3) → Issue 3 → Issues 1, 4 & 5 (mostly guardrails, verification, and documentation — the boundaries and transports are already sound).

**Next step:** Confirm the 5 headings against the actual video, then I can turn any section into a concrete implementation plan (starting with the `Project` aggregate for Issue 2).
