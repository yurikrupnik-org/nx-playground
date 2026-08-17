//! Caller authentication for the tasks gRPC service.
//!
//! The service authenticates its own callers rather than trusting fields in the
//! request body. Every RPC carries an `authorization: Bearer <access_token>` metadata
//! header; we verify it (RS256 against the IdP's cached JWKS) and derive the tenant
//! scope from the verified claims.
//!
//! This is what makes the boundary real: reaching the port is no longer sufficient to
//! read another organization's tasks, because the caller cannot state which
//! organization it is acting for. See `docs/adr-tasks-service-boundary.md` Phase 4.

use std::sync::Arc;

use contract_tasks::TaskScope;
use oidc_auth::OidcVerifier;
use tonic::{Request, Status};

/// Verifies caller tokens and turns them into a [`TaskScope`].
#[derive(Clone)]
pub struct CallerAuth {
    mode: Mode,
}

#[derive(Clone)]
enum Mode {
    /// Production: verify RS256 against the IdP's JWKS.
    Jwks(Arc<OidcVerifier>),
    /// Tests only: stand in for a verified token with a fixed scope, so RPC-level
    /// behaviour can be tested without a live IdP. Never constructible in a release
    /// build - the boundary must not have a bypass that ships.
    #[cfg(test)]
    Fixed(TaskScope),
}

impl CallerAuth {
    pub fn new(verifier: Arc<OidcVerifier>) -> Self {
        Self {
            mode: Mode::Jwks(verifier),
        }
    }

    #[cfg(test)]
    pub fn fixed(scope: TaskScope) -> Self {
        Self {
            mode: Mode::Fixed(scope),
        }
    }

    /// Verify the request's bearer token and derive the tenant scope it may act within.
    ///
    /// Failures are deliberately terse on the wire (the detail is logged): a caller
    /// learns that it is unauthenticated, never why the token was rejected.
    pub async fn scope<T>(&self, request: &Request<T>) -> Result<TaskScope, Status> {
        let Mode::Jwks(verifier) = &self.mode;

        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| {
                tracing::warn!("rpc rejected: missing bearer token");
                Status::unauthenticated("missing bearer token")
            })?;

        let identity = verifier.verify(token).await.map_err(|e| {
            tracing::warn!(error = %e, "rpc rejected: token verification failed");
            Status::unauthenticated("invalid token")
        })?;

        Ok(TaskScope {
            org_ref: identity.tenant_ref(),
            user_ref: identity.subject,
        })
    }
}
