use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{AppendHeaders, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_helpers::{ACCESS_TOKEN_TTL, JwtRedisAuth, REFRESH_TOKEN_TTL, ValidatedJson};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::OpenApi;

use crate::error::UserError;
use crate::models::{LoginRequest, LoginResponse, RegisterRequest, UserResponse};
use crate::oauth::providers::OAuthProvider;
use crate::oauth::providers::github::GithubProvider;
use crate::oauth::providers::google::GoogleProvider;
use crate::oauth::{
    AccountLinkingResult, AccountLinkingService, OAuthAccountRepository, OAuthState,
    OAuthStateManager,
};
use crate::repository::UserRepository;
use crate::service::UserService;

/// OpenAPI documentation for Auth API
#[derive(OpenApi)]
#[openapi(
    paths(
        register,
        login,
        logout,
        me,
        authorize,
    ),
    components(
        schemas(
            RegisterRequest,
            LoginRequest,
            LoginResponse,
            UserResponse
        )
    ),
    tags(
        (name = "auth", description = "Authentication endpoints")
    )
)]
pub struct AuthApiDoc;

/// OAuth configuration
#[derive(Clone)]
pub struct OAuthConfig {
    pub google_client_id: String,
    pub google_client_secret: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    pub redirect_base_url: String,
    pub frontend_url: String,
}

/// Application state for auth handlers
#[derive(Clone)]
pub struct AuthState<R: UserRepository, O: OAuthAccountRepository> {
    pub service: UserService<R>,
    pub oauth_config: OAuthConfig,
    pub jwt_auth: JwtRedisAuth,
    pub oauth_state_manager: OAuthStateManager,
    pub account_linking: AccountLinkingService<R, O>,
    /// Optional notification service for sending emails (requires `notifications` feature)
    #[cfg(feature = "notifications")]
    pub notifications: Option<email::NotificationService>,
}

/// Check if running in development mode
fn is_development() -> bool {
    std::env::var("APP_ENV")
        .map(|env| env == "development")
        .unwrap_or_else(|_| cfg!(debug_assertions))
}

