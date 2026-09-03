# Architecture Backlog

- **Status:** Living
- **Date:** 2026-07-25
- **Source:** the `tasks` service-boundary work and the communication/consistency review
- **Related:** [`adr-tasks-service-boundary.md`](./adr-tasks-service-boundary.md),
  [`communication-and-consistency.md`](./communication-and-consistency.md),
  [`modular-monolith-architecture.md`](./modular-monolith-architecture.md),
  [`architecture-review-todo.md`](./architecture-review-todo.md)

Actionable work queue, ordered by priority. Each item states the **problem**, the **fix**,
and the **acceptance test** — an item is not done until the acceptance test would fail if
the fix were reverted.

Size: **S** ≤ half a day · **M** 1–2 days · **L** > 2 days.

---

## P0 — Live defects

### 0.1 NATS durable consumer name is unique per process · S · ✅ FIXED 2026-07-25

**Problem.** `libs/core/messaging/src/nats/config.rs:92` and `:124` both build the name
with a fresh UUID:

```rust
durable_name: format!("{}-{}", S::CONSUMER_NAME, uuid::Uuid::new_v4()),
```

Both workers reach this via `WorkerConfig::from_stream::<S>()`
(`apps/zerg/email-nats/src/lib.rs:93`, `apps/todo/worker/src/lib.rs:85`). JetStream
identifies a durable consumer **by name**, so every process creates its own and receives
the full subject stream — fan-out semantics where a work queue was intended.

What fires **today** at `replicas: 1`:
- a new consumer is created on every restart/deploy; the old one **leaks server-side**
- the previous consumer's offset is abandoned; the new one starts from `DeliverPolicy`

What fires the moment there is more than one pod (**including the overlap window of a
rolling deploy**):
- **every email is sent once per replica**

**Reproduced 2026-07-25.** Two extra worker replicas are wired into
`manifests/mprocs/local.yaml` (`email-replica-a` / `email-replica-b`, `autostart: false`,
distinct health ports). With three durable consumers registered on `EMAILS`, publishing
**one** job delivered **three** emails to MailHog — `delivered == consumer count`, which
is the definition of fan-out:

```
  jobs published    : 1
  durable consumers : 3
  emails delivered  : 3
  FAIL - fan-out: 3 deliveries for 1 job (one per consumer).
```

Both secondary effects were observed directly too: consumers from earlier runs were still
registered (one nearly two hours old, long after its process exited), and a newly created
consumer **replayed the whole stream backlog** — an initial run delivered nine emails,
including old messages addressed to real inboxes, because a fresh consumer starts from its
`DeliverPolicy` rather than where the previous one stopped.

**Fixed.** `StreamConfig::CONSUMER_NAME` is now used verbatim as the durable name, so
every replica joins one consumer group. The field `WorkerConfig::durable_name` is gone —
the consumer name *is* the group identity, and having two fields is what let them drift.
Per-instance broadcast is now an explicit, documented opt-in
(`WorkerConfig::with_per_instance_broadcast`).

A second, server-side lever was added: `StreamConfig::KIND` declares whether a stream is
a `JobQueue` or an `EventLog`, and stream creation sets retention from it.

| Stream | Kind | Retention | Why |
|---|---|---|---|
| `EMAILS` | `JobQueue` | `WorkQueue` | sending is a job; must happen exactly once |
| `TODOS` | `EventLog` | `Limits` | domain facts; a second consumer group may be added later |
| `PROJECTS` | `EventLog` | `Limits` | domain facts (added by 0.3); `tasks-project-refs` is its first group |

`JobQueue` makes the bug **structurally impossible**, not merely fixed — NATS rejects a
second overlapping consumer:

```
$ nats consumer add EMAILS rogue-worker --filter 'emails.>'
nats: error: Consumer creation failed: filtered consumer not unique on workqueue stream (10100)
```

**Verified.** Three worker processes, one consumer group, one delivery:

```
  jobs published    : 1
  durable consumers : 1
  emails delivered  : 1
  PASS - work queue: one job, one delivery
```

Regression cover: `replicas_share_one_consumer_group`,
`stream_kind_defaults_to_job_queue`, `per_instance_broadcast_is_opt_in_and_unique`
(`libs/core/messaging/src/nats/config.rs`), plus `just email-replica-check` end to end.

**Gotcha found while fixing.** `EventLog` first mapped to `Interest` retention, which
drops a message published while no consumer exists and gives a later-added group nothing
that came before it — destroying the one property an event log is for. The
`domain_todo` integration test caught it. `EventLog` maps to `Limits`.

