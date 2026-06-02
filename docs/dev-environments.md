# Development Environments, Platform Architecture & Design Patterns

> **What this is:** the canonical map of how `nx-playground` (the *zerg* stack) is developed and
> run, across three environments of increasing fidelity, plus the platform that backs them and the
> design patterns that hold it together. It also specifies two things that are *staged but not yet
> built*: the **GKE + mirrord** environment (#3) and the **Flagsmith / CMS / identity** integration.
>
> **Live cluster observed:** `gke_bootstrap-491220_me-west1_paidevo-cluster` (me-west1), 2026-05-31.

---

## TL;DR — the parity ladder

Each environment trades **fidelity** for **iteration speed**. Pick the lowest rung that still
reproduces your problem.

| # | Environment | What runs where | Iteration speed | Fidelity | Use it for |
|---|-------------|-----------------|-----------------|----------|------------|
| 1 | **Docker Compose + bacon** | App on host (hot-reload), infra in Docker | ⚡⚡⚡ fastest | low | Inner loop: business logic, handlers, SQL, unit work |
| 2 | **Kind + Tilt + Kustomize** | App in local k8s, infra via operators (CNPG/Atlas) | ⚡⚡ medium | medium | Manifests, probes, migrations, k8s wiring, HPA |
| 3 | **GKE + mirrord** *(to build)* | App on host, *plugged into* the real cluster | ⚡⚡ medium | **high** | Debugging against real platform services (Flagsmith, NATS, Istio, mesh) |

```
fidelity ───────────────────────────────────────────────────►
  compose+bacon (1)        kind+tilt (2)            gke+mirrord (3)
  └ host process           └ in-cluster pods        └ host process, cluster network/env/secrets
  └ docker infra           └ operator-managed DBs   └ real CNPG/KubeBlocks/NATS/Flagsmith
  └ no k8s                 └ kustomize overlays      └ Istio mesh, external-secrets, Flux
◄─────────────────────────────────────────────────── iteration speed
```

---

## The platform (GKE) — capability map

The `paidevo` cluster is a full GitOps platform; the application is a small tenant inside it
(`zerg-dev` namespace). The application repo (`nx-playground`) owns app code + app/DB manifests;
the **`gitops-v1`** repo (referenced at `scripts/nu/mod.nu:15`) owns the platform via **Flux**.

| Capability | Component(s) | Namespace | Notes |
|------------|--------------|-----------|-------|
| **GitOps delivery** | Flux (source/kustomize/helm/notification controllers) | `flux-system` | Reconciles platform + app from `gitops-v1` |
| **IaC control plane** | Crossplane + GCP/GitHub/Helm/K8s/HTTP providers, komoplane UI | `crossplane-system` | Cloud resources as CRDs; KCL composition functions |
| **Service mesh + gateway** | Istio 1.29 (ambient/sidecar), Gateway API (`main-gateway`, `34.165.116.218`), agentgateway | `istio-system`, `gateway` | Host pattern `*.platform.yurikrupnik.com` |
| **Progressive delivery** | Flagger | `istio-system` | Canary/blue-green — pairs with feature flags |
| **Databases (operators)** | CloudNativePG, KubeBlocks (+ mysql/mongo/kafka/etcd/**redis**/**qdrant**/pg addons) | `cnpg-system`, `kb-system`, `dbs` | App data instances live in `dbs` |
| **Schema/migrations** | Atlas Operator | `atlas-operator` | `AtlasMigration` CR per DB (see `manifests/db/`) |
| **Messaging** | NATS (JetStream) | `nats-system` | Backs `zerg-email-nats` |
| **Secrets** | External Secrets Operator → GCP Secret Manager | `external-secrets` | `ExternalSecret` → k8s `Secret` |
| **Autoscaling** | KEDA, metrics-server, Goldilocks (right-sizing) | `keda`, `goldilocks` | Event-driven + VPA recommendations |
| **Policy** | Kyverno + Policy Reporter | `kyverno` | Admission policy + reports |
| **Observability** | kube-prometheus-stack, Grafana, Loki, Tempo, Alloy, OTel Operator + collector, GMP | `monitoring`, `gmp-*`, `opentelemetry-operator-system` | Metrics + logs + traces; OTLP collector present |
| **Identity** | Keycloak | `keycloak` | OIDC/SSO — the "others" for app auth |
| **Feature flags** | **Flagsmith** (`flagsmith-api:8000`, frontend:8080, task-processor) | `flagsmith` | Installed; **not yet consumed by the app** |
| **Headless CMS** | **Directus** (`directus:8055`) | `cms` | Installed; **not yet consumed by the app** |
| **Testing / lifecycle** | Testkube, Keptn | `testkube`, `keptn-system` | In-cluster tests + SLO orchestration |
| **Backup/DR** | Velero | `velero` | Cluster backup |
| **TLS** | cert-manager | `cert-manager` | Gateway/ingress certificates |

### The application tenant (`zerg-dev`)

```
                         Istio Gateway (main-gateway, 34.165.116.218)
                         zerg.platform.yurikrupnik.com
                                   │
                                   ▼  HTTPRoute (zerg-dev/zerg-web)
   ┌──────────────────────────────────────────────────────────────────┐
   │ ns: zerg-dev                                                       │
   │   zerg-web (nginx :8080)  ──►  zerg-api (:8080, REST+gRPC client)  │
   │                                   │      │                          │
   │                                   │      └──gRPC──► zerg-tasks (:50051)
   │                                   │                                 │
   │   zerg-email-nats (worker, health :8081)   mailhog (:8025)          │
   │   config: zerg-shared-config + zerg-api-config (ConfigMaps)         │
   │   secrets: zerg-*-secrets (ExternalSecret → GCP SM)                 │
   └───────────────┬───────────────────┬───────────────┬───────────────┘
                   │                    │               │
        ns: dbs    ▼          ns: nats-system ▼  ns: flagsmith ▼   ns: cms
   shared-postgres-rw:5432   nats:4222        flagsmith-api:8000  directus:8055
   redis-redis-redis:6379    (JetStream)      (feature flags)     (CMS)
   qdrant-qdrant:6333/6334
```

App delivery to GKE is **GitOps**: images are built in CI (`.github/workflows`), and Flux reconciles
the `zerg-dev` overlay (`apps/zerg/api/k8s/kustomize/overlays/dev`, etc.). The Atlas Operator runs
DB migrations *before* the app Kustomization is allowed to become Ready.

---

## Environment 1 — Docker Compose + bacon (inner loop)

**Goal:** sub-second feedback on Rust changes with zero Kubernetes.

```
host:  bacon (cargo run -p …, kill-then-restart on change)  ── mprocs panes ──┐
       zerg-api :8080   zerg-tasks :50051   zerg-email-nats   zerg-web :5173  │
docker: postgres :5432  redis :6379  nats :4222/8222  qdrant :6333  mailhog :8025  influxdb :8086
```

| File | Role |
|------|------|
| `manifests/dockers/compose.yaml` | Infra: Postgres 18 (tuned), Redis, NATS+JetStream, Qdrant, Mailhog, Mongo, InfluxDB |
| `bacon.toml` | Per-app hot-reload jobs (`zerg-api`, `zerg-tasks`, `zerg-email-nats`, …), `watch` globs incl. `libs` |
| `manifests/mprocs/local.yaml` | mprocs multiplexer: one pane per service, each runs `bacon <job>` with `DATABASE_URL` set |
| `.env` (gitignored) / `.env.example` | 12-factor env contract loaded via `direnv` (`.envrc`) |

**Commands**

```bash
just _docker-up            # docker compose up -d  (infra)
just db-fresh zerg         # drop+recreate zerg, apply schema.sql + seed.sql
just migrate zerg          # apply pending sqlx-style migrations locally
just dev                   # mprocs -c manifests/mprocs/local.yaml  (bacon hot-reload, all apps)
just run zerg-api          # or a single app via bacon
just web                   # bun nx dev zerg-web
just docker-down           # tear down infra
```

**Tradeoffs**
- ✅ Fastest loop; no cluster, no images, no port-forwards.
- ✅ DB lifecycle matches prod path (`schema.sql` is source of truth; `migrations/` is the path).
- ❌ No Istio, no probes/HPA, no external-secrets, no Flagsmith/Directus, no mesh behavior.
- ⚠️ Infra credentials are local dev defaults (`myuser/mypassword`); never reuse outside dev.

**Cleanups noticed** — `bacon.toml` has stale jobs (`terran-api`, `zerg-cli`, `zerg-operator`,
`zerg-email` pointing at non-existent `apps/zerg/email`). Prune to the four live binaries.

---

## Environment 2 — Kind + Tilt + Kustomize (k8s parity, local)

**Goal:** exercise the *Kubernetes* surface — manifests, probes, operator-driven DBs, migrations,
service DNS — on a throwaway local cluster.

Two entry points:

- **`just local-up`** → `scripts/nu/mod.nu up` (the orchestrator): creates the Kind cluster,
  app namespaces, External Secrets (from `~/dotconfig/tmp/secret-puller.json`), deploys DBs,
  optionally bootstraps Flux (`--flux`), then runs `tilt up`. Tear down with `just local-down`,
  fast cycle with `just local-restart`, inspect with `just local-status`.
- **`just dev-kind`** → `manifests/mprocs/kind.yaml`: assumes a cluster exists, waits for the CNPG
  cluster, port-forwards Postgres to `:5433`, then `tilt up`.

**What Tilt manages** (`Tiltfile`):
- `kustomize('./manifests/k8s/overlays/dev')` + `kustomize('./manifests/db/zerg/k8s/overlays/dev')`
- Port-forwards: Postgres `:5432`, Redis `:6379`, Mailhog `:8025`, Istio gateway `:8080/:8443`, Kiali `:20001`
- A `local_resource` that regenerates the migrations ConfigMap (`just gen-migrations-configmap zerg`) on change
- `live_update` for each app via the per-app `apps/zerg/*/Tiltfile`

| Endpoint | URL |
|----------|-----|
| Tilt UI | http://localhost:10350 |
| API | http://localhost:5221/api |
| Web | http://localhost:5173 |
| Postgres | localhost:5433 (CNPG) / 5432 (kompose) |

**Tradeoffs**
- ✅ Real manifests, probes, service DNS, Atlas migrations, CNPG — catches k8s wiring bugs.
- ✅ Closest *self-contained* parity; works offline.
- ❌ Kind ≠ GKE: no Istio mesh by default, simplified DBs (kompose Deployments vs CNPG/KubeBlocks),
  no Flagsmith/Directus/Keycloak unless you also reconcile the platform repo.
- ⚠️ Heavier; rebuild/redeploy cycle is seconds-to-minutes, not sub-second.

**Cleanups noticed** — the `backstage-*` recipes in `justfile` point at
`manifests/kustomize/backstage/…`, but the tree uses `manifests/k8s/`. Either path is stale or the
overlay is missing; reconcile before relying on those recipes.

---

## Environment 3 — GKE + mirrord (high-fidelity, **to build**)

**Goal:** run the Rust process **on your laptop** (native debugger, instant rebuilds) while it
behaves as if it were the `zerg-dev/zerg-api` pod — same env vars, same secrets, same in-cluster DNS,
same network reachability to **Flagsmith, NATS, CNPG, Redis, Qdrant, zerg-tasks**, and the Istio mesh.
No port-forwards, no fake config. `mirrord` is already installed locally.

### How it works

`mirrord` injects an ephemeral agent next to a target pod and bridges your local process into that
pod's context:

- **env:** inherits the pod's resolved env (ConfigMaps + Secrets — incl. the already-injected
  `FLAGSMITH_API_URL`, `DATABASE_URL`, `REDIS_URL`, `TASKS_SERVICE_ADDR`).
