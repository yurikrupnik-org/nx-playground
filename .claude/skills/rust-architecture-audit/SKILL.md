---
name: rust-architecture-audit
description: Audit where Rust logic lives and how it is managed across this workspace's crate layers (apps → domains → core/contracts) — layer placement, anemic models, logic leaking into apps or handlers, cross-domain coupling, service-boundary hygiene, duplication. Use when asked to review/check/audit the architecture, "where should this logic go", "is this in the right crate/layer", before extracting a service, or when a PR adds business rules, SQL, or a domain dependency to an app.
---

# Rust architecture audit

An audit answers two questions for every piece of logic in scope: **is it in the
right place** (crate + layer), and **is it managed by the right mechanism**
(type, service, event, gate). The verdict is a list of findings, each tied to
an executable check that would fail if the finding is ignored. No finding
without evidence; no rule without a command.

The rules below are the repo's own, distilled from
`docs/modular-monolith-architecture.md`, `docs/architecture-review-todo.md`,
`docs/adr-tasks-service-boundary.md`, `docs/architecture-backlog.md` and
`tools/nx/scope-tags.ts`. When a rule and the docs disagree, the docs win and
this file is what gets fixed.

## Where logic lives — the placement map

| Logic | Lives in | Never in |
|---|---|---|
| Entity invariants, state transitions, value validation | `libs/domains/<d>/models.rs` (intent methods on the type: `activate()? / suspend()?`) | service `if status ==` branches, handlers, app crates |
| Policy needing external data (quotas, counts, cross-aggregate) | `libs/domains/<d>/service.rs` | models (they cannot query), handlers |
| SQL / SeaORM queries for a domain table | `libs/domains/<d>/postgres.rs` behind the `repository.rs` trait | service, handlers, **any `apps/**` crate** |
| HTTP mapping (status codes, extractors, error → response) | `libs/domains/<d>/handlers.rs` (thin, delegates to service) | service, repository |
| Composition: wiring domains, `AppState`, middleware, routers | `apps/<vertical>/<bin>/src` | domain crates (a domain never imports another domain) |
| App-level metadata that is not domain data | app crate, with a `//!` header saying **why** it is not a domain (worked example: `apps/todo/api/src/stacks.rs`) | a domain crate it does not belong to |
| Cross-cutting infra (auth, retry, messaging, axum helpers) | `libs/core/*` | domains (they consume it), apps (they configure it) |
| Wire contract for an extracted service (DTOs, proto conversions, ts-rs) | `libs/contracts/<svc>` | the service's domain crate |
| HTTP→gRPC client handlers for an extracted service | the **caller's** app (`apps/zerg/api/src/api/tasks.rs`) | the callee's domain crate |
| Cross-domain facts | an event on an `EventLog` stream (`ProjectDeleted`), consumer holds an ID | an import of the other domain, a cross-context FK |
| Transport choice (NATS vs gRPC vs REST) | `skill://service-transport` decides | ad hoc |

Layer direction inside a domain is strict: `handlers → service → repository →
models`. Lower never imports higher.

## Invariants (each is executable)

1. **Domains compose only at the app layer.** No `domain_*` crate depends on
   another `domain_*` crate. The one grandfathered edge is
   `domain_cloud_resources -> domain_projects` (SeaORM FK, undecided context
   seam — `GRANDFATHERED` in `tools/nx/scope-tags.ts`). `task boundaries`
   enforces scope; this audit checks the stricter no-domain-imports-domain rule.
2. **A caller of an extracted service depends on its contract, never its
   domain crate.** `grep -rn "domain_tasks" apps/zerg/api/` must be empty.
3. **An app crate holds no domain SQL.** `sqlx::`/`sea_orm::` in `apps/**/src`
   is either composition (pool construction, health probe, JIT tenant
   provisioning with a stated reason) or a finding. Today's stated exceptions:
   `apps/zerg/api/src/orgs/` (mirror of IdP orgs, header explains),
   `apps/todo/api/src/stacks.rs` (reference data, header explains).
   `apps/terran/api/src/{db,provisioning,inventory}.rs` is different: terran
   is a flat single-crate API with its data layer in the binary and **no doc
   records that as a decision** — carry it as an open Issue-1 item, not an
   exception. A new site with no `//!` rationale is a finding.