**Migration note.** Retention cannot be changed in place on an existing stream; `EMAILS`
had to be deleted and recreated. Trivial locally (`nats stream rm EMAILS`), needs a drain
window anywhere real.

**Verified at scale 2026-07-25.** Four replicas, one consumer group
(`just email-scale-check <n>`):

| Jobs | Delivered | Distinct | Dupes | Lost | Per-replica split | End-to-end |
|---|---|---|---|---|---|---|
| 500 | 500 | 500 | 0 | 0 | — | ~125/s |
| 2 000 | 2 000 | 2 000 | 0 | 0 | 495 / 501 / 502 / 502 | 222/s |
| 10 000 | 10 000 | 10 000 | 0 | 0 | 2493 / 2501 / 2503 / 2503 | 301/s |

Load spreads within **0.3 %** of an even split at 10k, so the single consumer group is
genuinely balancing rather than one replica starving the others.

**Churn.** Killing a replica mid-flight (3 000 jobs): **0 lost**, **1 duplicate**. NATS
redelivered the dead replica's two unacked messages after `ack_wait`; one had already
been handed to SMTP before the process died, so it was sent twice. That is item 0.2
exactly — quantified: ~1 duplicate per replica loss, and no amount of work-queue
correctness removes it. Only `Nats-Msg-Id` + `duplicate_window` (or an idempotent
processor) will.

**Two harness traps, both fixed:**

- *MailHog caps `count` at 250 per request* regardless of `?limit=`. A recipient-level
  check MUST paginate; `.total` is accurate but cannot tell "1 000 delivered" from
  "500 delivered twice". The first 500-job run looked like 50 % duplicates purely
  because of this.
- *Unbounded in-flight JetStream publishes deadlock.* Queuing 10 000 acks before
  awaiting any wedged the publisher at ~5 000 while every message it had sent was
  consumed normally. `publish_bulk` now bounds in-flight acks to 500.

`stream_config_for()` (`libs/core/messaging/src/nats/consumer.rs`) is now the single
place a stream config is built. `examples/publish_test.rs` had hardcoded
`RetentionPolicy::Limits`, so running the publisher before the worker would have
recreated `EMAILS` as a non-work-queue and silently restored fan-out.

---

### 0.2 No publish-side idempotency · S

**Problem.** Nothing sets `Nats-Msg-Id`; no stream sets `duplicate_window`
(`grep` confirms both absent). With at-least-once delivery, a worker that sends an email
and dies before acking sends it again on redelivery.

**Measured, not hypothetical (2026-07-25).** Killing one of four replicas mid-flight
during a 3 000-job run produced **1 duplicate and 0 losses** — the message that had
reached SMTP but not yet been acked. The work-queue fix in 0.1 cannot prevent this; it
is the residual at-least-once exposure. Blast radius scales with replica restarts, so
every rolling deploy is a handful of duplicate emails.

**Fix.** Set `Nats-Msg-Id` to the job id on publish; configure `duplicate_window` on the
stream. Ideally also make the processor idempotent (record "welcome email sent for user X").
Note `duplicate_window` dedupes *publishes*, not *redeliveries* — the case measured above
is a redelivery, so the processor-side record is the part that actually closes it.

**Acceptance.** Publish the same job id twice within the dedupe window; assert one
delivery. Then re-run the churn test: kill a replica mid-flight during
`just email-scale-check 3000` and assert **0** duplicates (it reports 1 today).

---

### 0.3 Dangling `project_id` after project deletion · S · ✅ FIXED 2026-08-31

**Problem.** Phase 3 removed `tasks.project_id`'s FK
(`REFERENCES projects(id) ON DELETE SET NULL`). Deleting a project left tasks
pointing at a project that no longer existed. Unlike the user/org orphans the ADR
consciously accepted, **project deletion is implemented today**.

**Fixed with the event, not the read-side patch.** `ProjectService::delete_project`
publishes `ProjectDeleted`; `zerg_tasks` consumes it and nulls its refs. An ID
reference plus an event is what replaces a cross-context FK.

**The contract crate was forced by the gate from 1.3, which is the point.** Defining
the event in `domain_projects` and consuming it from `zerg_tasks` is the shared-kernel
coupling Phase 2 removed — and `just boundaries` now **rejects** it
(`scope:tasks` → `scope:zerg`). So the payload plus the stream identity live in a new
`libs/contracts/projects` (`scope:shared`), which is exactly 5.1's rule of thumb: a
serialization boundary between independently deployed processes. The consumer group
name is deliberately **not** in the contract — `CONSUMER_NAME = "tasks-project-refs"`
is declared by the service that reads it, so a second reader (search, audit) adds its
own group and gets its own copy.