/// Register a new user
#[utoipa::path(
    post,
    path = "/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "Registration successful", body = LoginResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Email already exists"),
        (status = 500, description = "Internal server error")
    )
)]
async fn register<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    ValidatedJson(input): ValidatedJson<RegisterRequest>,
) -> Result<Response, UserError> {
    // Create user
    let user = state
        .service
        .create_user(crate::models::CreateUser {
            email: input.email,
            name: input.name,
            password: input.password,
            roles: vec![],
        })
        .await?;

    // Queue welcome email (if notifications feature is enabled)
    #[cfg(feature = "notifications")]
    if let Some(ref notifications) = state.notifications {
        // TODO: Add email verification flow with token
        if let Err(e) = notifications
            .queue_welcome_email(user.id, &user.email, &user.name, false, None)
            .await
        {
            // Log error but don't fail registration
            tracing::warn!(
                user_id = %user.id,
                email = %user.email,
                error = %e,
                "Failed to queue welcome email"
            );
        } else {
            tracing::info!(
                user_id = %user.id,
                email = %user.email,
                "Welcome email queued successfully"
            );
        }
    }

    let user_id = user.id.to_string();

    // Create access token
    let access_token = state
        .jwt_auth
        .create_access_token(&user_id, &user.email, &user.name, &user.roles)
        .map_err(|e| {
            tracing::error!("Failed to create access token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    // Verify and whitelist access token
    let access_claims = state.jwt_auth.verify_token(&access_token).map_err(|e| {
        tracing::error!("Failed to verify access token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&access_claims.jti, &user_id, ACCESS_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist access token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    // Create refresh token
    let refresh_token = state
        .jwt_auth
        .create_refresh_token(&user_id, &user.email, &user.name, &user.roles)
        .map_err(|e| {
            tracing::error!("Failed to create refresh token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    // Verify and whitelist refresh token
    let refresh_claims = state.jwt_auth.verify_token(&refresh_token).map_err(|e| {
        tracing::error!("Failed to verify refresh token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&refresh_claims.jti, &user_id, REFRESH_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist refresh token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    // Create cookies
    let secure_flag = if is_development() { "" } else { " Secure;" };
    let access_cookie = format!(
        "access_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
        access_token, secure_flag, ACCESS_TOKEN_TTL
    );
    let refresh_cookie = format!(
        "refresh_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
        refresh_token, secure_flag, REFRESH_TOKEN_TTL
    );

    let response = LoginResponse { user };

    let access_cookie_header = HeaderValue::from_str(&access_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;
    let refresh_cookie_header = HeaderValue::from_str(&refresh_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, access_cookie_header),
            (header::SET_COOKIE, refresh_cookie_header),
        ]),
        Json(response),
    )
        .into_response())
}

/// Login with email/password
#[utoipa::path(
    post,
    path = "/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Invalid credentials"),
        (status = 500, description = "Internal server error")
    )
)]
async fn login<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    ValidatedJson(input): ValidatedJson<LoginRequest>,
) -> Result<Response, UserError> {
    // Verify credentials
    let user = state
        .service
        .verify_credentials(&input.email, &input.password)
        .await?;

    let user_id = user.id.to_string();

    // Create JWT tokens
    let access_token = state
        .jwt_auth
        .create_access_token(&user_id, &user.email, &user.name, &user.roles)
        .map_err(|e| {
            tracing::error!("Failed to create access token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    let access_claims = state.jwt_auth.verify_token(&access_token).map_err(|e| {
        tracing::error!("Failed to verify access token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&access_claims.jti, &user_id, ACCESS_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist access token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    let refresh_token = state
        .jwt_auth
        .create_refresh_token(&user_id, &user.email, &user.name, &user.roles)
        .map_err(|e| {
            tracing::error!("Failed to create refresh token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    let refresh_claims = state.jwt_auth.verify_token(&refresh_token).map_err(|e| {
        tracing::error!("Failed to verify refresh token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&refresh_claims.jti, &user_id, REFRESH_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist refresh token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    // Create cookies
    let secure_flag = if is_development() { "" } else { " Secure;" };
    let access_cookie = format!(
        "access_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
        access_token, secure_flag, ACCESS_TOKEN_TTL
    );
    let refresh_cookie = format!(
        "refresh_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
        refresh_token, secure_flag, REFRESH_TOKEN_TTL
    );

    let response = LoginResponse { user };

    let access_cookie_header = HeaderValue::from_str(&access_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;
    let refresh_cookie_header = HeaderValue::from_str(&refresh_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, access_cookie_header),
            (header::SET_COOKIE, refresh_cookie_header),
        ]),
        Json(response),
    )
        .into_response())
}

/// Logout
#[utoipa::path(
    post,
    path = "/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Logout successful"),
        (status = 500, description = "Internal server error")
    )
)]
async fn logout<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, UserError> {
    // Extract tokens from cookies
    let cookies_str = headers.get("cookie").and_then(|v| v.to_str().ok());

    if let Some(cookies) = cookies_str {
        // Revoke access token if present
        if let Some(access_token) = extract_cookie_value(cookies, "access_token")
            && let Ok(claims) = state.jwt_auth.verify_token(&access_token)
        {
            let now = chrono::Utc::now().timestamp();
            let remaining_ttl = (claims.exp - now).max(0) as u64;

            let _ = state.jwt_auth.revoke_token(&claims.jti).await;
            let _ = state
                .jwt_auth
                .blacklist_token(&claims.jti, remaining_ttl)
                .await;
            tracing::debug!("Revoked and blacklisted access token: {}", claims.jti);
        }

        // Revoke refresh token if present
        if let Some(refresh_token) = extract_cookie_value(cookies, "refresh_token")
            && let Ok(claims) = state.jwt_auth.verify_token(&refresh_token)
        {
            let now = chrono::Utc::now().timestamp();
            let remaining_ttl = (claims.exp - now).max(0) as u64;

            let _ = state.jwt_auth.revoke_token(&claims.jti).await;
            let _ = state
                .jwt_auth
                .blacklist_token(&claims.jti, remaining_ttl)
                .await;
            tracing::debug!("Revoked and blacklisted refresh token: {}", claims.jti);
        }
    }

    // Clear cookies
    let secure_flag = if is_development() { "" } else { " Secure;" };
    let clear_access = format!(
        "access_token=; HttpOnly;{} SameSite=Strict; Path=/; Max-Age=0",
        secure_flag
    );
    let clear_refresh = format!(
        "refresh_token=; HttpOnly;{} SameSite=Strict; Path=/; Max-Age=0",
        secure_flag
    );

    let clear_access_header = HeaderValue::from_str(&clear_access)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;
    let clear_refresh_header = HeaderValue::from_str(&clear_refresh)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, clear_access_header),
            (header::SET_COOKIE, clear_refresh_header),
        ]),
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

/// Get current user from JWT claims
#[utoipa::path(
    get,
    path = "/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current user info", body = UserResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn me<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<crate::models::UserResponse>, UserError> {
    // Extract token from Authorization header or cookie
    let token = extract_token(&headers).ok_or(UserError::Unauthorized)?;

    // Verify token
    let claims = state
        .jwt_auth
        .verify_token(&token)
        .map_err(|_| UserError::Unauthorized)?;

    // Check not blacklisted
    if state
        .jwt_auth
        .is_token_blacklisted(&claims.jti)
        .await
        .map_err(|e| {
            tracing::error!("Redis error checking blacklist: {}", e);
            UserError::Internal("Service temporarily unavailable".to_string())
        })?
    {
        return Err(UserError::Unauthorized);
    }

    // Check whitelisted
    if !state
        .jwt_auth
        .is_token_whitelisted(&claims.jti)
        .await
        .map_err(|e| {
            tracing::error!("Redis error checking whitelist: {}", e);
            UserError::Internal("Service temporarily unavailable".to_string())
        })?
    {
        return Err(UserError::Unauthorized);
    }

    // Get full user from database
    let user_id = uuid::Uuid::parse_str(&claims.sub).map_err(|_| UserError::Unauthorized)?;

    let user = state.service.get_user(user_id).await?;

    Ok(Json(user))
}

/// Helper: Extract token from Authorization header or cookie
fn extract_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer ").map(|s| s.to_string()))
        .or_else(|| {
            extract_cookie_value(
                headers.get("cookie").and_then(|v| v.to_str().ok())?,
                "access_token",
            )
        })
}

/// Helper: Extract cookie value by name
fn extract_cookie_value(cookies: &str, name: &str) -> Option<String> {
    cookies.split(';').find_map(|cookie| {
        let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
        if parts.len() == 2 && parts[0] == name {
            Some(parts[1].to_string())
        } else {
            None
        }
    })
}

/// Extract host:port from a URL string (e.g. "https://example.com:3000/path" -> "example.com:3000")
/// Includes port when present so scheme-swapping attacks can't bypass validation.
fn extract_host_port(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    Some(after_scheme.split('/').next().unwrap_or(after_scheme))
}

/// Derive the origin URL from request headers (X-Forwarded-Proto + Host)
/// Returns None if headers are missing, allowing fallback to configured values
fn derive_origin_url(headers: &axum::http::HeaderMap) -> Option<String> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())?;

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");

    Some(format!("{}://{}", scheme, host))
}

