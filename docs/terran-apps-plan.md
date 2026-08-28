# Plan: `terran` apps — Rust API + SolidJS web on Keycloak auth

- **Status:** Planning
- **Date:** 2026-06-02
- **Related:** `docs/auth-idp-decision.md`, `docs/engineering-review.md`

## Goal

Stand up a greenfield app pair at `apps/terran/` — a Rust (Axum) **api** and a SolidJS **web** — that
use **Keycloak OIDC** for authentication from day one, with **mandatory auth, multi-tenant isolation,
and ownership checks enforced** (the exact gaps that made zerg's review findings #1/#2 critical). The
`zerg` apps are left untouched. Reuse existing workspace libraries where they are sound; introduce one
new auth crate that implements the provider-agnostic OIDC seam from the ADR.

## Product domain

terran is a **B2B multi-tenant enterprise observability + internal developer platform**. Each customer
company (tenant) sees only its own:

- **Cloud resources** — inventory and lifecycle, managed via **Crossplane**.
- **DB schemas** — catalog/observability across the org's databases.
- **Internal Developer Portal** — integrating **Backstage** (note: Backstage is the developer
  *portal*, **not** an identity provider; Keycloak remains the IdP).
- **FinOps** — cost suggestions and right-sizing recommendations.
- **Security alerts** — posture and findings.
- **AI agents** — mapping and usage/observability.

This is full-stack observability for enterprises. The domain will be planned incrementally; the
constant is that **tenant isolation is the primary security boundary** — cross-tenant data exposure is
the worst-case failure and drives the auth model below.

Non-goals (this plan): migrating zerg; adopting WorkOS. We scaffold auth + multi-tenancy + health +
one ownership-scoped sample resource, then grow the domain as we go.

## Conventions to mirror (from zerg)

| Concern | zerg convention | terran follows |
|---|---|---|
| Rust app | nx project (`@monodon/rust`) + Cargo workspace member; `project.json` with `nx:run-commands` calling `cargo` | same |
| Container | `manifests/dockers/rust.Dockerfile` w/ `APP_NAME` build-arg; web via `Dockerfile` + `DIST_PATH` | same |
| Web | Vite + `vite-plugin-solid` + Tailwind v4 + TanStack Solid Router/Query + Biome + Vitest | same |
| Frontend ↔ API | BFF: backend sets `HttpOnly` cookie; Vite dev proxy `/api` → api port | same |
| Shared types | `ts-rs` Rust→TS into a `types/` dir consumed via Vite alias | same |
| Migrations | Atlas under `manifests/db/<schema>` | new `manifests/db/terran` |
| Edition | Rust `edition = "2024"` | same |

## Port allocation (avoid collisions with zerg)

| Service | zerg | terran |
|---|---|---|
| API (HTTP) | 8080 | **8081** |
| Web (Vite dev) | 3000 | **3001** |
| Keycloak (shared, local) | — | **8088** → container 8080 |
| Flagsmith (feature flags) | — | **8000** |
| Directus (CMS) | — | **8055** |

`REDIRECT_BASE_URL` for terran api = `http://localhost:8081`; `FRONTEND_URL` = `http://localhost:3001`.

## Supporting services

Added to `manifests/dockers/compose.yaml` (started via `just docker-up`); all share the existing
Postgres (extra DBs created by `manifests/dockers/config/postgres-init/01-create-databases.sql`):

- **Keycloak** (`:8088`) — identity provider; realm imported from
  `manifests/dockers/config/keycloak/terran-realm.json` (client `terran-api`, roles, test user,
  Google/GitHub brokering via `${…}` env placeholders). Uses embedded H2 in `start-dev` (no DB).
- **Flagsmith** (`:8000`) — feature flags; DB `flagsmith`. The terran api reads flags server-side
  (and may expose a scoped subset to the SPA); evaluate behind the `org_id` for per-tenant rollout.
- **Directus** (`:8055`) — headless CMS; DB `directus`. Content (docs, announcements, marketing/help
  surfaces) consumed by the web app via the api (BFF) or directly for public content.

## Target folder structure

```
apps/terran/
  api/
    Cargo.toml                 # package "terran_api", edition 2024
    project.json               # nx: build/test/lint/run/container/scan (mirror zerg_api)
    Tiltfile
    src/
      main.rs                  # bootstrap: config, tracing, state, server (mirror zerg api/main.rs)
      config.rs                # OIDC_* + DB + server config from env
      state.rs                 # AppState { db, oidc, provider, ... }
      api/
        mod.rs                 # routers; MANDATORY auth on all business routers
        auth.rs                # /auth/login, /auth/callback, /auth/logout, /auth/me
        health.rs              # /health, /ready
        sample.rs              # one ownership-scoped protected resource (example)
    k8s/                       # kustomize base/overlays (mirror zerg api/k8s later)
  web/
    package.json               # solid-js, @tanstack/solid-router|query, tailwind v4, biome, vitest
    vite.config.ts             # port 3001, proxy /api -> http://localhost:8081
    project.json               # container/scan (mirror zerg-web)
    index.html  tsconfig.json
    src/
      index.tsx
      lib/auth-api.ts          # login()/logout()/me() against BFF
      lib/auth-context.tsx     # session state from /api/auth/me
      components/protected-route.tsx
      components/social-login.tsx   # Google/GitHub buttons via kc_idp_hint
      pages/...

libs/core/oidc-auth/           # NEW shared crate (the ADR seam)
  Cargo.toml                   # package "oidc_auth"
  src/
    lib.rs
    verifier.rs                # OidcVerifier: JWKS fetch+cache (by kid, rotation), RS256, iss/exp/aud
    identity.rs                # AuthIdentity { subject, org_id, roles, email, name, session_id }
    session.rs                 # SessionStore trait + Redis impl (opaque id -> tokens/identity)
    middleware.rs              # auth_required (MANDATORY): cookie->session OR Bearer->verify
    provider/
      mod.rs                   # IdentityProvider trait
      keycloak.rs              # standard OIDC adapter (authorize/exchange/refresh/logout)
      workos.rs                # (deferred) stub documenting the proprietary adapter

manifests/db/terran/           # Atlas schema + migrations for terran users + sample resource
manifests/dockers/config/keycloak/terran-realm.json   # realm import (client + Google/GitHub brokering)
```

## Multi-tenancy & security model

Tenant isolation is the core control. The model:

- **Identity is global, authorization is tenant-scoped.** A `user` (keyed by IdP `subject`) may belong
  to one or more **organizations** (customer companies). The active **`org_id` and coarse role
  (`org_admin` / `member` / `viewer`) are read from the verified IdP token on every request** — not
  from app-local state — because tenant membership is a security boundary and must not drift from the
  IdP. Model each company as a **Keycloak Organization** (KC 26) or group; emit `org_id` + role claims.
- **Fine-grained, resource-level permissions** (which projects/cloud resources a user may see, who can
  ack alerts or apply right-sizing) are application domain logic, stored app-local — but every such
  assignment and query is **always filtered by the token's `org_id`**.
- **Enforce isolation at the data layer.** Every tenant-owned row carries `org_id`; every query filters
  by the request's `org_id`. Use **Postgres Row-Level Security** as defense-in-depth so a missing
  handler-level filter cannot leak across tenants. Handler/UI checks alone are insufficient.
- **Sessions are server-side and revocable.** Browser auth uses the *token-handler / BFF* pattern: the
  IdP tokens live in Redis, the browser holds only an opaque session id in a `__Host-` cookie. This
  keeps tokens out of the browser and makes revocation instant (a security event deletes sessions).
  The session store **fails closed** — Redis unavailable means deny, never bypass.
- **Machine clients** (CI, Crossplane, AI agents) authenticate with `Authorization: Bearer` access
  tokens verified per-request via JWKS; prefer **Keycloak service accounts** (client-credentials) over
  long-lived user tokens.
- **CSRF done properly** (zerg's #8 was a no-op): `SameSite=Lax` cookies + a required custom header or
  double-submit token on state-changing requests.
- **Audit** auth events and security-sensitive actions — terran is itself a security product.

## Backend design

### New crate `libs/core/oidc-auth`

The provider-agnostic seam from the ADR. Dependencies already in the workspace:
`jsonwebtoken = 10.4` (RS256 + `DecodingKey::from_jwk`), `reqwest`, `serde`, `axum`, `tokio`,
`thiserror`, `tracing`.

- **`OidcVerifier`** (shared, hot path): fetches JWKS from a configured URL, caches keys by `kid`,
  refetches on unknown `kid` (handles rotation), verifies RS256, validates `iss` and `exp`, and
  validates `aud` **only when configured** (Keycloak sets `aud`; WorkOS access tokens do not). Maps
  raw claims → `AuthIdentity`. This is the one piece of load-bearing security code; it gets the most
  unit tests.
- **`IdentityProvider` trait** (login/acquisition, the only divergent layer):
  ```rust
  #[async_trait]
  pub trait IdentityProvider: Send + Sync {
      fn authorize_url(&self, state: &str, pkce: &PkceChallenge, idp_hint: Option<&str>) -> Url;
      async fn exchange_code(&self, code: &str, verifier: &str) -> Result<Session>;
      async fn refresh(&self, refresh_token: &str) -> Result<Session>;
      fn logout_url(&self, session_id: &str, post_logout: &str) -> Url;
      fn jwks_url(&self) -> &str;
      fn issuer(&self) -> &str;
  }
  ```
  - `KeycloakProvider`: standard OIDC — `authorize_url` supports `kc_idp_hint` (direct Google/GitHub);
    `exchange_code`/`refresh` hit the realm's `/token`; `logout_url` hits `/protocol/openid-connect/logout`.
  - `WorkosProvider`: deferred stub; documents the proprietary `POST /user_management/authenticate`
    mapping so the future adapter is a known quantity.
- **`SessionStore`** (trait + Redis impl): maps an opaque session id → `{ access/refresh tokens, sub,
  org_id, roles }`; supports delete-one and delete-all-for-user (revocation). Fails closed.
- **`auth_required` middleware** (MANDATORY): two ingress paths, both yielding one `AuthIdentity` (incl.
  `org_id`) into request extensions, else 401:
  - browser → `__Host-` session cookie → `SessionStore` lookup (lazy token refresh when near expiry);
  - machine → `Authorization: Bearer` → `OidcVerifier` (JWKS/RS256).
  There is no optional variant on business routes — that was zerg's #1.

### `apps/terran/api`

Skeleton mirrors `apps/zerg/api/src` (`main.rs`/`config.rs`/`state.rs`/`api/mod.rs`). Reuses:
`axum-helpers` (server assembly, rate-limit, security headers, CORS), `core_config`, `database`,
core tracing. New: `oidc_auth`.

- **Routing (`api/mod.rs`):** every business router is wrapped in `auth_required` (mandatory).
  Public routes: `/auth/*`, `/health`, `/ready`, and doc UIs only.
- **`/auth` endpoints (`api/auth.rs`), token-handler / BFF model:**
  - `GET /auth/login?idp=google|github|` → 302 to `provider.authorize_url(...)` (adds `kc_idp_hint`
    when `idp` is set); PKCE verifier + CSRF state held server-side (Redis), keyed by a short-lived
    pre-session cookie.
  - `GET /auth/callback` → `provider.exchange_code` → verify id/access token → resolve `org_id` →
    **JIT find-or-create** `organization` and `user`, upsert membership → create a server-side session
    (tokens stored in Redis) → set `__Host-tid=<opaque id>; HttpOnly; Secure; SameSite=Lax; Path=/` →
    302 to frontend. The browser never receives an IdP token.
  - `POST /auth/logout` → delete the server-side session, clear cookie → 302 to
    `provider.logout_url(sid, post_logout)` (ends the Keycloak session too).
  - `GET /auth/me` → read `AuthIdentity` from extensions → return profile + active org + role.
- **Data model (`manifests/db/terran`), tenant-scoped:**
  - `organizations (id UUID PK, external_org_id TEXT UNIQUE /* IdP org id */, name, created_at)`.
  - `users (id UUID PK, subject TEXT UNIQUE /* IdP sub */, email, name, created_at, updated_at,
    last_login_at)`. **No** `password_hash`, `google_id`, `github_id`, or lockout columns — the IdP
    owns credentials.
  - `memberships (user_id, org_id, role)` — app-local record of membership; the **authoritative**
    active org+role for a request still comes from the token (memberships back fine-grained data and
    admin UIs).
  - Every tenant-owned table carries `org_id UUID REFERENCES organizations(id)` (+ `user_id` where an
    owner is meaningful). **Postgres RLS** policies key off a per-transaction `app.org_id` GUC set from
    the token.
- **Isolation + ownership from day one:** handlers extract `AuthIdentity`, every query is scoped by
  `org_id` (and `user_id` for owned resources) via `_for_org` / `_for_user` methods — no IDOR (zerg's
  #2), no cross-tenant exposure.

## Frontend design (`apps/terran/web`, SolidJS)

Mirror zerg/web stack. The frontend is **IdP-agnostic** because the backend owns the session cookie:

- `lib/auth-api.ts`: `login(idp?)` → `window.location = '/api/auth/login' + (idp ? '?idp='+idp : '')`;
  `logout()` → `POST /api/auth/logout`; `me()` → `GET /api/auth/me`.
- `components/social-login.tsx`: Google/GitHub buttons that call `login('google')` / `login('github')`
  — these deep-link through Keycloak via `kc_idp_hint`, preserving zerg's current UX
  (`auth-api.ts:107`).
- `components/protected-route.tsx`: guard using a `solid-query` resource over `/api/auth/me`; redirect
  to login on 401.
- `vite.config.ts`: `server.port = 3001`, proxy `/api` → `http://localhost:8081` (same shape as
  zerg's `vite.config.ts:13-25`).

## Local dev (no cluster)

Reuse the Keycloak compose setup from the auth investigation:

- `manifests/dockers/compose.yaml`: `keycloak` service (`quay.io/keycloak/keycloak:26.4`, `start-dev
  --import-realm`, host `8088:8080`), env passthrough for `GOOGLE_*` / `GITHUB_*`.
- `manifests/dockers/config/keycloak/terran-realm.json`: realm `terran`, confidential client
  `terran-api` (standard flow + direct-access for curl), realm roles, a seeded test user, and
  `identityProviders` for Google/GitHub using `${GOOGLE_CLIENT_ID}` / `${GITHUB_CLIENT_ID}` etc.
  (Keycloak realm imports support `${ENV_VAR}` placeholders, so no secrets are committed.)
- External one-time step: register the broker redirect URIs in the Google Cloud Console and GitHub
  OAuth App: `http://localhost:8088/realms/terran/broker/{google,github}/endpoint`.
- `manifests/mprocs/local.yaml`: add `terran-api` (`bacon`) and `terran-web` (`bun nx dev terran-web`)
  procs.
- `justfile`: add a `keycloak-token` helper (direct-access grant) for smoke-testing the verifier.

Env (terran api): `OIDC_ISSUER=http://localhost:8088/realms/terran`,
`OIDC_JWKS_URL=.../protocol/openid-connect/certs`, `OIDC_CLIENT_ID=terran-api`,
`OIDC_CLIENT_SECRET=...`, `REDIRECT_BASE_URL=http://localhost:8081`,
`FRONTEND_URL=http://localhost:3001`.

## Workspace wiring

- `Cargo.toml`: add `apps/terran/api` and `libs/core/oidc-auth` to `[workspace] members`; add
  `oidc_auth = { path = 'libs/core/oidc-auth' }` to `[workspace.dependencies]`.
- nx: new `project.json` for `terran_api` and `terran-web` mirroring zerg (build/test/lint/run +
  container/scan with `terran-api` / `terran-web` image tags). `@monodon/rust` infers nothing extra;
  keep explicit `nx:run-commands` like zerg.
- CI: the existing matrix keys off nx projects; terran picks up build/test/lint/container/scan
  automatically once the `project.json`s exist.

## Phased build plan

1. **Foundation** — create `apps/terran/api` + `apps/terran/web` skeletons; add to Cargo workspace
   and nx; both build and serve a hello-world; ports wired (8081 / 3001). No auth yet.
2. **Auth core** — `libs/core/oidc-auth`: `OidcVerifier` + `SessionStore` (Redis, fail-closed) +
   `IdentityProvider` trait + `KeycloakProvider` + mandatory `auth_required` (cookie+Bearer paths).
   Unit tests: valid / expired / wrong-iss / wrong-alg / unknown-kid / JWKS rotation; session
   lookup/revocation; missing `org_id` → reject. WorkOS adapter left as a documented stub.
3. **API** — terran api wired: mandatory auth on business routers, `/auth/*` token-handler endpoints,
   JIT org+user provisioning, Atlas schema + RLS (`manifests/db/terran`), one tenant- and
   ownership-scoped sample resource. Tests: 401 without session/token, 200 with, **cross-tenant
   isolation** (org A cannot read org B), ownership isolation (no IDOR).
4. **Web** — Solid app: auth-api/auth-context, protected route, Google/GitHub buttons via
   `kc_idp_hint`, sample resource page. Component tests for the auth guard.
5. **Local dev** — Keycloak `terran-realm.json` (client + Google/GitHub brokering), compose/mprocs/
   justfile wiring; end-to-end smoke: `just keycloak-token` → call a protected endpoint; browser
   login round-trip.
6. **Verify** — `cargo test`/`clippy` for new crates, `vitest` for web, manual e2e login. Gates run
   once across the union of changed projects.

## Resolved decisions

- **Session storage → server-side opaque sessions (Redis), token-handler/BFF.** Browser holds only an
  opaque `__Host-` session id; IdP tokens stay server-side; revocation is instant; store fails closed.
  Not stateless encrypted cookies (can't revoke, puts tokens in the browser). Machine clients use
  Bearer + JWKS. *(Rationale: most secure for a security product; reuses Redis already in the stack.)*
- **Roles source → tenant + coarse role authoritative from the IdP token; fine-grained perms
  app-local but always `org_id`-filtered.** App-local roles are NOT the source of truth for
  tenant/coarse authz (avoids drift from the IdP). Model companies as Keycloak Organizations/groups.

## Open (plan as we go)

- First real domain slice after the sample resource (cloud-resource inventory via Crossplane is the
  likely candidate) — defines what Phase 3+ builds past the example.
- Whether to adopt Keycloak **Organizations** (KC 26) vs groups for tenant modeling — confirm during
  Phase 5 realm design.