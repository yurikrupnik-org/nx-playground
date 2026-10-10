//! Configuration for verifying caller tokens.
//!
//! The tasks service needs only enough IdP configuration to *verify* tokens - an
//! issuer and the client id its callers' tokens are minted for. It holds no client
//! secret and no API key: it never initiates a login, it only checks one.

use eyre::Result;

#[derive(Debug, Clone)]
pub struct TasksAuthConfig {
    /// WorkOS client id whose tokens this service accepts (`azp` check).
    pub workos_client_id: String,
    /// Expected `iss` claim; also the base for JWKS discovery.
    pub oidc_issuer: String,
}

impl TasksAuthConfig {
    pub fn from_env() -> Result<Self> {
        let workos_client_id = core_config::env_required("WORKOS_CLIENT_ID")?;
        // Mirrors `apps/zerg/api/src/config.rs`: WorkOS OIDC discovery lives at
        // `{api}/user_management/{client_id}`. Both services must agree on the issuer
        // or the BFF would mint tokens this service rejects.
        let oidc_issuer = std::env::var("WORKOS_ISSUER")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!("https://api.workos.com/user_management/{workos_client_id}")
            });
        Ok(Self {
            workos_client_id,
            oidc_issuer,
        })
    }
}