| Piece | Where |
|---|---|
| `ProjectDeleted` + stream identity | `libs/contracts/projects` |
| `ProjectEventPublisher` / `NatsProjectPublisher` | `libs/domains/projects/{events,nats}.rs` |
| `clear_project_refs` | `libs/domains/tasks` (repository + service) |
| `ProjectRefsStream` / `ProjectRefsProcessor` | `apps/zerg/tasks/src/project_events.rs` |

**Four properties that are load-bearing:**

- **Publish is best-effort**, matching the `establish_session` welcome-email dual write:
  the row is already gone, so failing the request would report an error for committed
  work and invite a retry of an unrepeatable delete. A lost event therefore leaves a
  stale id, which is why the read side must still tolerate one — the two halves of this
  item. Upgrade to an outbox (5.2) only if a consumer needs exactly-once.
- **Idempotent by construction.** The work is "null the refs to this id", so a
  redelivery clears 0 rows. At-least-once needs no dedupe here, unlike 0.2 where the
  side effect is an email.
- **Not a boot dependency.** The consumer is spawned but never gates gRPC startup, the
  same reasoning that ungated `/ready` in Phase 5. NATS down = corrections delayed, not
  tasks unavailable. `PROJECTS` is an `EventLog` with a server-side cursor, so a service
  that was down when a project was deleted still applies the correction on restart.
- **`clear_project_refs` is cross-tenant on purpose**, unlike every other method on
  `TaskRepository`: the event carries no org, project ids are globally unique, and
  org-scoping it would leave every other org's rows dangling.

**Acceptance — verified red/green 2026-08-31.**
`apps/zerg/tasks/tests/project_refs_it.rs` creates tasks in a doomed project and one in
a surviving project, publishes `ProjectDeleted` through real JetStream using **only the
contract** (never `domain_projects` — a dev-dependency is a real cargo edge the gate
sees), runs the real `NatsWorker` + processor against real Postgres, then asserts the
tasks still load with `project_id: None` while the other project's task is untouched.
Stubbing `clear_project_refs` to `Ok(0)` fails it on the timeout; reverting is green.
Publisher side is covered by three unit tests in `domain_projects`: delete publishes,
a missing project publishes nothing, and a publish failure does not fail the delete.

---

### 0.4 The retry counter never incremented, so the DLQ was unreachable · S · ✅ FIXED 2026-07-26

**Problem.** `worker.rs:238` decided retry-vs-DLQ from `message.job.retry_count()` — the
**payload's** counter. But `nak` asks JetStream to redeliver the *stored* bytes, so a
counter the consumer increments is discarded on the way out. `Job::with_retry()` was
implemented by every job type and called **only in tests**
(`job.rs:162`, `email/src/job.rs:290`, `email/tests/integration_test.rs:273`). The value
was therefore `0` on every delivery, `should_retry(0)` was always true, and the
`else` arm that moves a transient failure to the DLQ (`worker.rs:279-295`) **could never
execute**.

Two consequences, both silent:

- **No transient failure ever reached the DLQ.** Only `ErrorCategory::Permanent` did.
  A job failing retryably — SMTP down for an hour — exhausted JetStream's
  `max_deliver` (5 for `EMAILS`/`TODOS`) and the server dropped it. No DLQ entry, no
  `job_moved_to_dlq()` metric, one `warn!` per attempt.
- **Backoff never backed off.** `backoff_delay_ms(0)` is `base * 2^0`, so retries went
  out at a flat 1 s five times instead of 1→2→4→8 s. `RateLimited`'s 120 s ceiling was
  unreachable, and a rate-limiting downstream got hammered at the base interval.

**Fixed.** The attempt count now comes from `NatsMessage::delivery_count`, the server's
own counter, which was already being read (`consumer.rs:203`) and used only for a
`debug!` log. The whole retry surface — `retry_count`, `with_retry`, `max_retries`,
`can_retry` — is **gone from the `Job` trait**, along with the `TodoEvent.retry_count`
and `EmailJob.retry_count` payload fields that existed only to back it. A trait method
that structurally cannot work is worse than no method: the next person writes
`if job.retry_count() > 3` and it silently never fires.