/// Query parameters for OAuth callback
#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    code: String,
    // TODO: Implement CSRF state validation with PKCE (see terran's OAuthStateManager)
    #[allow(dead_code)]
    state: String,
}

/// Generate a secure random password that meets all validation requirements
#[allow(dead_code)]
fn generate_oauth_password() -> String {
    use rand::RngExt;
    const CHARSET_LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const CHARSET_UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const CHARSET_DIGIT: &[u8] = b"0123456789";
    const CHARSET_SPECIAL: &[u8] = b"!@#$%^&*()_+-=[]{}|;:,.<>?";

    let mut rng = rand::rng();
    let mut password = String::new();

    // Ensure at least one of each required character type
    password.push(CHARSET_UPPER[rng.random_range(0..CHARSET_UPPER.len())] as char);
    password.push(CHARSET_LOWER[rng.random_range(0..CHARSET_LOWER.len())] as char);
    password.push(CHARSET_DIGIT[rng.random_range(0..CHARSET_DIGIT.len())] as char);
    password.push(CHARSET_SPECIAL[rng.random_range(0..CHARSET_SPECIAL.len())] as char);

    // Add remaining random characters (total 20 chars)
    let all_chars = [CHARSET_LOWER, CHARSET_UPPER, CHARSET_DIGIT, CHARSET_SPECIAL].concat();
    for _ in 0..16 {
        password.push(all_chars[rng.random_range(0..all_chars.len())] as char);
    }

    // Shuffle to avoid predictable pattern
    let mut chars: Vec<char> = password.chars().collect();
    for i in (1..chars.len()).rev() {
        let j = rng.random_range(0..=i);
        chars.swap(i, j);
    }

    chars.into_iter().collect()
}

/// Get OAuth provider by name
fn get_provider(
    provider_name: &str,
    config: &OAuthConfig,
) -> Result<Arc<dyn OAuthProvider>, UserError> {
    match provider_name.to_lowercase().as_str() {
        "google" => Ok(Arc::new(GoogleProvider::new(
            config.google_client_id.clone(),
            config.google_client_secret.clone(),
        )) as Arc<dyn OAuthProvider>),
        "github" => Ok(Arc::new(GithubProvider::new(
            config.github_client_id.clone(),
            config.github_client_secret.clone(),
        )) as Arc<dyn OAuthProvider>),
        _ => Err(UserError::OAuth(format!(
            "Unsupported provider: {}",
            provider_name
        ))),
    }
}