- **network (outgoing):** local `connect()`/DNS resolve **through the cluster**, so
  `flagsmith-api.flagsmith.svc.cluster.local:8000`, `shared-postgres-rw.dbs:5432`,
  `redis-redis-redis.dbs:6379`, `nats.nats-system:4222`, `zerg-tasks.zerg-dev:50051` all just work.
- **network (incoming):** `mirror` (copy real traffic to your process, read-only) or `steal`
  (intercept it). Use a **header-filtered steal** in a shared cluster so you only capture *your* requests.
- **fs:** read files from the pod image; writes stay local.

### Proposed config — `.mirrord/zerg-api.json`

```json
{
  "target": { "path": "deployment/zerg-api", "namespace": "zerg-dev" },
  "agent": { "namespace": "zerg-dev", "ttl": 60 },
  "feature": {
    "env": true,
    "fs": "read",
    "network": {
      "dns": true,
      "outgoing": true,
      "incoming": {
        "mode": "steal",
        "http_filter": { "header_filter": "x-dev-user: .*" }
      }
    }
  }
}
```

- **Mirror-first for safety:** start with `"incoming": "mirror"` (observe, never disrupt shared dev),
  switch to header-filtered `steal` only when you need to serve responses.
- **`http_filter`** is the critical guardrail in a *shared* `zerg-dev`: only requests carrying your
  header (e.g. `x-dev-user: yuri`) divert to your laptop; everyone else hits the real pod.