The fix also closes a case the original code could not express. `ErrorCategory` may
permit more retries than the stream will deliver — `RateLimited` allows 5 while
`EMAILS` sets `max_deliver = 5` — so the worker now checks
`delivery_count >= max_deliver` and DLQs on the final attempt regardless of category.
Without that, the server drops the message one delivery after the policy would have
saved it.

**Verified.** `libs/core/messaging/tests/dlq_it.rs::transient_failure_reaches_the_dlq_after_max_deliver`
publishes one always-transiently-failing job against a real JetStream server and asserts
it arrives in the DLQ with its subject, `job_type` and `delivery_count` intact, and that
the processor was called no more than `max_deliver` times. The test reports 0 entries
against the old code.

---

### 0.5 Undeserializable messages were acked and discarded · S · ✅ FIXED 2026-07-26

**Problem.** `consumer.rs:216-224` handled a deserialization failure by calling
`message.ack()`. On a `WorkQueue` stream — which `EMAILS` is — ack **deletes** the
message, and `fetch` had already dropped the bytes. A producer/consumer schema skew
(rename a field, deploy the producer first) silently ate the in-flight backlog, leaving
a `warn!` with a serde message and no job id, because deserialization is what failed.
The local variable was even named `nak_err` while the call was `ack()`.

**Fixed.** `NatsConsumer::fetch` now returns `Fetched { jobs, poison }`. The consumer
owns no `DlqManager` so it cannot decide their fate; the worker captures the raw bytes
as `DlqPayload::Raw { base64 }` and `term`s the message — redelivery cannot help, the
bytes will not deserialize next time either.

**Verified.** `dlq_it.rs::poison_message_is_captured_not_dropped` publishes valid JSON of
the wrong shape and asserts the processor is never called, a DLQ entry appears with
`job_id: None`, and the original bytes round-trip verbatim.

---

### 0.6 The DLQ was write-only · S · ✅ FIXED 2026-07-26

**Problem.** `DlqManager` exposed only `move_to_dlq`/`stream_info`/`stats`. Nothing in
the repo read a DLQ — no consumer, no redrive, no alert. Worse, `DlqEntry` recorded no
**original subject**, so an entry was unroutable even by hand: `EMAILS` fans across
`emails.welcome`, `emails.password_reset` and five others, and the entry could not say
which. Retention was also `Limits` only by accident, via `..Default::default()`.

**Fixed — and deliberately *not* by adding a DLQ consumer.** A message reaches the DLQ
only after automatic retry is exhausted, so a daemon that reprocesses it is the same
failure on a slower loop, and for a poison pill an infinite one across two streams. What
went in instead:

- **Alarm.** `metrics.dlq_depth()` and `stream_depth()` — both defined since the module
  was written and **never called** — are now published once per batch by the worker. A
  non-zero `nats_worker_dlq_depth` is the alert; a human decides.
- **Route.** `DlqEntry` gains `original_subject`, `job_type` and the server's
  `delivery_count`, and splits its payload into `DlqPayload::{Job, Raw}`.
- **Redrive.** `DlqManager::redrive(start_sequence, limit)` republishes decoded entries
  to their original subject and **skips** poison. Operator-triggered, run after the fix
  is deployed. It reads through an ephemeral consumer, not `direct_get`: sequences go
  sparse as `max_age` expires entries, and `direct_get` needs `allow_direct`, which an
  existing DLQ does not have. Entries are left in place — `Limits` retention is the
  audit trail, and a half-completed redrive must stay diagnosable.
- **Retention is now explicit** with a comment on why the DLQ must *not* go through
  `stream_config_for`: that derives retention from the parent's `StreamKind`, so
  `EMAILS_DLQ` would inherit `WorkQueue` and the first inspection would ack away the
  evidence.

**Verified.** `dlq_it.rs::redrive_returns_jobs_to_their_original_subject_and_skips_poison`.

**Still open:** nothing alerts on the gauge. Wire `nats_worker_dlq_depth > 0` into
whatever watches Prometheus.

---

## P1 — Make the boundary enforced, not documented

Everything good about the `tasks` boundary is currently a claim in markdown. This tier
converts claims into gates. **Do this before adding service #2** — the enforcement is
what carries over; a doc will not.

### 1.1 Promote `CallerAuth` into `libs/core/grpc` · M

**Problem.** `apps/zerg/tasks/src/auth.rs` is ~80 lines of security-critical token
verification that service #2 would copy-paste. `libs/core/grpc/src/server/` contains only
`builder.rs` and `config.rs`; the existing `AuthInterceptor` is **client-side** (attaches
a token to outgoing calls), not server-side verification.

