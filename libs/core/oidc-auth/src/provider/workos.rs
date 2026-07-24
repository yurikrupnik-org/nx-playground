//! WorkOS AuthKit adapter — **deferred** (see `docs/auth-idp-decision.md`).
//!
//! Intentionally not implemented. WorkOS is kept as a future drop-in behind the
//! [`crate::provider::IdentityProvider`] seam; it is *not* wired today, and no
//! placeholder `IdentityProvider` impl is provided so it cannot be used by accident.
//!
//! When implemented, an adapter must translate WorkOS's **proprietary** flow into the
//! trait — it is OAuth2-flavored but not standard OIDC:
//!
//! - **Authorize:** `https://api.workos.com/user_management/authorize` (hosted AuthKit;
//!   `provider=authkit` or `GoogleOAuth`/`GitHubOAuth` for direct social).
//! - **Exchange/refresh:** `POST https://api.workos.com/user_management/authenticate`
//!   with `client_secret` in the body and custom `grant_type`s
//!   (e.g. `urn:workos:oauth:grant-type:...`), returning a proprietary JSON shape
//!   (`user` object + `access_token` + `refresh_token`, not a standard token response).
//! - **Verify:** RS256 access tokens via JWKS at `.../sso/jwks/<client_id>` — this part
//!   reuses [`crate::verifier::OidcVerifier`] unchanged (claims: `sub`, `sid`, `org_id`,
//!   `role`, `permissions`; note WorkOS tokens carry no standard `aud`).
//! - **Logout:** `https://api.workos.com/user_management/sessions/logout`.
//!
//! There is no official Rust SDK, so the adapter would be hand-rolled `reqwest` calls.
