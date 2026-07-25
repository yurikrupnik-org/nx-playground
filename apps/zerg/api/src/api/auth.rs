//! Token-handler / BFF auth endpoints backed by WorkOS AuthKit. The browser only
//! ever holds an opaque session id; the IdP tokens are exchanged here and stored
//! server-side (mirrors `apps/terran/api/src/auth.rs`, provider swapped).
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
use domain_users::{PostgresUserRepository, UserResponse, UserService};
use oidc_auth::{IdentityProvider, SessionRecord, SessionStore, TokenSet, cookie};
use oidc_auth::{LoginFlow, UserProfile};
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Short-lived cookie carrying the opaque login-flow id between authorize and callback.
const FLOW_COOKIE: &str = "zerg_flow";
/// Readable double-submit CSRF cookie issued alongside the session. MUST stay
/// `csrf_token`: the SPA (`apps/zerg/web/src/lib/csrf.ts`) and the router's
/// `CsrfConfig::new("csrf_token")` both hardcode it.
pub const CSRF_COOKIE: &str = "csrf_token";
/// Lifetime of an in-flight login (authorize → callback window).
pub const FLOW_TTL_SECS: u64 = 600;

fn user_service(st: &AppState) -> UserService<PostgresUserRepository> {
    UserService::new(PostgresUserRepository::new(st.db.clone()))
}

#[derive(Deserialize)]
pub struct LoginQuery {
    /// Provider hint: `google`/`github` deep-link the social provider; `sign-up`
    /// lands on AuthKit's sign-up screen; absent → hosted AuthKit sign-in.
    #[serde(default)]
    idp: Option<String>,
}

/// `GET /api/auth/login` → redirect to the WorkOS authorize endpoint (PKCE + CSRF state).
#[utoipa::path(
    get,
    path = "/auth/login",
    tag = "auth",
    params(("idp" = Option<String>, Query, description = "Provider hint: google, github, or sign-up")),
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

/// `GET /api/auth/callback` → exchange code, provision the user, mint a server-side session.
#[utoipa::path(
    get,
    path = "/auth/callback",
    tag = "auth",
    params(
        ("code" = Option<String>, Query, description = "Authorization code from WorkOS"),
        ("state" = Option<String>, Query, description = "CSRF state echoed by WorkOS"),
        ("error" = Option<String>, Query, description = "Error code if WorkOS rejected the request")
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
        tracing::warn!(error = %err, "workos returned an error on callback");
        return Ok(
            Redirect::to(&format!("{}/login?auth_error=1", st.config.frontend_url)).into_response(),
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
    let landing = format!("{}/tasks", st.config.frontend_url);
    Ok(redirect_with_cookies(&landing, cookies))
}

/// Native email+password login form payload.
#[derive(Deserialize, ToSchema)]
pub struct PasswordLogin {
    pub email: String,
    pub password: String,
}

/// `POST /api/auth/login/password` → native email+password sign-in via WorkOS's
/// password grant. Establishes the same server-side session as the redirect flow
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

/// `POST /api/auth/logout` → revoke the server session and return the WorkOS
/// end-session URL as JSON. RP-initiated logout is a *top-level* browser navigation
/// (the SPA can't `fetch()`-follow a cross-origin redirect), so we hand back the URL
/// rather than 30x-redirecting; the session/CSRF cookies are still cleared on this 200.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Session revoked; returns the IdP end-session URL as { \"logout_url\": ... } (clears cookies)", body = Object)
    )
)]
pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let mut logout_hint = None;
    if let Some(sid) = cookie::parse(&headers, &st.config.cookie_name) {
        // WorkOS session logout needs the access token's `sid` claim — grab the
        // stored access token before dropping the session.
        if let Ok(Some(rec)) = st.sessions.get(sid).await {
            logout_hint = Some(rec.access_token);
        }
        let _ = st.sessions.delete(sid).await;
    }
    let post_logout = format!("{}/login", st.config.frontend_url);
    let clear = cookie::clear(&st.config.cookie_name, st.config.cookie_secure);
    let clear_csrf = csrf::build_cookie(CSRF_COOKIE, "", 0, st.config.cookie_secure);
    let url = st.provider.logout_url(logout_hint.as_deref(), &post_logout);
    let mut resp = Json(json!({ "logout_url": url })).into_response();
    set_cookies(&mut resp, [clear, clear_csrf]);
    resp
}