**Fix.** Move to `libs/core/grpc/src/server/auth.rs`, generic over the scope type it
derives (or returning `AuthIdentity` and letting the service map it). Keep the
`#[cfg(test)]` fixed-scope constructor so services can test RPCs without a live IdP.

**Why first.** `zerg_vector` needs the same verification. Two hand-written copies of
token checking is how you eventually get one that is subtly wrong.

**Acceptance.** `zerg_tasks` uses the library version and its tests still pass;
`apps/zerg/tasks/src/auth.rs` is gone.

---

### 1.2 Enforce additive-only proto in CI · S · ✅ FIXED 2026-08-31

**Problem.** The policy is documented in [`grpc.md`](./grpc.md) and the proto uses
`reserved`, but nothing checked a PR. `just proto-breaking` existed and was not wired up.

**The recipe was also broken, which is why "just wire it up" would have failed CI on the
first PR.** It read `cd manifests/grpc && buf breaking --against '.git#branch=main'`, and
buf resolves a `.git` input relative to the **cwd** — so from inside the module directory
it looked for `manifests/grpc/.git` and died with
`fatal: '…/manifests/grpc/.git' does not appear to be a git repository`. It now runs from
the repo root with the module as the input and `subdir` aiming the historical side at the
same `buf.yaml`:

```
buf breaking manifests/grpc --against '.git#branch=main,subdir=manifests/grpc'
```

**Fixed.** Wired into the CI `supply-chain` job (which already had `fetch-depth: 0` and
`buf-setup`), guarded `if: github.event_name == 'pull_request'` — on a push to main the
comparison is main against itself. Also added to `just verify`, with the caveat noted at
the recipe: locally it compares against the **local** `main` ref, so it is only as fresh
as your last fetch and is a no-op while standing on main.

**Acceptance — verified red/green 2026-08-31.** Renumbering `CreateRequest.title` from
`1` to `99` in `tasks.proto` fails the gate with
`Previously present field "1" with name "title" on message "CreateRequest" was deleted.`
(exit 100); reverting turns it green. The `breaking: use: [FILE]` rule was already
configured in `manifests/grpc/buf.yaml`.

---

### 1.3 Enforce the dependency direction in CI · S · ✅ FIXED 2026-08-31

**Problem.** Nothing stopped the next developer adding `domain_tasks` back to `zerg_api`.
The Phase 2 invariant was a grep in a doc.

**Fixed — as a tag-based gate over the whole nx graph, not a `Cargo.toml` grep.** Every
node now carries a `scope:` tag: contributed by `tools/nx/plugin.ts` from the declared
ownership map in `tools/nx/scope-tags.ts` (`apps/<vertical>/**` → its vertical;
`apps/zerg/tasks` + `domain_tasks` → their own `scope:tasks`, because the service
boundary is what 1.3 protects; `libs/**` → `scope:shared` except the zerg-owned domains),
plus the hand-written `tags` of the remaining `project.json` files. The rule, enforced by
`tools/nx/check-boundaries.ts` over `nx graph --file` output: **an edge may stay inside
its scope or point at `scope:shared`; `shared` may only depend on `shared`.** Because the
nx graph carries both cargo edges (@monodon/rust) and TS workspace edges, one gate covers
both ecosystems. Untagged nodes default to `shared` — strictest as a source, so a new
project cannot silently reach into a vertical.

Wired as `just boundaries`, part of `just verify`, and a step in the CI `web` job.

**Found on first run:** `domain_cloud_resources → domain_projects` — the SeaORM FK seam
Issue 5 of [`architecture-review-todo.md`](./architecture-review-todo.md) flags as
undecided. Grandfathered explicitly in `scope-tags.ts` with a pointer to that decision;
removing the entry is how the decision gets enforced once made.

**Acceptance — verified red/green 2026-08-31.** Adding `domain_tasks` to
`apps/zerg/api/Cargo.toml` fails the gate
(`zerg_api (scope:zerg) -> domain_tasks (scope:tasks)`); reverting turns it green
(52 projects, every edge within scope).

---

### 1.4 Run the boundary smoke test · S

**Problem.** `apps/zerg/tasks/tests/boundary_smoke.rs` proves the security property — an
unauthenticated and a forged-token call to `:50051` both get `Unauthenticated` — but it is
`#[ignore]`d and runs nowhere.

**Fix.** Boot the service in CI and run with `--ignored`, or at minimum document the
invocation in [`TESTING_GUIDE.md`](./TESTING_GUIDE.md).

**Acceptance.** Removing the auth check from the service makes CI red.

