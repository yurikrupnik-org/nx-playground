//! Provider-agnostic OIDC authentication for the terran services.
//!
//! Layers:
//! - [`verifier::OidcVerifier`] — verifies RS256 JWTs against a JWKS (the Bearer path).
//! - [`session`] — opaque server-side sessions (token-handler/BFF; the browser path).
//! - [`provider::IdentityProvider`] — the login/acquisition seam; [`provider::keycloak`]
//!   is the first adapter, WorkOS is a documented future drop-in.
//! - [`auth_required`] — mandatory middleware that resolves one [`AuthIdentity`] from
//!   either path.
//!
//! See `docs/auth-idp-decision.md` and `docs/terran-apps-plan.md`.

pub mod cookie;
pub mod error;
pub mod flow;
pub mod identity;
pub mod provider;
pub mod session;
pub mod verifier;

mod middleware;

pub use error::{AuthError, Result};
pub use flow::{LoginFlow, LoginFlowStore, StoredFlow};
pub use identity::AuthIdentity;
pub use middleware::{AuthLayerState, auth_required};
pub use provider::keycloak::KeycloakProvider;
pub use provider::workos::WorkosProvider;
pub use provider::{IdentityProvider, TokenSet, UserProfile};
pub use session::{RedisSessionStore, SessionRecord, SessionStore};
pub use verifier::{OidcVerifier, VerifierConfig};
