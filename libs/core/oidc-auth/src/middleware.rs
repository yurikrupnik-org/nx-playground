use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::{AuthError, Result};
use crate::identity::AuthIdentity;
use crate::provider::IdentityProvider;
use crate::session::{SessionRecord, SessionStore};
use crate::verifier::OidcVerifier;

/// Default seconds-before-expiry at which the session path refreshes the access token.
const DEFAULT_REFRESH_LEEWAY: u64 = 30;

/// Shared state for [`auth_required`], built once and passed via
/// `axum::middleware::from_fn_with_state`.
#[derive(Clone)]
pub struct AuthLayerState {
    pub verifier: Arc<OidcVerifier>,
    pub sessions: Arc<dyn SessionStore>,
    /// Login/acquisition provider, used to refresh access tokens on the session path.
    pub provider: Arc<dyn IdentityProvider>,
    /// Session cookie name (e.g. `__Host-tid`).
    pub cookie_name: Arc<str>,
    /// Refresh the stored access token when within this many seconds of its expiry.
    pub refresh_leeway: u64,
}

impl AuthLayerState {
    pub fn new(
        verifier: Arc<OidcVerifier>,
        sessions: Arc<dyn SessionStore>,
        provider: Arc<dyn IdentityProvider>,
        cookie_name: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            verifier,
            sessions,
            provider,
            cookie_name: cookie_name.into(),
            refresh_leeway: DEFAULT_REFRESH_LEEWAY,
        }
    }
}

/// Mandatory authentication middleware.
///
/// Resolves exactly one [`AuthIdentity`] from two ingress paths and inserts it into
/// request extensions, or rejects with 401/503:
/// - **machine** → `Authorization: Bearer <jwt>` verified via JWKS/RS256;
/// - **browser** → opaque session cookie resolved against the [`SessionStore`].
///
/// There is no optional variant — guarded routes are always authenticated.
pub async fn auth_required(
    State(state): State<AuthLayerState>,
    mut req: Request,
    next: Next,
) -> Result<Response> {
    // Pull credentials out synchronously so no borrow of `&Request` is held across an
    // await: `axum::body::Body` is not `Sync`, so a live `&Request` across `.await`
    // would make this future `!Send` and the middleware unusable as a tower `Service`.
    let creds = extract_credentials(&req, &state.cookie_name);
    let identity = resolve(&state, creds).await?;
    req.extensions_mut().insert(identity);
    Ok(next.run(req).await)
}

/// Credentials lifted out of the request as owned values.
enum Credentials {
    Bearer(String),
    Session(String),
    None,
}

fn extract_credentials(req: &Request, cookie_name: &str) -> Credentials {
    if let Some(token) = bearer_token(req) {
        return Credentials::Bearer(token.to_owned());
    }
    if let Some(sid) = crate::cookie::parse(req.headers(), cookie_name) {
        return Credentials::Session(sid.to_owned());
    }
    Credentials::None
}

async fn resolve(state: &AuthLayerState, creds: Credentials) -> Result<AuthIdentity> {
    match creds {
        // Bearer token (machine clients) — verified statelessly via JWKS.
        Credentials::Bearer(token) => state.verifier.verify(&token).await,
        // Opaque session cookie (browser) — resolved server-side; store errors fail closed.
        Credentials::Session(sid) => resolve_session(state, sid).await,
        Credentials::None => Err(AuthError::MissingCredentials),
    }
}

/// Resolve the browser/session path: load the record, enforce the absolute session
/// cap, and lazily refresh the access token near expiry so IdP-side changes (roles,
/// org, revocation) propagate within the access-token lifetime instead of the full
/// session TTL. Any failure denies (fail closed) and drops the session.
async fn resolve_session(state: &AuthLayerState, sid: String) -> Result<AuthIdentity> {
    let mut rec = state
        .sessions
        .get(&sid)
        .await?
        .ok_or(AuthError::SessionInvalid)?;
    let now = now_secs();

    // Absolute cap: a refreshed session still cannot outlive its original lifetime.
    if now >= rec.session_expires_at {
        let _ = state.sessions.delete(&sid).await;
        return Err(AuthError::SessionInvalid);
    }

    // Lazy refresh near (or past) access-token expiry.
    if now + state.refresh_leeway >= rec.access_expires_at
        && refresh_session(state, &sid, &mut rec, now).await.is_err()
    {
        // Refresh token expired/revoked, or the user was disabled at the IdP.
        let _ = state.sessions.delete(&sid).await;
        return Err(AuthError::SessionInvalid);
    }

    Ok(AuthIdentity {
        subject: rec.subject,
        org_id: rec.org_id,
        roles: rec.roles,
        email: rec.email,
        name: rec.name,
        session_id: Some(sid),
    })
}