---

### 1.5 Fix stale `nx.json` `sharedGlobals` · S · ✅ FIXED 2026-08-28

**Problem.** `nx.json:18` referenced `{workspaceRoot}/.github/workflows/ci.yml`, which
does not exist — the workflow is `ci-optimized.yml`. Cache invalidation on CI changes was
silently not happening.

**Fix.** Path now points at `ci-optimized.yml`.

**Acceptance.** Path points at a file that exists. ✅

---

## P2 — Deployment truth

The split is a claim until it deploys. These gaps were introduced by Phases 3–5.

### 2.0 The tasks database is not created by `docker compose` · S · ✅ FIXED 2026-07-26

**Problem.** Phase 3 added `CREATE DATABASE tasks;` to
`manifests/dockers/config/postgres-init/01-create-databases.sql`, but compose mounts a
**sibling** directory — `manifests/dockers/compose.yaml:54` mounts `./postgres-init`,
not `./config/postgres-init`. The whole `config/` file was dead: `terran` was never
created by compose either. A fresh `docker compose up` therefore produced a Postgres
with no `tasks` database, and `zerg_tasks` failed at startup until someone ran
`just db-fresh tasks`.

**Fixed.** `tasks` and `terran` joined the loop in the *mounted*
`manifests/dockers/postgres-init/10-create-extra-databases.sh`, and the dead
`config/postgres-init/` file was deleted rather than left as a second, wrong source of
truth. Nothing referenced it (`grep -rn config/postgres-init` is empty).

### 2.1 Point `zerg_tasks` at the tasks database · S · ✅ FIXED 2026-07-26

**Problem — worse than the original entry described.** `zerg-tasks-config` never carried
`DATABASE_URL` at all. The tasks deployment's `envFrom` pulled **`zerg-api-secrets`**
(`apps/zerg/tasks/k8s/kustomize/base/deployment.yaml:49-51`), which supplies
`DATABASE_URL=…/zerg` plus `WORKOS_API_KEY`. The intended override, `zerg-tasks-secrets`,
was `optional: true` and **generated nowhere in the repo**, so it silently resolved to
nothing. A deployed `zerg_tasks` would have connected to the **zerg** database — where
Phase 3 had just dropped the `tasks` table. Every RPC would fail with
`relation "tasks" does not exist`, and the service would have held the WorkOS management
API key it has no business holding.

**Fixed.** `zerg-api-secrets` is gone from the tasks deployment and `zerg-tasks-secrets`
carries the tasks `DATABASE_URL` + `WORKOS_CLIENT_ID` only. The `optional: true` was
dropped deliberately: a missing credential must fail the pod, not fall through to
whatever another service happened to mount.

**Where both halves live now.** The kustomize tree named above is gone — an app's k8s
surface is its `butler.toml`. The mount is `apps/zerg/tasks/butler.toml`:

```toml
[[workload.envFrom]]
kind = "secret"
name = "zerg-tasks-secrets"
optional = false
```

and the dev-only literals are one file for every zerg app,
`manifests/k8s/dev/app-secrets.yaml`, pulled into the generated aggregate through the
root `butler.toml` `[k8s] extraResources`.

**Verified.** `kubectl kustomize manifests/k8s/apps` renders the `zerg-tasks` Deployment
with no `zerg-api-secrets` reference and decodes `DATABASE_URL` to `…/tasks`.

### 2.2 Give `zerg_tasks` `WORKOS_CLIENT_ID` · S · ✅ FIXED 2026-07-26

Supplied by `zerg-tasks-secrets` (2.1). Verification-only — no `WORKOS_API_KEY`, matching
`apps/zerg/tasks/src/config.rs:20-30`, which requires the client id and derives the
issuer.
**Acceptance:** deployed service logs `Caller token verification enabled` with the right issuer.

### 2.3 Provision the tasks database in the cluster · M · 🟡 PARTIAL

`tasks-shared-secrets` now exists (`manifests/db/tasks/k8s/overlays/dev/kustomization.yaml`)
and `kubectl kustomize manifests/db/tasks/k8s/overlays/dev` renders a complete
ConfigMap + Secret + AtlasSchema. **Still open:** CNPG must actually create the `tasks`
database in-cluster — the compose fix in 2.0 covers local only.