/// `GET /api/auth/me` → current user profile + active org context (guarded route).
///
/// Response is the flattened [`UserResponse`] plus an `org` object (terran's `Me`
/// shape) so the SPA can render tenant context and gate admin UI.
#[derive(serde::Serialize, ToSchema)]
pub struct MeResponse {
    #[serde(flatten)]
    pub user: UserResponse,
    pub org: OrgInfo,
}

#[derive(serde::Serialize, ToSchema)]
pub struct OrgInfo {
    pub id: uuid::Uuid,
    pub external_id: String,
    pub name: String,
    pub role: String,
    pub is_personal: bool,
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current user with org context", body = MeResponse),
        (status = 401, description = "Not authenticated")
    )
)]
pub async fn me(
    State(st): State<AppState>,
    identity: oidc_auth::AuthIdentity,
) -> ApiResult<Json<MeResponse>> {
    // A session always follows JIT provisioning, so a missing row means the
    // principal was deleted out-of-band — treat as unauthenticated.
    let user = user_service(&st)
        .get_user_by_subject(&identity.subject)
        .await
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "unknown principal"))?;
    let tenant = crate::orgs::resolve_tenant(&st, &identity).await?;
    Ok(Json(MeResponse {
        user,
        org: OrgInfo {
            id: tenant.org_id,
            external_id: tenant.external_org_id.clone(),
            name: tenant.org_name.clone(),
            role: tenant.role.clone(),
            is_personal: tenant.is_personal(),
        },
    }))
}

// --- helpers --------------------------------------------------------------------

/// Verify a fresh token set, JIT-provision the local user, mint a server-side
/// session, and return the session + readable-CSRF cookies. Shared by the callback
/// (redirect) and password-login (native form) paths.
async fn establish_session(st: &AppState, tokens: TokenSet) -> ApiResult<Vec<String>> {
    let identity = st.verifier.verify(&tokens.access_token).await?;

    // Profile: the WorkOS access token carries no email/name; the authenticate
    // response's `user` object (TokenSet::profile) fills the gap.
    let profile = tokens.profile.as_ref();
    let email = identity
        .email
        .clone()
        .or_else(|| profile.and_then(|p: &UserProfile| p.email.clone()));
    let name = identity
        .name
        .clone()
        .or_else(|| profile.and_then(|p: &UserProfile| p.name.clone()));

    // JIT provisioning: find by subject → link by email → create. Welcome email
    // only on first creation (parity with the old register handler; non-fatal).
    let email = email.ok_or(ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "identity provider returned no email",
    ))?;
    let (user, created) = user_service(st)
        .provision_oidc_user(&identity.subject, &email, name.as_deref())
        .await?;
    if created
        && let Err(e) = st
            .notifications
            .queue_welcome_email(user.id, &user.email, &user.name, false, None)
            .await
    {
        tracing::warn!(error = %e, user_id = %user.id, "failed to queue welcome email");
    }

    // Eager tenant provisioning: mirrors the org + membership locally so invited
    // B2B users (token carries the org's `org_id`) are queryable on first login.
    if let Err(e) = crate::orgs::provision_tenant(st, &identity, user.id, Some(&user.name)).await {
        // Non-fatal: the tenant middleware re-provisions on the first API call.
        tracing::warn!(user_id = %user.id, "tenant provisioning at login failed: {e:?}");
    }

    let now = now_secs();
    let record = SessionRecord {
        subject: identity.subject,
        org_id: identity.org_id,
        roles: identity.roles,
        email: Some(user.email.clone()),
        name: Some(user.name.clone()),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens.id_token,
        // WorkOS reports no expires_in; 300s forces a refresh at least every 5
        // minutes (matching WorkOS's default access-token life).
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
        .unwrap_or_default()
        .as_secs()
}