/// Initiate OAuth flow for any provider
#[utoipa::path(
    get,
    path = "/oauth/{provider}",
    tag = "auth",
    params(
        ("provider" = String, Path, description = "OAuth provider name (google, github)")
    ),
    responses(
        (status = 302, description = "Redirect to OAuth provider"),
        (status = 400, description = "Unsupported provider"),
        (status = 500, description = "Internal server error")
    )
)]
async fn authorize<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    Path(provider_name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Redirect, UserError> {
    let provider = get_provider(&provider_name, &state.oauth_config)?;

    // Origin URL from request headers — used for post-login redirect to the frontend.
    // OAuth callback URI always uses redirect_base_url (must match Google Console config).
    // Validate that derived host matches frontend_url to prevent open redirect attacks.
    let origin_url = derive_origin_url(&headers)
        .filter(|derived| {
            extract_host_port(derived) == extract_host_port(&state.oauth_config.frontend_url)
        })
        .unwrap_or_else(|| state.oauth_config.frontend_url.clone());

    let redirect_uri = format!(
        "{}/api/auth/oauth/{}/callback",
        state.oauth_config.redirect_base_url,
        provider.name()
    );
    tracing::info!("{} OAuth redirect URI: {}", provider_name, redirect_uri);

    // Generate CSRF state and PKCE verifier
    let csrf_state = state.oauth_state_manager.generate_state();
    let pkce_verifier = state.oauth_state_manager.generate_pkce_verifier();

    // Store state in Redis for validation in callback
    let oauth_state = OAuthState {
        state: csrf_state.clone(),
        pkce_verifier: pkce_verifier.clone(),
        nonce: None,
        redirect_uri: redirect_uri.clone(),
        provider: provider.name().to_string(),
        origin_url: Some(origin_url),
    };
    state.oauth_state_manager.store_state(&oauth_state).await?;

    // Build authorization URL with PKCE
    use oauth2::{
        AuthUrl, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
        RedirectUrl, Scope, basic::BasicClient,
    };

    let client = BasicClient::new(ClientId::new(provider.client_id().to_string()))
        .set_client_secret(ClientSecret::new(provider.client_secret().to_string()))
        .set_auth_uri(
            AuthUrl::new(provider.auth_url().to_string())
                .map_err(|e| UserError::OAuth(format!("Invalid auth URL: {}", e)))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri)
                .map_err(|e| UserError::OAuth(format!("Invalid redirect URL: {}", e)))?,
        );

    // Generate PKCE challenge from verifier
    let pkce_verifier_obj = PkceCodeVerifier::new(pkce_verifier);
    let pkce_challenge = PkceCodeChallenge::from_code_verifier_sha256(&pkce_verifier_obj);

    let mut auth_request = client
        .authorize_url(|| CsrfToken::new(csrf_state))
        .set_pkce_challenge(pkce_challenge);

    for scope in provider.required_scopes() {
        auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
    }

    let (auth_url, _csrf_token) = auth_request.url();

    Ok(Redirect::to(auth_url.as_str()))
}