**Also open — prod secrets.** Both new secrets exist for **dev** only.
`apps/zerg/tasks/butler.toml` needs an `[env.prod.externalSecret]` table — the treatment
`apps/zerg/api/butler.toml` already gives `zerg-api-secrets`, which the `app` package
renders as an `ExternalSecret` against `gcp-secret-manager` — and
`manifests/db/tasks/k8s/overlays/prod` needs the equivalent of
`apps/zerg/shared/k8s/kustomize/overlays/prod/external-secret.yaml` (that one is still
hand-written kustomize: it belongs to no app, so butler does not own it). Note the 2.1
fix made `zerg-tasks-secrets` **non-optional**, so a prod deploy now fails the pod
outright instead of silently inheriting the zerg database URL. That is the intended
failure mode, but it is a hard blocker on the next prod apply.
**Acceptance:** Atlas reconciles `tasks-schema` against a live cluster database.

### 2.4 Enforce database isolation with grants, not connection strings · M

**Problem.** "`zerg_api` has no access to the tasks database" is currently true only
because it lacks the connection string. `manifests/db/tasks/roles.sql` creates `tasks_app`,
but zerg has no `roles.sql` and local dev runs as superuser `myuser`.

**Acceptance.** Connecting as the zerg app role to the tasks database is refused by Postgres.

---

## P3 — Vector + GCS (next feature)

Design rationale in
[`communication-and-consistency.md`](./communication-and-consistency.md).

### 3.1 Collapse the vector service to one door · M

**Problem.** Two doors to Qdrant, and the gRPC one is dead: `VectorServiceClient` appears
nowhere in the repo, while `apps/zerg/api/src/main.rs:93` builds its own `QdrantRepository` and
serves `/api/vector/*` in-process. This is `/api/tasks-direct` again.

**Fix (given vector work is starting).** Point `/api/vector/*` at `zerg_vector` over gRPC
and delete the in-process repository from `zerg_api`. Depends on **1.1** so the service
gets verified identity from the library rather than a second hand-written copy.

*Alternative if vector work slips:* delete `apps/zerg/vector` and re-introduce it via the
playbook (**4.1**) when the feature actually starts. Do not leave both doors open.

**Acceptance.** `grep -rn QdrantRepository::new apps/zerg/api` is empty; vector endpoints work.

### 3.2 Split vector's sync and async paths · M

`search` is request/response (gRPC). `embed`/ingestion is slow, bursty and retryable
(NATS). Today `libs/domains/vector/src/service.rs:166` runs `embed` on the request path,
which is the wrong shape for document ingestion.

**Target:** one `zerg_vector` binary, two entry points — gRPC search, plus a NATS consumer
for ingestion. Both own Qdrant.

### 3.3 GCS access as a library, with signed URLs · M

- `libs/core/storage` wrapping GCS. **Not a service** — GCS is the datastore, so a service
  wrapping it would own no data of its own.
- **Do not proxy bytes through the API.** Mint a signed URL; the browser uploads directly
  to GCS. This removes upload bandwidth, request timeouts and memory pressure from every
  service, and is painful to retrofit.

**Acceptance.** A large file upload never transits `zerg_api`.

### 3.4 Ingestion pipeline · L

`DocumentUploaded` **event** (not a `queue_embedding_job` command — search indexing and
thumbnailing will want it later) → NATS → `zerg_vector` consumer: fetch, extract, chunk,
embed, upsert to Qdrant.

**This is a genuine outbox case (see 5.2).** Losing the message means a document that is
uploaded but never searchable — silent and confusing, unlike a lost welcome email.

---

## P4 — Reference artifact

### 4.1 `docs/adding-a-grpc-service.md` playbook · M

The ordered checklist — **contract → data → auth → process** — derived from the tasks
work, with the boundary checklist as acceptance criteria. Goal: service #2 takes a day,
not five phases of archaeology.

Must cover: contract crate layout, the `orm` feature-gate trick for shared enums, own
database + `roles.sql`, token verification via **1.1**, `reserved` discipline, `/ready`
must not gate on it, per-call deadlines, and the boundary smoke test.

---

## P5 — Hygiene and follow-ups

### 5.1 Extract `libs/contracts/todo` · S

`apps/todo/worker/Cargo.toml:17` depends on `domain_todo` purely to name `TodoEvent`
(`apps/todo/worker/src/lib.rs:12`) — the shared-kernel pattern Phase 2 removed from tasks, over NATS
instead of gRPC. **A JSON event payload is a wire contract too.** Milder than the tasks
case (the worker inherits no repository), so lower priority — but the same class of bug.

**Rule of thumb:** contract crates are justified by a *serialization boundary between
independently deployed processes* — not by having a domain, and **not** by exporting
TypeScript types. `domain_todo` correctly exports `@domain/todo` to `apps/todo/web` with
no contract crate. `users` and `cloud_resources` need none.

