use std::collections::HashMap;
use std::sync::RwLock;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;

use crate::error::{AuthError, Result};
use crate::identity::AuthIdentity;

/// Configuration for [`OidcVerifier`].
#[derive(Clone, Debug)]
pub struct VerifierConfig {
    /// Expected `iss` claim, e.g. `http://localhost:8088/realms/terran`.
    pub issuer: String,
    /// JWKS endpoint, e.g. `{issuer}/protocol/openid-connect/certs`.
    pub jwks_url: String,
    /// If set, the token `aud` must contain this value. Keycloak access tokens
    /// generally do **not** target the client as audience (they carry `azp`), so
    /// leave this `None` unless a dedicated audience mapper is configured.
    pub audience: Option<String>,
    /// If non-empty, the token's `azp` (authorized party) MUST be one of these
    /// client ids. Keycloak access tokens always carry `azp` = the client the
    /// token was issued for, so this rejects tokens minted for *other* clients in
    /// the same realm even when `aud` is absent (the common Keycloak case).
    pub authorized_parties: Vec<String>,
    /// Reject tokens lacking an organization claim when `true`.
    pub require_org: bool,
    /// Name of the claim carrying the organization id (custom mapper / KC Organizations).
    pub org_claim: String,
    /// Optional claim holding a single flat role string (e.g. WorkOS `role`), used
    /// when the token has no Keycloak-style `realm_access.roles`.
    pub role_claim: Option<String>,
    /// Clock-skew leeway in seconds for `exp`/`nbf`/`iat`.
    pub leeway: u64,
}

impl VerifierConfig {
    /// Build a config from an issuer, deriving the standard Keycloak JWKS URL.
    pub fn keycloak(issuer: impl Into<String>) -> Self {
        let issuer = issuer.into();
        let jwks_url = format!("{issuer}/protocol/openid-connect/certs");
        Self {
            issuer,
            jwks_url,
            audience: None,
            authorized_parties: Vec::new(),
            require_org: false,
            org_claim: "org_id".to_string(),
            role_claim: None,
            leeway: 60,
        }
    }

    /// Build a config for WorkOS AuthKit access tokens: JWKS at
    /// `https://api.workos.com/sso/jwks/{client_id}`, flat `role` claim, `org_id`
    /// organization claim. WorkOS tokens carry no `aud`/`azp`, so those checks stay
    /// off. Per WorkOS OIDC discovery, `issuer` is
    /// `https://api.workos.com/user_management/{client_id}` (a custom auth domain
    /// changes it).
    pub fn workos(client_id: &str, issuer: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            jwks_url: format!("https://api.workos.com/sso/jwks/{client_id}"),
            audience: None,
            authorized_parties: Vec::new(), // WorkOS tokens carry no azp
            require_org: false,
            org_claim: "org_id".to_string(),
            role_claim: Some("role".to_string()),
            leeway: 60,
        }
    }
}

#[derive(Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct RawClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    realm_access: Option<RealmAccess>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// Verifies RS256 JWTs against a provider's JWKS and maps claims to [`AuthIdentity`].
///
/// This is the load-bearing security component on the Bearer path. It caches signing
/// keys by `kid` and refetches the JWKS once on an unknown `kid` (handling rotation).
/// It is provider-agnostic: point it at any OIDC issuer's JWKS.
pub struct OidcVerifier {
    config: VerifierConfig,
    http: reqwest::Client,
    keys: RwLock<HashMap<String, DecodingKey>>,
}