/// Handle OAuth callback for any provider
async fn callback<R: UserRepository, O: OAuthAccountRepository>(
    State(state): State<AuthState<R, O>>,
    Path(provider_name): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Response, UserError> {
    tracing::info!("OAuth callback started for provider: {}", provider_name);

    // Verify and consume OAuth state (CSRF protection + retrieve PKCE verifier)
    let oauth_state = state
        .oauth_state_manager
        .verify_and_consume_state(&query.state)
        .await
        .map_err(|e| {
            tracing::error!("Failed to verify OAuth state: {:?}", e);
            e
        })?;

    tracing::debug!("OAuth state verified successfully");

    // Verify provider matches
    let provider = get_provider(&provider_name, &state.oauth_config)?;
    if oauth_state.provider != provider.name() {
        return Err(UserError::OAuth("Provider mismatch".to_string()));
    }

    // Use the trait's exchange_code method with PKCE verification
    let token_response = provider
        .exchange_code(
            &query.code,
            &oauth_state.pkce_verifier,
            &oauth_state.redirect_uri,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to exchange OAuth code: {:?}", e);
            e
        })?;

    tracing::debug!("Token exchange successful");

    let access_token = token_response.access_token.clone();
    let refresh_token = token_response.refresh_token;
    let expires_in = token_response.expires_in;

    // Fetch user info from provider
    let user_info = provider.get_user_info(&access_token).await.map_err(|e| {
        tracing::error!("Failed to get user info from provider: {:?}", e);
        e
    })?;

    tracing::info!(
        "User info retrieved: email={:?}, name={:?}",
        user_info.email,
        user_info.name
    );

    // Use AccountLinkingService to handle account linking logic
    let linking_result = state
        .account_linking
        .handle_oauth_login(
            provider.name(),
            user_info,
            Some(access_token),
            refresh_token,
            expires_in,
            true, // auto_link_verified_emails
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to handle account linking: {:?}", e);
            e
        })?;

    tracing::debug!("Account linking completed successfully");

    // Handle different linking results
    let user = match linking_result {
        AccountLinkingResult::NewUser(user) | AccountLinkingResult::ExistingUser(user) => user,
        AccountLinkingResult::LinkRequired {
            existing_user_id: _,
            provider_data,
        } => {
            // For now, return an error. In a real app, you'd redirect to a linking confirmation page
            return Err(UserError::OAuth(format!(
                "Account linking required. An account with email '{}' already exists. Please log in first and link your {} account from your profile.",
                provider_data.email.unwrap_or_default(),
                provider_name
            )));
        }
    };

    let user_id = user.id.to_string();
    let user_roles: Vec<String> = user.roles.iter().map(|r| r.to_string()).collect();

    // Create JWT tokens
    let access_token = state
        .jwt_auth
        .create_access_token(&user_id, &user.email, &user.name, &user_roles)
        .map_err(|e| {
            tracing::error!("Failed to create access token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    let access_claims = state.jwt_auth.verify_token(&access_token).map_err(|e| {
        tracing::error!("Failed to verify access token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&access_claims.jti, &user_id, ACCESS_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist access token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    let refresh_token = state
        .jwt_auth
        .create_refresh_token(&user_id, &user.email, &user.name, &user_roles)
        .map_err(|e| {
            tracing::error!("Failed to create refresh token: {:?}", e);
            UserError::Internal("Failed to create token".to_string())
        })?;

    let refresh_claims = state.jwt_auth.verify_token(&refresh_token).map_err(|e| {
        tracing::error!("Failed to verify refresh token: {:?}", e);
        UserError::Internal("Failed to verify token".to_string())
    })?;

    state
        .jwt_auth
        .whitelist_token(&refresh_claims.jti, &user_id, REFRESH_TOKEN_TTL as u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to whitelist refresh token: {:?}", e);
            UserError::Internal("Failed to whitelist token".to_string())
        })?;

    // Create cookies for redirect
    let secure_flag = if is_development() { "" } else { " Secure;" };
    let access_cookie = format!(
        "access_token={}; HttpOnly;{} SameSite=Lax; Path=/; Max-Age={}",
        access_token, secure_flag, ACCESS_TOKEN_TTL
    );
    let refresh_cookie = format!(
        "refresh_token={}; HttpOnly;{} SameSite=Lax; Path=/; Max-Age={}",
        refresh_token, secure_flag, REFRESH_TOKEN_TTL
    );

    // Redirect to frontend with cookies set
    // Use the origin from the OAuth state (where the user started) or fall back to configured frontend_url
    let frontend_base = oauth_state
        .origin_url
        .unwrap_or_else(|| state.oauth_config.frontend_url.clone());
    let redirect_url = format!("{}/tasks", frontend_base);

    let access_cookie_header = HeaderValue::from_str(&access_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;
    let refresh_cookie_header = HeaderValue::from_str(&refresh_cookie)
        .map_err(|e| UserError::Internal(format!("Failed to create cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, access_cookie_header),
            (header::SET_COOKIE, refresh_cookie_header),
            (
                header::LOCATION,
                HeaderValue::from_str(&redirect_url).unwrap(),
            ),
        ]),
        StatusCode::FOUND,
    )
        .into_response())
}

/// Create auth router
pub fn auth_router<R, O>(state: AuthState<R, O>) -> Router
where
    R: UserRepository + Clone + Send + Sync + 'static,
    O: OAuthAccountRepository + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/register", post(register::<R, O>))
        .route("/login", post(login::<R, O>))
        .route("/logout", post(logout::<R, O>))
        .route("/me", get(me::<R, O>))
        .route("/oauth/{provider}", get(authorize::<R, O>))
        .route("/oauth/{provider}/callback", get(callback::<R, O>))
        .with_state(state)
}