`projects` gained one in **0.3** — `libs/contracts/projects`, holding `ProjectDeleted`
and the `PROJECTS` stream identity — which is this rule applied, not an exception to it:
the crate exists for the event crossing to `zerg_tasks`, and `Project` itself stays out
of it. That is also the worked example this item should copy: same shape, `TodoEvent`
instead, with `apps/todo/worker` depending on the contract rather than `domain_todo`.

### 5.2 Transactional outbox · M

No outbox exists. Add one when the first must-not-lose message appears — **3.4** is the
likely trigger. The welcome-email dual write in `establish_session` is deliberately
best-effort and does not need it.

### 5.3 Retry policy on the tasks client · S

`libs/core/retry` exists and is unused here. A 5 s deadline with no retry means one blip
is a user-visible 503.

### 5.4 `POST /api/org` — idempotency + compensation · M

Four steps across WorkOS, Redis and Postgres with no transaction
(`apps/zerg/api/src/api/org.rs:117,141,161,167`). Failure after step 1 leaves an orphan
org. Move to consistency rungs 1–2: make org creation idempotent, compensate a failed
membership by deleting the org. No new infrastructure.

### 5.5 Batch resolvers for cross-service display data · S

`find_by_subjects` / `find_by_ids` do not exist on `domain_users`/`domain_projects`.
Needed before any UI renders a task's owner or project name. Must be **batched** (one
query, not N) and must render a missing ref gracefully — there is no referential
integrity guaranteeing the row still exists.

**This is the read-side half of 0.3 and is still open.** 0.3 closed the write side (a
`ProjectDeleted` consumer nulls the refs), but that correction is *eventually*
consistent and its publish is best-effort, so a resolver can still be handed an id that
resolves to nothing. Nothing renders a project name today, which is why 0.3 needed no
read-side change to be complete — the moment one does, "missing ref renders as no
project" is a requirement, not a nicety.

### 5.6 Team-boundary tooling · S · ✅ FIXED 2026-08-31

Both rungs landed the same day as **1.3**, because they are the same fact expressed
twice:

- **`scope:` tags** on every graph node — the ownership map is
  `tools/nx/scope-tags.ts`, contributed through the plugin (verticals for `apps/**`,
  `scope:tasks` for the extracted service + its domain, `scope:shared` for libs, with
  the zerg-owned domains called out), enforced by `just boundaries` (see 1.3).
- **`.github/CODEOWNERS`** — one human owns everything today, but the per-path rows
  mirror the scope map (verticals, the tasks boundary, shared platform), so handing a
  vertical to another team is editing owner handles, not inventing structure. The file
  is **inert until user #2**: GitHub never requests review from a PR's own author. The
  ordered activation runbook — write access before ownership, the last-match-wins row
  order, making it blocking via a ruleset, and the `release.yml` direct-push conflict
  that ruleset creates — is
  [`todo/codeowners-activation.md`](./todo/codeowners-activation.md).

CI already runs `nx affected -t lint/test/build`, so per-project isolation is in place.

### 5.7 `zerg_vector` deployment artifacts · S

The crate has a binary and now ships an image — root `butler.toml` `[container] extra`
lists `apps/zerg/vector`, so it is in the 12-app container/scan set — but it declares no
`[workload]`, so it has no Tiltfile, no manifests, and does not deploy. Only needed if
**3.1** resolves toward keeping the service.

---

## Suggested order

1. **P0** — live defects, all small
2. **1.1** then **1.2–1.5** — enforcement; 1.1 first because 3.1 depends on it
3. **P2** — deployment truth, before the next deploy
4. **P3** — vector + GCS feature work
5. **4.1** — write the playbook while the tasks work is still fresh
6. **P5** — as convenient

## Not doing (decided)

- **Temporal** — two short saga flows do not justify a workflow cluster. Revisit at 5+
  multi-step cross-system flows, or flows lasting days/weeks. Reasoning and adoption
  triggers in [`communication-and-consistency.md`](./communication-and-consistency.md#6-temporal-evaluation).
- **Contract crates for in-process domains** — see 5.1.
- **Extracting `projects` / `users` / `cloud_resources`** — no runtime trigger. See
  [`modular-monolith-architecture.md`](./modular-monolith-architecture.md#migration-to-microservices).
- **Cascade deletes for tasks** — orphan user/org rows accepted; account deletion is not
  implemented. Note this does **not** cover 0.3, which is reachable today.