/// Exchange the stored refresh token for a fresh token set, re-verify it (so the
/// identity reflects current IdP claims), and persist the updated record in place.
async fn refresh_session(
    state: &AuthLayerState,
    sid: &str,
    rec: &mut SessionRecord,
    now: u64,
) -> Result<()> {
    let refresh_token = rec.refresh_token.clone().ok_or(AuthError::SessionInvalid)?;
    let tokens = state.provider.refresh(&refresh_token).await?;
    let identity = state.verifier.verify(&tokens.access_token).await?;

    rec.access_token = tokens.access_token;
    if tokens.refresh_token.is_some() {
        rec.refresh_token = tokens.refresh_token;
    }
    if tokens.id_token.is_some() {
        rec.id_token = tokens.id_token;
    }
    rec.org_id = identity.org_id;
    rec.roles = identity.roles;
    rec.email = identity.email;
    rec.name = identity.name;
    rec.access_expires_at = now + tokens.expires_in.unwrap_or(300);

    let ttl = rec.session_expires_at.saturating_sub(now).max(1);
    state.sessions.update(sid, rec, ttl).await
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Extract the bearer token from the `Authorization` header, if present.
fn bearer_token(req: &Request) -> Option<&str> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::TokenSet;
    use crate::verifier::VerifierConfig;
    use async_trait::async_trait;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PRIV_PEM: &str = include_str!("../testdata/test_key.pem");
    const JWKS_JSON: &str = include_str!("../testdata/test_jwks.json");
    const ISSUER: &str = "https://issuer.test/realms/terran";

    fn mint(claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        let key = EncodingKey::from_rsa_pem(PRIV_PEM.as_bytes()).unwrap();
        encode(&header, &claims, &key).unwrap()
    }

    #[derive(Default)]
    struct FakeStore {
        map: Mutex<HashMap<String, SessionRecord>>,
        deleted: Mutex<Vec<String>>,
        updated: AtomicUsize,
    }

    #[async_trait]
    impl SessionStore for FakeStore {
        async fn create(&self, record: &SessionRecord, _ttl: u64) -> Result<String> {
            self.map
                .lock()
                .unwrap()
                .insert("sid-1".into(), record.clone());
            Ok("sid-1".into())
        }
        async fn update(&self, id: &str, record: &SessionRecord, _ttl: u64) -> Result<()> {
            self.updated.fetch_add(1, Ordering::SeqCst);
            self.map.lock().unwrap().insert(id.into(), record.clone());
            Ok(())
        }
        async fn get(&self, id: &str) -> Result<Option<SessionRecord>> {
            Ok(self.map.lock().unwrap().get(id).cloned())
        }
        async fn delete(&self, id: &str) -> Result<()> {
            self.deleted.lock().unwrap().push(id.into());
            self.map.lock().unwrap().remove(id);
            Ok(())
        }
        async fn delete_all_for_user(&self, _subject: &str) -> Result<()> {
            Ok(())
        }
    }

    /// Provider whose `refresh` mints a fresh RS256 token with `next_roles`, or errors
    /// when `next_roles` is `None` (simulating a revoked/expired refresh token).
    struct FakeProvider {
        refreshed: AtomicUsize,
        next_roles: Option<Vec<String>>,
    }

    #[async_trait]
    impl IdentityProvider for FakeProvider {
        fn authorize_url(&self, _s: &str, _c: &str, _h: Option<&str>) -> String {
            String::new()
        }
        async fn exchange_code(&self, _c: &str, _v: &str) -> Result<TokenSet> {
            unimplemented!()
        }
        async fn exchange_password(&self, _u: &str, _p: &str) -> Result<TokenSet> {
            unimplemented!()
        }
        async fn refresh(&self, _rt: &str) -> Result<TokenSet> {
            self.refreshed.fetch_add(1, Ordering::SeqCst);
            match &self.next_roles {
                Some(roles) => Ok(TokenSet {
                    access_token: mint(json!({
                        "sub": "user-1", "iss": ISSUER, "exp": now_secs() + 300,
                        "realm_access": { "roles": roles },
                    })),
                    refresh_token: Some("rt2".into()),
                    id_token: None,
                    expires_in: Some(300),
                }),
                None => Err(AuthError::Provider("refresh denied".into())),
            }
        }
        fn logout_url(&self, _h: Option<&str>, _p: &str) -> String {
            String::new()
        }
        fn issuer(&self) -> &str {
            ISSUER
        }
        fn jwks_url(&self) -> &str {
            ""
        }
    }

    fn layer(store: Arc<FakeStore>, provider: Arc<FakeProvider>) -> AuthLayerState {
        let verifier = OidcVerifier::new(VerifierConfig::keycloak(ISSUER));
        verifier.seed_jwks(JWKS_JSON).unwrap();
        AuthLayerState {
            verifier: Arc::new(verifier),
            sessions: store,
            provider,
            cookie_name: "sid".into(),
            refresh_leeway: 30,
        }
    }

    fn record(access_expires_at: u64, session_expires_at: u64, roles: &[&str]) -> SessionRecord {
        SessionRecord {
            subject: "user-1".into(),
            org_id: Some("org-1".into()),
            roles: roles.iter().map(|s| s.to_string()).collect(),
            email: None,
            name: None,
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            id_token: None,
            access_expires_at,
            session_expires_at,
        }
    }

    #[tokio::test]
    async fn valid_session_is_not_refreshed() {
        let store = Arc::new(FakeStore::default());
        let provider = Arc::new(FakeProvider {
            refreshed: AtomicUsize::new(0),
            next_roles: None,
        });
        let st = layer(store.clone(), provider.clone());
        let now = now_secs();
        store.map.lock().unwrap().insert(
            "sid-1".into(),
            record(now + 1000, now + 2000, &["org_admin"]),
        );

        let id = resolve_session(&st, "sid-1".into())
            .await
            .expect("valid session");
        assert_eq!(id.subject, "user-1");
        assert!(id.has_role("org_admin"));
        assert_eq!(
            provider.refreshed.load(Ordering::SeqCst),
            0,
            "no refresh when fresh"
        );
    }

    #[tokio::test]
    async fn near_expiry_refreshes_and_picks_up_new_claims() {
        let store = Arc::new(FakeStore::default());
        // IdP now reports the user as a viewer (downgraded from org_admin).
        let provider = Arc::new(FakeProvider {
            refreshed: AtomicUsize::new(0),
            next_roles: Some(vec!["viewer".into()]),
        });
        let st = layer(store.clone(), provider.clone());
        let now = now_secs();
        store.map.lock().unwrap().insert(
            "sid-1".into(),
            record(now.saturating_sub(5), now + 2000, &["org_admin"]),
        );

        let id = resolve_session(&st, "sid-1".into())
            .await
            .expect("refreshed session");
        assert_eq!(
            provider.refreshed.load(Ordering::SeqCst),
            1,
            "refresh attempted"
        );
        assert_eq!(store.updated.load(Ordering::SeqCst), 1, "record persisted");
        assert!(id.has_role("viewer"), "roles reflect the refreshed token");
        assert!(!id.has_role("org_admin"), "stale role dropped");
    }

    #[tokio::test]
    async fn hard_session_expiry_denies_without_refresh() {
        let store = Arc::new(FakeStore::default());
        let provider = Arc::new(FakeProvider {
            refreshed: AtomicUsize::new(0),
            next_roles: Some(vec!["member".into()]),
        });
        let st = layer(store.clone(), provider.clone());
        let now = now_secs();
        store.map.lock().unwrap().insert(
            "sid-1".into(),
            record(now + 1000, now.saturating_sub(1), &["member"]),
        );

        let err = resolve_session(&st, "sid-1".into()).await.unwrap_err();
        assert!(matches!(err, AuthError::SessionInvalid), "got {err:?}");
        assert!(
            store.deleted.lock().unwrap().contains(&"sid-1".to_string()),
            "session dropped"
        );
        assert_eq!(
            provider.refreshed.load(Ordering::SeqCst),
            0,
            "no refresh past hard cap"
        );
    }

    #[tokio::test]
    async fn failed_refresh_denies_and_revokes() {
        let store = Arc::new(FakeStore::default());
        // Refresh token revoked / user disabled at the IdP.
        let provider = Arc::new(FakeProvider {
            refreshed: AtomicUsize::new(0),
            next_roles: None,
        });
        let st = layer(store.clone(), provider.clone());
        let now = now_secs();
        store.map.lock().unwrap().insert(
            "sid-1".into(),
            record(now.saturating_sub(5), now + 2000, &["member"]),
        );

        let err = resolve_session(&st, "sid-1".into()).await.unwrap_err();
        assert!(matches!(err, AuthError::SessionInvalid), "got {err:?}");
        assert!(
            store.deleted.lock().unwrap().contains(&"sid-1".to_string()),
            "session revoked on failed refresh"
        );
    }
}