- Add a sibling `.mirrord/zerg-tasks.json` (`target: deployment/zerg-tasks`, port 50051) and
  `.mirrord/zerg-email-nats.json` (`target: deployment/zerg-email-nats`) for those binaries.

### Proposed `justfile` recipes

```just
# Run a zerg app locally, plugged into the GKE zerg-dev namespace via mirrord.
# Usage: just dev-gke api      (api|tasks|email-nats)
dev-gke app:
    mirrord exec -f .mirrord/zerg-{{app}}.json -- cargo run -p zerg_{{ replace(app, "-", "_") }}

# Observe-only (mirror): never disrupts shared traffic.
# Uses a sibling config whose feature.network.incoming is "mirror" instead of the steal block.
dev-gke-mirror app:
    mirrord exec -f .mirrord/zerg-{{app}}.mirror.json -- cargo run -p zerg_{{ replace(app, "-", "_") }}
```

### Caveats & guardrails

- **Istio sidecar:** target pods are meshed. `mirrord` auto-detects Istio; `steal` works because mesh
  mTLS is **not** `STRICT` here (see `docs/engineering-review.md` #10). If mTLS is later set to
  STRICT, verify the agent can intercept (mirrord supports meshed steal but it's the thing to test).
- **Shared namespace:** `zerg-dev` is shared. *Always* mirror or header-filter steal; an unfiltered
  steal black-holes everyone's traffic to your laptop.
- **Writes hit real data:** outgoing traffic reaches the **real** `dbs` Postgres/Redis and **real**
  NATS subjects. Treat it as a shared dev datastore; prefer a per-developer Flagsmith trait/segment
  and avoid destructive operations. For isolation, point `DATABASE_URL` at a scratch DB via
  `--override-env-vars` on the mirrord invocation.
- **RBAC:** `mirrord` needs permission to create the agent pod in `zerg-dev`. Bind a dev Role
  (`pods`, `pods/log`, `pods/exec`, `ephemeralcontainers`) for the developer's GCP identity.
- **Onboarding:** add `mirrord` to the README prerequisites table and to `check-prerequisites` in
  `scripts/nu/mod.nu:27`.

**Tradeoffs**
- ✅ Highest fidelity short of deploying: real Flagsmith eval, real mesh, real secrets, native debugger.
- ✅ No image build/push; edit-compile-run on the host.
- ❌ Requires cluster connectivity + RBAC; not offline.
- ❌ Shared-namespace blast radius if misused (mitigated by mirror/filter).

---

## Data, schema & migrations (the "DB tool")

One folder per database under `manifests/db/<db>/` (`db.just` recipes imported by the root
`justfile`). **`schema.sql` is the source of truth; `migrations/` is the reversible path to it.**

```
manifests/db/zerg/
├── schema.sql                 # canonical desired state (PG18; uuidv7() built-in)
├── seed.sql                   # local-dev only
├── migrations/<ts>_<name>.{up,down}.sql
└── k8s/{base,overlays/{dev,prod}}/   # AtlasMigration CR + generated migrations ConfigMap
```

| Layer | Tool | Where |
|-------|------|-------|
| Author migrations | `just migrate-add <db> <name>` | host |
| Apply locally | `sqlx` via `just migrate <db>` / `db-fresh` | Compose Postgres `:5432` |
| Validate | `just migrate-validate <db>` (diff schema vs migrations), `just migrate-test <db>` | host |
| Generate cluster artifact | `pg-cli` via `just gen-migrations-configmap <db>` (committed) | host |
| Apply in-cluster | **Atlas Operator** reads the ConfigMap, applies in version order, gates the app | GKE/Kind |
| Provision DB | **CloudNativePG** (`shared-postgres-rw`), KubeBlocks for redis/qdrant | `dbs` |
| Deliver | **Flux** Kustomization per DB (in `gitops-v1`) | GKE |

This is a clean **declarative-desired-state + versioned-path** design: the same migration files drive
local `sqlx` and the in-cluster Atlas Operator, so the local DB and prod DB converge through one path.

---

## Feature flags, CMS & identity — integration design

**Current state:** the *infra* is live (Flagsmith, Directus, Keycloak namespaces) and the *config* is
staged — `FLAGSMITH_API_URL` is already injected into `zerg-api` via
`apps/zerg/api/k8s/kustomize/overlays/{dev,prod}/kustomization.yaml`, `FLAGSMITH_ENVIRONMENT_KEY` is
stubbed in the `ExternalSecret` (`overlays/dev/external-secret.yaml:34`), and `.env.example` and a
commented `flagsmith = "2.1.0"` dep (`Cargo.toml:51`) exist. **But no Rust code consumes any of it.**
This section specifies how to wire it, following the repo's existing ports-&-adapters style.

### Pattern: Ports & Adapters (hexagonal), same shape as email/embedding providers

The codebase already uses provider traits with swappable adapters
(`libs/notifications/email/src/provider/{smtp,sendgrid,mock}.rs`,
`libs/domains/vector/src/embedding/{openai,vertexai}.rs`). Feature flags and CMS should follow suit so
every environment can pick an adapter — **real in #3/GKE, a static/noop adapter in #1/#2 where
Flagsmith isn't running.**

**Proposed crate: `libs/core/feature-flags`**

```rust
/// Port: what the app depends on (object-safe, async).
#[async_trait]
pub trait FeatureFlags: Send + Sync {
    /// Boolean gate, evaluated for an optional identity (user-targeted).
    async fn is_enabled(&self, flag: &str, ctx: &FlagContext) -> bool;
    /// Typed remote config value with a caller-provided default.
    async fn value<T: DeserializeOwned>(&self, flag: &str, ctx: &FlagContext, default: T) -> T;
}

/// Identity + traits used for targeting/segments (built from JwtClaims).
#[derive(Default)]
pub struct FlagContext {
    pub identity: Option<String>,      // user id / "anonymous"
    pub traits: HashMap<String, Value> // email_domain, plan, roles, ...
}

/// Adapter 1 — real Flagsmith, server-side LOCAL EVALUATION (no per-request HTTP).
pub struct FlagsmithProvider { /* flagsmith::Flagsmith with local-eval + polling + default handler */ }

/// Adapter 2 — static map from env/file. Used in compose/kind and tests; never network.
pub struct StaticProvider(HashMap<String, bool>);
```

**Key design decisions**

- **Server-side *local evaluation* mode.** Use the Flagsmith server-side environment key and the
  SDK's local-eval (it fetches the environment document and evaluates in-process, refreshing on a
  poll interval). This gives ~µs evaluations, survives Flagsmith blips, and keeps user identity on
  the server. *Do not* use the client-side key or per-request remote calls in a hot path.
- **Fail-open with explicit defaults.** Every call takes a default; a `default_flag_handler` returns
  it if Flagsmith is unreachable. Flags must never be able to take the API down (contrast the panic
  surface in `docs/engineering-review.md` #5).
- **Identity-aware extractor.** Add an axum extractor that builds `FlagContext` from `JwtClaims`
  (`sub`, `email` domain, `roles`) so flags can target users/segments — and degrade to "anonymous"
  when unauthenticated.
- **Selection by env, like the email provider** (`apps/zerg/api/src/main.rs` builds providers from
  config): `FlagsmithProvider` when `FLAGSMITH_ENVIRONMENT_KEY` is set, else `StaticProvider`
  (compose/kind/CI). Store `Arc<dyn FeatureFlags>` in `AppState`.

**Usage**

```rust
// In a handler — gate behavior:
if state.flags.is_enabled("tasks.streaming_api", &ctx).await {
    return stream_tasks(...).await;
}
// Typed remote config (e.g. server-driven pagination cap — also fixes review #7):
let max = state.flags.value::<u64>("api.max_page_size", &ctx, 1000).await;
```

**Config & secret flow** (already half-wired):
1. `FLAGSMITH_ENVIRONMENT_KEY` → GCP Secret Manager → uncomment in `overlays/*/external-secret.yaml`
   → `zerg-api-secrets` → pod env. 2. `FLAGSMITH_API_URL` is already in the ConfigMap. 3. In #3 (mirrord)
   both arrive automatically from the pod. 4. In #1/#2, leave the key unset → `StaticProvider`.

**Operational tie-in:** Flagsmith + **Flagger** (already installed) is the canonical
**progressive-delivery** pair — ship dark behind a flag, canary the rollout, flip the flag per segment.

### Directus (CMS) — content as a service

Directus (`directus.cms.svc.cluster.local:8055`) is a headless CMS exposing REST + GraphQL.
Mirror the same pattern: a `ContentProvider` port in (e.g.) `libs/notifications/email` consumers or a
small `libs/core/content` crate, with a `DirectusProvider` adapter over `reqwest` (already a dep) and
a static/fixture adapter for local. Use it for things like email templates, marketing copy, or
feature descriptions — content the app renders but product/ops should edit without a deploy. Add
`CMS_API_URL` (`http://directus.cms.svc.cluster.local:8055`) + a service token via external-secrets,
following the Flagsmith contract exactly.

### Keycloak (the "others") — federated identity

Keycloak (`keycloak.platform.yurikrupnik.com`) can become the OIDC issuer behind the existing OAuth
flow (`libs/domains/users/src/oauth`). Today the app does its own Google/GitHub OAuth + local JWT;
the cleaner long-term model is Keycloak as the single IdP (it federates Google/GitHub), the app
validating Keycloak-issued OIDC tokens (`iss`/`aud`/JWKS) instead of minting HS256 tokens itself.
This also closes the JWT issues in `docs/engineering-review.md` (RS256/JWKS, real `aud`/`iss`). Treat
as a roadmap item, not a quick wire-up.

---

## Design patterns catalog (and where they live)

| Pattern | Where in this repo | Why |
|---------|--------------------|-----|
| **Modular monolith** | `libs/domains/*` as independent crates; one deployable API | Microservice-ready boundaries without distributed-systems tax |
| **Ports & Adapters (hexagonal)** | `Repository`/`*Provider` traits + Pg/SMTP/SendGrid/OpenAI/Vertex impls; *(proposed)* `FeatureFlags`/`ContentProvider` | Swap real ↔ static per environment; testability |
| **Repository + generic base** | `libs/database/src/repository.rs` (`BaseRepository<E>`), per-domain `postgres.rs` | Reuse CRUD, isolate persistence |
| **Service layer** | `libs/domains/*/service.rs` | Business rules + the ownership-aware methods auth should call |
| **Strategy** | email providers, embedding providers, *(proposed)* flag providers | Behavior chosen by config/env |
| **Newtype config + `FromEnv`** | `libs/core/config`, per-crate `*Config` | 12-factor, validated-at-startup config |
| **Compile-time codegen (derive macros)** | `api_resource`, `sea_orm_resource`, `selectable_fields` | Kill boilerplate (URLs/tags/field policy) |
| **At-least-once worker + DLQ** | `libs/core/messaging/src/nats/*` | Reliable async processing with backpressure |
| **Operator / sidecar** | Atlas Operator, CNPG, External Secrets, OTel Operator | Declarative, self-healing infra concerns |
| **GitOps** | Flux reconciling `gitops-v1` + app overlays | Git as the single source of truth for state |
| **IaC control plane** | Crossplane + KCL composition functions | Cloud resources as Kubernetes CRDs |
| **Progressive delivery** | Flagger + *(proposed)* Flagsmith | Decouple deploy from release |
| **Environment parity ladder** | compose → kind → gke+mirrord | Match fidelity to the bug; keep the inner loop fast |
| **Declarative desired-state + versioned path** | `schema.sql` + `migrations/` (sqlx local, Atlas in-cluster) | One convergence path, local and prod |
| **Strangler-fig / extraction** | domain crates can graduate to services | Documented migration path to microservices |

---

## Cross-cutting suggestions

**Build environment #3 (highest leverage right now).** Add `.mirrord/*.json`, the `dev-gke` recipes,
RBAC for the dev identity, and a mirror-first default. It turns the already-running `zerg-dev`
deployment into a live debug target and unlocks real Flagsmith/NATS/mesh testing without a deploy.

**Wire Flagsmith for real.** The config is staged; add `libs/core/feature-flags` (ports & adapters,
local-eval, fail-open), uncomment the `ExternalSecret` key and the `Cargo.toml` dep, and select the
provider by env like the email provider. First flag to ship: server-driven `api.max_page_size`
(also fixes the unbounded-list issue in `docs/engineering-review.md` #7).

**Then Directus, then Keycloak**, in that order — each reuses the same provider/env/secret contract.

**Hygiene found while mapping (low effort, high signal):**
- `bacon.toml` — remove dead jobs (`terran-api`, `zerg-cli`, `zerg-operator`, `zerg-email`).
- `justfile` — `backstage-*` recipes reference `manifests/kustomize/backstage/…` which doesn't exist;
  fix the path or drop the recipes.
- `scripts/nu/mod.nu` — `down --keep-cluster` deletes ns `zerg`, but the live app namespace is
  `zerg-dev`; reconcile the name so teardown actually targets the right namespace.
- `zerg-api` GKE env has `REDIS_URL` with an inline plaintext password
  (`redis://default:…@redis-redis-redis.dbs…`) in the Deployment spec; move it to the
  `zerg-api-secrets` ExternalSecret instead of an env literal.
- `.env.example` ships `JWT_SECRET=dev-secret-change-in-production` (< 32 chars; the app rejects it at
  startup — see `auth/config.rs`). Replace with an `openssl rand -base64 32` hint to avoid confusion.
- Add `mirrord` to the README prerequisites table and to `check-prerequisites`.

---

## Appendix — command & port cheat-sheet

**Environments**

```bash
# 1) Compose + bacon (inner loop)
just _docker-up && just db-fresh zerg && just dev

# 2) Kind + Tilt (k8s parity)
just local-up          # full lifecycle (kind + dbs + secrets + tilt)
just dev-kind          # if a cluster already exists
just local-status / just local-down / just local-restart

# 3) GKE + mirrord (high fidelity — to build)
just dev-gke api       # proposed: mirrord exec into zerg-dev/zerg-api
```

**Ports**

| Service | Local (1) | Kind (2) | In-cluster DNS (3 / GKE) |
|---------|-----------|----------|--------------------------|
| API (REST) | :8080 | :5221 | `zerg-api.zerg-dev:8080` |
| Tasks (gRPC) | :50051 | port-fwd | `zerg-tasks.zerg-dev:50051` |
| email-nats (health/metrics) | :8081 | — | `zerg-email-nats.zerg-dev:8081` |
| Web | :5173 | :5173 | `zerg.platform.yurikrupnik.com` |
| Postgres | :5432 | :5433 | `shared-postgres-rw.dbs:5432` |
| Redis | :6379 | :6379 | `redis-redis-redis.dbs:6379` |
| Qdrant | :6333/:6334 | — | `qdrant-qdrant.dbs:6333/6334` |
| NATS | :4222/:8222 | — | `nats.nats-system:4222` |
| Mailhog | :8025 | :8025 | `mailhog.zerg-dev:8025` |
| Flagsmith | (n/a) | (n/a) | `flagsmith-api.flagsmith:8000` |
| Directus (CMS) | (n/a) | (n/a) | `directus.cms:8055` |
| Tilt UI | — | :10350 | — |
| Kiali | — | :20001 | — |

> Related: `docs/engineering-review.md` (security/reliability findings),
> `docs/local-dev-cluster-plan.md`, `manifests/db/README.md`.
