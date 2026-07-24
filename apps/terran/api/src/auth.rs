//! Token-handler / BFF auth endpoints. The browser only ever holds an opaque
//! session id; the IdP tokens are exchanged here and stored server-side.
//!
//! The security primitives — PKCE/CSRF generation, the single-use login-flow store,
//! and cookie attributes — live in the `oidc-auth` crate ([`oidc_auth::flow`],
//! [`oidc_auth::cookie`]); these handlers are the glue that wires them to HTTP.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum_helpers::http::csrf;
use oidc_auth::{IdentityProvider, LoginFlow, SessionRecord, SessionStore, TokenSet, cookie};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::provisioning::{provision_tenant, resolve_tenant};
use crate::state::AppState;

/// Short-lived cookie carrying the opaque login-flow id between authorize and callback.
const FLOW_COOKIE: &str = "terran_flow";
/// Readable double-submit CSRF cookie issued alongside the session.
pub const CSRF_COOKIE: &str = "terran_csrf";
/// Lifetime of an in-flight login (authorize → callback window).
pub const FLOW_TTL_SECS: u64 = 600;

#[derive(Deserialize)]
pub struct LoginQuery {
    /// Optional brokered provider hint (`google`/`github`) → Keycloak `kc_idp_hint`.
    #[serde(default)]
    idp: Option<String>,
}

