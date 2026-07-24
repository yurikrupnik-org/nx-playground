# ADR: Identity Provider Strategy — Keycloak as primary, OIDC seam keeps WorkOS optional

- **Status:** Accepted
- **Date:** 2026-06-02
- **Deciders:** yurikrupnik
- **Related:** `docs/engineering-review.md` (findings #1, #2, #8, #12), `docs/terran-apps-plan.md`

## Context

The engineering review found the homegrown auth stack has the right crypto but the wrong
enforcement and an unbuilt operational tail:

- **#1 (CRITICAL):** mandatory `jwt_auth_middleware` (`libs/core/axum-helpers/src/auth/middleware.rs:56`)
  is never wired; only `optional_jwt_auth_middleware` is applied globally
  (`apps/zerg/api/src/api/mod.rs:101-104`) → every business endpoint is unauthenticated.
- **#2 (HIGH):** handlers ignore caller identity even when authenticated (IDOR) — the `_for_user`
  service methods exist but are not called (`libs/domains/projects/src/handlers.rs:150,175,199`).
- The custom subsystem (`libs/domains/users/src/oauth/`, `libs/core/axum-helpers/src/auth/`) carries
  a long unbuilt tail: **no `/refresh` route exists** (refresh tokens are minted and whitelisted but
  never consumable), email verification is a TODO (`auth_handlers.rs:111`), no password reset, no
  MFA, CSRF is a no-op stub (#8), and the rate limiter fails open on `/auth` (#12).

We evaluated three directions: keep hand-rolling identity, adopt **WorkOS** (hosted), or adopt
**Keycloak** (self-hostable). A hard constraint dominated the decision: **closed / air-gapped
deployment clients require a self-hostable IdP.** WorkOS cannot run on-prem.

Verified facts informing the decision:

- WorkOS has **no official Rust SDK** (backends: Node, Go, Ruby, Python, PHP, Laravel, Java/Kotlin,
  .NET). AuthKit issues RS256 JWTs verified via JWKS; its login exchange is proprietary
  (`POST /user_management/authenticate`, custom grant types), i.e. **not standard OIDC**.
- Keycloak is a standard OIDC provider (discovery doc, standard token/refresh, RS256 + JWKS),
  self-hostable, with built-in **identity brokering** for Google/GitHub and a native Organizations
  feature for B2B multi-tenancy (v26+).
- WorkOS's genuine differentiator over Keycloak is **not capability** — it is managed/zero-ops and
  the **Admin Portal** (hosted self-serve enterprise-SSO onboarding for customer IT admins), plus
  Directory Sync (SCIM) as a managed product. Pricing is not the deciding factor at our scale
  (AuthKit is free to 1M MAUs incl. SSO).

## Decision

1. **Stop hand-rolling identity.** Adopt the OIDC-provider model: the backend **verifies** an
   external provider's RS256 JWT via JWKS and **JIT-provisions** a local user keyed by the token
   `sub`. The app never mints identity tokens.

2. **Keycloak is the primary IdP for all deployments** (SaaS and closed/on-prem). Rationale: closed
   clients require a self-hostable IdP regardless, so running a second hosted IdP for SaaS would
   mean operating two identity stacks for no capability gain. One stack, one onboarding story, one
   set of quirks.

3. **Social login (Google/GitHub) moves into Keycloak** via identity brokering, reusing the existing
   `GOOGLE_*` / `GITHUB_*` credentials. The app-side OAuth subsystem (`libs/domains/users/src/oauth/`:
   providers, PKCE/CSRF state manager, account linking) is **retired in the new stack** — Keycloak
   owns token exchange, userinfo, and account linking (First Login Flow).

4. **WorkOS stays optional, behind the same OIDC seam.** The auth layer is built around an
   `IdentityProvider` trait modeled on standard OIDC; Keycloak is the first adapter. WorkOS becomes a
   drop-in adapter if/when the revisit trigger fires. Designing on OIDC also makes any other provider
   (ZITADEL, Authentik, Auth0, Entra) a cheap addition.

5. **Enforcement is independent of the IdP choice.** Mandatory auth on all non-public routers and
   ownership checks (`_for_user`) are required regardless — an IdP swap does not fix #1/#2. The new
   stack enforces both from day one.

6. **Scope:** this strategy is implemented greenfield in the new `apps/terran/{api,web}` stack
   (see `docs/terran-apps-plan.md`). **The `zerg` apps are left as-is**; their auth gaps remain
   tracked in the engineering review and can be remediated or migrated later as a separate effort.

## Architecture seam (summary)

```
                 ┌──────────────────────────────────────────────┐
   request ───►  │  jwt_auth_middleware (MANDATORY)              │
                 │    └─ OidcVerifier: JWKS cache + RS256 verify  │  ← SHARED across providers
                 │         (iss/exp/aud per provider config)      │
                 │    └─ claims → AuthIdentity (sub, email, roles)│
                 │    └─ JIT find-or-create local user by `sub`   │
                 └──────────────────────────────────────────────┘
   login/exchange/refresh/logout ─► IdentityProvider trait
        ├─ KeycloakProvider   (standard OIDC: authorize / token / refresh / end_session)
        └─ WorkosProvider     (optional later: proprietary REST adapter)
```

- **Verification / enforcement** is provider-agnostic (both emit RS256 JWTs verified via JWKS).
- **Login / token acquisition** is the only divergent layer: a trait with one impl per provider.
- **Frontend stays IdP-agnostic** via the BFF/backend-session model (backend holds the client
  secret, performs the code exchange, sets an `HttpOnly` cookie). No `authkit-react` coupling.

## Consequences

**We gain**
- One self-hostable identity stack that works for SaaS and air-gapped clients.
- Deletion of the homegrown OAuth/JWT/session subsystem in the new stack; email verification,
  password reset, MFA, refresh, and social brokering come from Keycloak instead of being built.
- A provider-agnostic seam: WorkOS (or any OIDC IdP) is a later adapter, not a rewrite.

**We own**
- **Keycloak operations:** HA, Postgres backing store, backups, and disciplined version upgrades
  (Keycloak upgrades are non-trivial).
- **Enterprise-SSO onboarding UX:** with Keycloak we configure each customer's SSO connection
  ourselves, or build a thin self-serve UI on Keycloak's admin REST API / adopt KC Organizations.
  This is precisely the gap the WorkOS Admin Portal fills.
- **Compliance posture** for the identity system (audit, SLA) sits with us.

**Revisit trigger (when to add WorkOS)**

> Add WorkOS — as an `IdentityProvider` adapter behind the existing seam — when **per-customer
> enterprise-SSO onboarding becomes a sales/engineering bottleneck**: i.e. when enough enterprise
> deals stall on "your customer's IT admin must self-configure SSO/SCIM this week" that paying for
> the Admin Portal beats configuring connections by hand or building the portal ourselves.
> Until then, Keycloak-only is the default.

## Alternatives considered

- **Keep hand-rolling identity** — rejected: unbuilt tail (reset/verify/MFA/refresh) is recurring
  CVE-prone work, and #1/#2 show enforcement discipline is the real risk, not crypto.
- **WorkOS as primary** — rejected now: cannot serve air-gapped clients, so it can only ever be the
  SaaS half; running it alongside the mandatory Keycloak doubles the stack. Its main advantage
  (managed ops) is partly forfeited the moment we self-host for closed clients anyway.
- **WorkOS + Keycloak split (SaaS vs on-prem)** — deferred: viable thanks to the OIDC seam, but not
  worth two stacks until the onboarding trigger fires.