4. **Handlers are thin.** A handler file contains no `sqlx::`, `sea_orm::`,
   status-transition branches, or business arithmetic.
5. **Invariants sit on the type, not in the service.** A service branching on
   `entity.status ==` before a write is the anemic-model smell (Issue 2 in
   `docs/architecture-review-todo.md`). Every write path must go through the
   same guard — the known defect class is a guarded `activate()` next to an
   unguarded `PUT /{id}` → `apply_update`.
6. **Every domain error maps through `axum_helpers::AppError`** via
   `impl_into_response_via_app_error!` in its `error.rs`.
7. **Extraction requires the seven-row boundary checklist**
   (`docs/modular-monolith-architecture.md` → "Then: the boundary checklist"):
   contract-only client dep, exclusive table ownership, IDs not FKs,
   authenticated hop, decoupled readiness, additive contract, one process one
   capability. Resilience plumbing alone is not a boundary.
8. **Reuse is measured, not assumed.** A proposed abstraction must cite a
   measured delta (LOC removed, compile-time enforcement gained, tests
   collapsed); see `docs/code-reuse-patterns.md` for the accepted patterns.

## Procedure

### 0. Scope

- **Whole workspace**: all of `libs/domains/*`, `libs/core/*`,
  `libs/contracts/*`, `apps/*/*` Rust crates (`Cargo.toml` `[workspace]
  members` is the authoritative list).
- **One crate**: that crate plus every crate that depends on it
  (`cargo tree -i -p <crate> --depth 1`).
- **A diff**: `git diff --name-only <base>` filtered to `*.rs` and
  `Cargo.toml`, then widen to the crates those files belong to.

Never run these probes through nx (`nx run-many … tag:lang:rust`); everything here
is cargo-direct or plain grep, per AGENTS.md.

### 1. Mechanical probes — run all, paste raw output into the report

```bash
# I1 — domain imports domain (expect only the grandfathered edge)
for c in libs/domains/*/Cargo.toml; do
  echo "$c: $(grep -oE '^domain_[a-z_]+' "$c" | tr '\n' ' ')"; done

# I2 — caller depends on an extracted service's domain crate (expect empty;
# match the manifest and code paths, not doc comments that mention the crate)
grep -n 'domain_tasks' apps/zerg/api/Cargo.toml
grep -rnE '^\s*use domain_tasks|domain_tasks::' apps/zerg/api/src --include='*.rs'

# I3 — SQL in app crates (each hit needs a //! rationale or is a finding)
grep -rln 'sqlx::\|sea_orm::' apps --include='*.rs' | sort

# I4 — thick handlers (expect empty)
grep -ln 'sqlx::\|sea_orm::\|\.status = ' \
  libs/domains/*/src/handlers.rs libs/domains/*/src/handlers/*.rs 2>/dev/null

# I5 — anemic model signal: pub-field count vs intent methods, and service-side guards
for m in libs/domains/*/src/models.rs; do
  echo "$m: $(grep -c '^\s*pub [a-z_]*:' "$m") pub fields, $(grep -c 'pub fn' "$m") pub fns"; done
grep -n 'status\s*==' libs/domains/*/src/service.rs

# I6 — error mapping (expect one line per domain that has handlers)
grep -L 'impl_into_response_via_app_error' libs/domains/*/src/error.rs

# Composition edges of every app (what each binary wires together)
for a in apps/*/*/Cargo.toml; do
  echo "$a: $(grep -oE '^(domain_|contract_|rpc|database|messaging|oidc-auth|axum-helpers)[a-z_-]*' "$a" | tr '\n' ' ')"; done

# Real (non-dev) dependency closure of one crate — the nx graph is WIDER
# (@monodon/rust flattens dev-deps), so use cargo for "who really links what"
cargo tree -p <crate> -e normal --depth 1 --prefix none | grep -E '^(domain_|contract_|rpc)'

# Module map of a domain (layer files present / missing)
cargo modules structure -p <crate> --lib --no-fns --no-traits --no-types

# Test surface per domain (a domain with 0 unit + 0 integration is unaudited logic)
for d in libs/domains/*/; do
  echo "$d unit=$(grep -rl '#\[cfg(test)\]' "$d/src" | wc -l | tr -d ' ') it=$(ls "$d/tests" 2>/dev/null | wc -l | tr -d ' ')"; done

# Executable gates that already encode architecture rules
task boundaries        # scope tags over the nx graph
task proto-breaking    # wire-contract additivity
```