/// `GET /api/auth/login` → redirect to the IdP authorize endpoint (PKCE + CSRF state).
#[utoipa::path(
    get,
    path = "/auth/login",
    tag = "auth",
    params(("idp" = Option<String>, Query, description = "Brokered IdP hint (e.g. google, github) → Keycloak kc_idp_hint")),
    responses(
        (status = 303, description = "Redirect to the IdP authorize endpoint (sets the login-flow cookie)")
    )
)]
pub async fn login(State(st): State<AppState>, Query(q): Query<LoginQuery>) -> ApiResult<Response> {
    let flow = LoginFlow::new();
    let flow_id = st.flows.begin(&flow).await?;
    let url = st
        .provider
        .authorize_url(&flow.state, &flow.code_challenge, q.idp.as_deref());
    let cookie = cookie::build(
        FLOW_COOKIE,
        &flow_id,
        FLOW_TTL_SECS,
        st.config.cookie_secure,
    );
    Ok(redirect_with_cookies(&url, [cookie]))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /api/auth/callback` → exchange code, provision tenant, mint a server-side session.
#[utoipa::path(
    get,
    path = "/auth/callback",
    tag = "auth",
    params(
        ("code" = Option<String>, Query, description = "Authorization code from the IdP"),
        ("state" = Option<String>, Query, description = "CSRF state echoed by the IdP"),
        ("error" = Option<String>, Query, description = "Error code if the IdP rejected the request")
    ),
    responses(
        (status = 303, description = "Session established; redirect to the frontend (sets session + CSRF cookies)"),
        (status = 400, description = "Missing/expired flow, missing code/state, or state mismatch")
    )
)]
pub async fn callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    if let Some(err) = q.error {
        tracing::warn!(error = %err, "idp returned an error on callback");
        return Ok(
            Redirect::to(&format!("{}?auth_error=1", st.config.frontend_url)).into_response(),
        );
    }
    let code = q
        .code
        .ok_or_else(|| ApiError::bad_request("missing code"))?;
    let state_param = q
        .state
        .ok_or_else(|| ApiError::bad_request("missing state"))?;
    let flow_id = cookie::parse(&headers, FLOW_COOKIE)
        .ok_or_else(|| ApiError::bad_request("missing flow"))?;

    // Single-use, atomic consume (replay-safe).
    let flow = st
        .flows
        .consume(flow_id)
        .await?
        .ok_or_else(|| ApiError::bad_request("login flow expired"))?;

    // CSRF: the returned state must match the one we issued.
    if flow.state != state_param {
        return Err(ApiError::bad_request("state mismatch"));
    }

    let tokens = st
        .provider
        .exchange_code(&code, &flow.code_verifier)
        .await?;
    // Mint the server-side session + cookies (shared with the password-login path).
    let mut cookies = establish_session(&st, tokens).await?;
    cookies.push(cookie::clear(FLOW_COOKIE, st.config.cookie_secure));
    Ok(redirect_with_cookies(&st.config.frontend_url, cookies))
}

/// Native email+password login form payload.
#[derive(Deserialize, ToSchema)]
pub struct PasswordLogin {
    pub email: String,
    pub password: String,
}

/// `POST /api/auth/login/password` → native email+password sign-in via Keycloak's
/// Direct Access Grant. Establishes the same server-side session as the redirect flow
/// and returns `204` with the session + CSRF cookies; the SPA then loads `/me`.
///
/// Public and CSRF-exempt: it runs before any session/CSRF cookie exists. The `Json`
/// body forces `content-type: application/json`, so a cross-site `<form>` POST is
/// rejected and a cross-origin `fetch` is blocked by the CORS preflight — login CSRF
/// cannot mint a session in the victim's browser.
#[utoipa::path(
    post,
    path = "/auth/login/password",
    tag = "auth",
    request_body = PasswordLogin,
    responses(
        (status = 204, description = "Session established (sets session + CSRF cookies)"),
        (status = 400, description = "Missing email or password"),
        (status = 401, description = "Invalid credentials")
    )
)]
pub async fn password_login(
    State(st): State<AppState>,
    Json(body): Json<PasswordLogin>,
) -> ApiResult<Response> {
    let email = body.email.trim();
    if email.is_empty() || body.password.is_empty() {
        return Err(ApiError::bad_request("email and password are required"));
    }
    // A bad pair surfaces as `AuthError::InvalidCredentials` → 401.
    let tokens = st.provider.exchange_password(email, &body.password).await?;
    let cookies = establish_session(&st, tokens).await?;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set_cookies(&mut resp, cookies);
    Ok(resp)
}

/// `POST /api/auth/logout` → revoke the server session and return the IdP end-session
/// URL as JSON. RP-initiated logout is a *top-level* browser navigation (the SPA can't
/// `fetch()`-follow a cross-origin redirect to Keycloak), so we hand back the URL rather
/// than 30x-redirecting; the session/CSRF cookies are still cleared on this 200.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Session revoked; returns the IdP end-session URL as { \"logout_url\": ... } (clears cookies)", body = Object)
    )
)]
pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let mut id_token_hint = None;
    if let Some(sid) = cookie::parse(&headers, &st.config.cookie_name) {
        // Grab the id_token for RP-initiated logout before dropping the session.
        if let Ok(Some(rec)) = st.sessions.get(sid).await {
            id_token_hint = rec.id_token;
        }
        let _ = st.sessions.delete(sid).await;
    }
    let clear = cookie::clear(&st.config.cookie_name, st.config.cookie_secure);
    let clear_csrf = csrf::build_cookie(CSRF_COOKIE, "", 0, st.config.cookie_secure);
    let url = st
        .provider
        .logout_url(id_token_hint.as_deref(), &st.config.frontend_url);
    let mut resp = Json(json!({ "logout_url": url })).into_response();
    set_cookies(&mut resp, [clear, clear_csrf]);
    resp
}

/// `GET /api/auth/me` → current principal + resolved tenant (guarded route).
#[utoipa::path(
    get,
    path = "/auth/me",
    tag = "auth",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Current principal and resolved tenant", body = Object),
        (status = 401, description = "Missing or invalid session")
    )
)]
pub async fn me(
    State(st): State<AppState>,
    identity: oidc_auth::AuthIdentity,
) -> ApiResult<Json<Value>> {
    let tenant = resolve_tenant(&st.db, &identity).await?;
    Ok(Json(json!({
        "subject": identity.subject,
        "email": identity.email,
        "name": identity.name,
        "roles": identity.roles,
        "org_id": tenant.org_id,
        "role": tenant.role,
    })))
}

// --- helpers --------------------------------------------------------------------

/// Verify a fresh token set, provision the tenant, mint a server-side session, and
/// return the session + readable-CSRF cookies. Shared by the callback (redirect) and
/// password-login (native form) paths.
async fn establish_session(st: &AppState, tokens: TokenSet) -> ApiResult<Vec<String>> {
    let identity = st.verifier.verify(&tokens.access_token).await?;
    // Provision internal user/org/membership and record the login.
    provision_tenant(&st.db, &identity).await?;
    db::touch_last_login(&st.db, &identity.subject).await?;

    let now = now_secs();
    let record = SessionRecord {
        subject: identity.subject,
        org_id: identity.org_id,
        roles: identity.roles,
        email: identity.email,
        name: identity.name,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        access_expires_at: now + tokens.expires_in.unwrap_or(300),
        session_expires_at: now + st.config.session_ttl_secs,
    };
    let sid = st
        .sessions
        .create(&record, st.config.session_ttl_secs)
        .await?;
    let session_cookie = cookie::build(
        &st.config.cookie_name,
        &sid,
        st.config.session_ttl_secs,
        st.config.cookie_secure,
    );
    // Readable double-submit token for state-changing requests (see oidc_auth::csrf).
    let csrf_cookie = csrf::build_cookie(
        CSRF_COOKIE,
        &csrf::token(),
        st.config.session_ttl_secs,
        st.config.cookie_secure,
    );
    Ok(vec![session_cookie, csrf_cookie])
}

fn redirect_with_cookies(location: &str, cookies: impl IntoIterator<Item = String>) -> Response {
    let mut resp = Redirect::to(location).into_response();
    set_cookies(&mut resp, cookies);
    resp
}

fn set_cookies(resp: &mut Response, cookies: impl IntoIterator<Item = String>) {
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(&c) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