impl OidcVerifier {
    pub fn new(config: VerifierConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            keys: RwLock::new(HashMap::new()),
        }
    }

    /// Seed the key cache from a JWKS JSON document (used for warm-start and tests),
    /// bypassing the network.
    pub fn seed_jwks(&self, jwks_json: &str) -> Result<()> {
        let set: JwkSet =
            serde_json::from_str(jwks_json).map_err(|e| AuthError::Internal(e.to_string()))?;
        self.insert_jwks(set)
    }

    fn insert_jwks(&self, set: JwkSet) -> Result<()> {
        let mut guard = self.keys.write().expect("jwks lock poisoned");
        for jwk in &set.keys {
            let Some(kid) = jwk.common.key_id.clone() else {
                continue;
            };
            let key = DecodingKey::from_jwk(jwk).map_err(|e| AuthError::Internal(e.to_string()))?;
            guard.insert(kid, key);
        }
        Ok(())
    }

    fn has_key(&self, kid: &str) -> bool {
        self.keys
            .read()
            .expect("jwks lock poisoned")
            .contains_key(kid)
    }

    async fn refresh_jwks(&self) -> Result<()> {
        let resp = self
            .http
            .get(&self.config.jwks_url)
            .send()
            .await
            .map_err(|e| AuthError::Provider(format!("jwks fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AuthError::Provider(format!(
                "jwks endpoint returned {}",
                resp.status()
            )));
        }
        let set: JwkSet = resp
            .json()
            .await
            .map_err(|e| AuthError::Provider(format!("jwks parse failed: {e}")))?;
        self.insert_jwks(set)
    }

    fn validation(&self) -> Validation {
        let mut v = Validation::new(Algorithm::RS256);
        v.set_issuer(&[&self.config.issuer]);
        v.leeway = self.config.leeway;
        // `exp` is validated by default.
        match &self.config.audience {
            Some(aud) => v.set_audience(&[aud]),
            None => v.validate_aud = false,
        }
        v
    }

    fn decode_with_kid(&self, kid: &str, token: &str) -> Result<RawClaims> {
        let guard = self.keys.read().expect("jwks lock poisoned");
        let key = guard
            .get(kid)
            .ok_or_else(|| AuthError::UnknownKey(kid.to_string()))?;
        let data = decode::<RawClaims>(token, key, &self.validation())
            .map_err(|e| AuthError::InvalidToken(e.to_string()))?;
        Ok(data.claims)
    }

    fn map_claims(&self, c: RawClaims) -> Result<AuthIdentity> {
        if !self.config.authorized_parties.is_empty() {
            let azp = c.azp.as_deref();
            let trusted =
                azp.is_some_and(|p| self.config.authorized_parties.iter().any(|a| a == p));
            if !trusted {
                return Err(AuthError::InvalidToken(format!(
                    "untrusted authorized party (azp={azp:?})"
                )));
            }
        }
        let mut roles = c.realm_access.map(|r| r.roles).unwrap_or_default();
        if roles.is_empty()
            && let Some(rc) = &self.config.role_claim
            && let Some(r) = c.extra.get(rc).and_then(|v| v.as_str())
        {
            roles.push(r.to_string());
        }
        let org_id = self.extract_org(&c.extra);
        if self.config.require_org && org_id.is_none() {
            return Err(AuthError::OrgRequired);
        }
        Ok(AuthIdentity {
            subject: c.sub,
            org_id,
            roles,
            email: c.email,
            name: c.name.or(c.preferred_username),
            session_id: None,
        })
    }

    /// Extract the external organization id from the claims, supporting:
    /// 1. a flat configured claim (`org_claim`, e.g. `org_id`) holding a string;
    /// 2. Keycloak Organizations' `organization` claim — either `["alias", ..]`
    ///    (alias array, KC's default) or `{"alias": {"id": ".."}}` (with id), in
    ///    which case the id is preferred over the alias.
    fn extract_org(&self, extra: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
        if let Some(s) = extra.get(&self.config.org_claim).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
        match extra.get("organization") {
            Some(serde_json::Value::Array(arr)) => {
                arr.first().and_then(|v| v.as_str()).map(str::to_string)
            }
            Some(serde_json::Value::Object(map)) => map.iter().next().map(|(alias, v)| {
                v.get("id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| alias.clone())
            }),
            _ => None,
        }
    }

    /// Verify a bearer token and produce an [`AuthIdentity`].
    ///
    /// Enforces RS256, a known `kid`, `iss`, `exp` (with leeway), optional `aud`, and
    /// — when configured — the presence of an organization claim.
    pub async fn verify(&self, token: &str) -> Result<AuthIdentity> {
        let header = decode_header(token).map_err(|e| AuthError::InvalidToken(e.to_string()))?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::InvalidToken(format!(
                "unsupported alg {:?}; only RS256 is accepted",
                header.alg
            )));
        }
        let kid = header
            .kid
            .ok_or_else(|| AuthError::InvalidToken("missing kid".to_string()))?;

        // Refetch the JWKS once if we don't recognize the key (rotation).
        if !self.has_key(&kid) {
            self.refresh_jwks().await?;
        }
        let claims = self.decode_with_kid(&kid, token)?;
        self.map_claims(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Deterministic RSA-2048 test keypair (PKCS#8) + matching JWKS. Test-only.
    const PRIV_PEM: &str = include_str!("../testdata/test_key.pem");
    const JWKS_JSON: &str = include_str!("../testdata/test_jwks.json");
    const ISSUER: &str = "https://issuer.test/realms/terran";

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn mint(claims: serde_json::Value, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let key = EncodingKey::from_rsa_pem(PRIV_PEM.as_bytes()).expect("priv key");
        encode(&header, &claims, &key).expect("encode")
    }

    fn verifier(require_org: bool) -> OidcVerifier {
        let mut cfg = VerifierConfig::keycloak(ISSUER);
        cfg.require_org = require_org;
        let v = OidcVerifier::new(cfg);
        v.seed_jwks(JWKS_JSON).expect("seed jwks");
        v
    }

    #[tokio::test]
    async fn accepts_valid_token() {
        let v = verifier(false);
        let token = mint(
            json!({
                "sub": "user-1", "iss": ISSUER, "exp": now() + 300,
                "email": "a@b.c", "preferred_username": "alice",
                "realm_access": { "roles": ["org_admin", "member"] }
            }),
            "test-key-1",
        );
        let id = v.verify(&token).await.expect("valid token");
        assert_eq!(id.subject, "user-1");
        assert_eq!(id.email.as_deref(), Some("a@b.c"));
        assert_eq!(id.name.as_deref(), Some("alice"));
        assert!(id.has_role("org_admin"));
        assert!(!id.has_role("viewer"));
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let v = verifier(false);
        let token = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() - 3600 }),
            "test-key-1",
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_wrong_issuer() {
        let v = verifier(false);
        let token = mint(
            json!({ "sub": "u", "iss": "https://evil.example", "exp": now() + 300 }),
            "test-key-1",
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_non_rs256_alg() {
        // HS256 token with the same kid must be refused on algorithm grounds.
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("test-key-1".to_string());
        let token = encode(
            &header,
            &json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300 }),
            &EncodingKey::from_secret(b"shhh"),
        )
        .unwrap();
        let v = verifier(false);
        let err = v.verify(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::InvalidToken(ref m) if m.contains("RS256")),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_unknown_kid_when_jwks_has_no_match() {
        let v = verifier(false);
        // No network in tests, so an unknown kid cannot be resolved by refetch.
        let token = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300 }),
            "rotated-key-99",
        );
        let err = v.verify(&token).await.unwrap_err();
        // refresh_jwks fails (no server) -> Provider; otherwise UnknownKey. Either is a denial.
        assert!(
            matches!(err, AuthError::Provider(_) | AuthError::UnknownKey(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn requires_org_when_configured() {
        let v = verifier(true);
        let token = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300 }),
            "test-key-1",
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::OrgRequired), "got {err:?}");

        let with_org = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300, "org_id": "org-42" }),
            "test-key-1",
        );
        let id = v.verify(&with_org).await.expect("token with org");
        assert_eq!(id.org_id.as_deref(), Some("org-42"));
    }

    #[tokio::test]
    async fn extracts_org_from_keycloak_organization_array() {
        // Keycloak 26's default org mapper emits an alias array.
        let v = verifier(false);
        let token = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300, "organization": ["demo-corp"] }),
            "test-key-1",
        );
        let id = v.verify(&token).await.expect("valid");
        assert_eq!(id.org_id.as_deref(), Some("demo-corp"));
    }

    #[tokio::test]
    async fn extracts_org_id_from_organization_object() {
        // When the mapper is configured to include the id, prefer it over the alias.
        let v = verifier(false);
        let token = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300,
                    "organization": { "acme": { "id": "org-123" } } }),
            "test-key-1",
        );
        let id = v.verify(&token).await.expect("valid");
        assert_eq!(id.org_id.as_deref(), Some("org-123"));
    }

    #[tokio::test]
    async fn rejects_untrusted_authorized_party() {
        let mut cfg = VerifierConfig::keycloak(ISSUER);
        cfg.authorized_parties = vec!["terran-api".to_string()];
        let v = OidcVerifier::new(cfg);
        v.seed_jwks(JWKS_JSON).expect("seed jwks");

        // Token minted for another client in the same realm must be refused.
        let other = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300, "azp": "some-other-client" }),
            "test-key-1",
        );
        let err = v.verify(&other).await.unwrap_err();
        assert!(
            matches!(err, AuthError::InvalidToken(ref m) if m.contains("azp")),
            "got {err:?}"
        );

        // Missing azp is also untrusted when an allow-list is configured.
        let no_azp = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300 }),
            "test-key-1",
        );
        assert!(
            v.verify(&no_azp).await.is_err(),
            "missing azp must be rejected"
        );

        // The expected client's token passes.
        let ours = mint(
            json!({ "sub": "u", "iss": ISSUER, "exp": now() + 300, "azp": "terran-api" }),
            "test-key-1",
        );
        assert!(v.verify(&ours).await.is_ok(), "trusted azp must pass");
    }
}