Baseline as of 2026-09-19, so a drift is visible: I1 → only
`cloud_resources: domain_projects`; I2 → empty; I3 → the two stated
exceptions (`zerg/api/src/orgs/*`, `todo/api/src/stacks.rs`) plus terran's
`{db,error,health,provisioning}.rs` and `tests/api.rs`; I4 → empty; I5 → `projects/service.rs` has 3 `status ==` guards;
I6 → `libs/domains/tasks` (no handlers by design, contract-only); tests →
`domain_tasks` 0/0, `domain_users` and `domain_cloud_resources` 1/0.

### 2. Reading pass — per crate in scope

Read in this order and note where each answer is "elsewhere":

- `lib.rs` — which layer modules exist. A domain missing `service.rs` or
  `repository.rs` has its logic somewhere else; find it.
- `models.rs` — for each entity: are fields `pub`? Is there a constructor and
  intent methods? List every status/lifecycle field and every write path that
  can set it (`apply_update`, `update_*`, direct `postgres.rs` writes). Any
  write path that bypasses the guard is a finding with a red test:
  "`PUT` status=X on a Y entity → 4xx".
- `service.rs` — classify each `pub async fn`: **orchestration** (calls repo,
  emits event — fine), **policy needing external data** (fine, document it),
  **invariant on one aggregate** (belongs on the type — finding).
- `repository.rs` / `postgres.rs` — CRUD and queries only. Business rules in
  SQL (`WHERE status != 'deleting'` as the only guard) are a finding.
- `handlers.rs` — mapping only. Named command endpoints (`/activate`,
  `/complete`) are preferred over semantic `PUT`s; a `PUT` that carries
  business meaning is an Issue 3 finding.
- `error.rs` — `impl_into_response_via_app_error!` present; no `500` for a
  domain-rule violation.
- The app crate(s) that consume it — is composition the only thing there? Any
  business decision in `apps/**` (beyond auth/tenancy middleware, which is
  app-level by design) is a finding: name the domain it belongs to.

For an extracted or about-to-be-extracted service, walk the seven-row
checklist explicitly and cite a file per row. `tasks` is the worked example:
every row holds and `apps/zerg/tasks/tests/boundary_smoke.rs` proves the auth
row.

### 3. Verdict format

One entry per finding, in the `docs/architecture-backlog.md` shape, ordered by
blast radius (data integrity → boundary → placement → duplication):

```text
### <n>. <one-line problem> · S|M|L
**Problem.** <what and where — file:line, plus the probe output that shows it>
**Rule.** <invariant number above, or the doc section it comes from>
**Fix.** <the smallest move that makes the rule hold — one aggregate/crate per PR>
**Acceptance test.** <a command or test that is red now and green after>
```

Also record what is **already right** with the same evidence, so the next
audit does not re-litigate it (the review doc marks these `[x]`).

Finish with the scorecard rows the review doc uses (layering, anemic model,
CRUD-vs-intent, distributed-monolith, boundaries) and a status per row.

### 4. What the audit does NOT do

- **It does not refactor.** Findings first; a fix is a separate change, one
  aggregate or one crate per PR, reviewed via `skill://precommit-review`.
- **It does not propose an abstraction without a measured delta** (rule 8).
  "Generic base repository" is only a finding if the duplicated LOC is counted.
- **It does not grade a service split on resilience plumbing** (lazy connect,
  retries, health gating). Those are necessary, not sufficient — grade the
  checklist rows.
- **It does not recommend extraction.** A domain crate is already the
  boundary; extraction needs the "needs its own process because it must ___"
  sentence with a concrete ending (replicas, survivability, language, team).
- **It does not touch generated code** (`libs/**/types`, `libs/rpc/src/generated`,
  `docs/openapi`) — those are outputs of `export_bindings_*` tests and buf.

## Companion skills

- `skill://new-rust-crate` — where a new crate goes and its template; consult
  when a finding's fix is "this needs its own crate".
- `skill://service-transport` — when a finding's fix is "replace this import
  with an event" or "this hop needs a contract".
- `skill://db-migration` — when a finding's fix moves a table between
  databases or converts an FK to an ID column.
- `skill://precommit-review` — the fix PR goes through it.
