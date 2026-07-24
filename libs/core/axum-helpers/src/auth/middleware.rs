use super::jwt::JwtRedisAuth;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Extract JWT from Authorization header or cookie
fn extract_token_from_request(headers: &HeaderMap) -> Option<String> {
    // Try Authorization header first: "Bearer <token>"
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer ").map(|s| s.to_string()))
        .or_else(|| {
            // Fallback to cookie: "access_token=<token>"
            headers
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .and_then(|cookies| {
                    cookies.split(';').find_map(|cookie| {
                        let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
                        if parts.len() == 2 && parts[0] == "access_token" {
                            Some(parts[1].to_string())
                        } else {
                            None
                        }
                    })
                })
        })
}

/// JWT authentication middleware
///
/// Validates JWT tokens from Authorization header or cookies.
/// Checks token signature, blacklist, and whitelist in Redis.
/// Inserts JwtClaims into request extensions on success.
///
/// # Example
///
/// ```ignore
/// use axum::Router;
/// use axum::routing::get;
/// use axum_helpers::{JwtRedisAuth, middleware::jwt_auth_middleware};
///
/// let auth = JwtRedisAuth::new(redis_manager, Some("secret")).unwrap();
///
/// let protected_routes = Router::new()
///     .route("/api/protected", get(protected_handler))
///     .layer(axum::middleware::from_fn_with_state(
///         auth.clone(),
///         jwt_auth_middleware
///     ));
/// ```
pub async fn jwt_auth_middleware(
    State(auth): State<JwtRedisAuth>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let Some(token) = extract_token_from_request(&headers) else {
        tracing::debug!("No JWT found in Authorization header or cookie");
        return Err((StatusCode::UNAUTHORIZED, "No token provided"));
    };

    // Verify signature + claims; require an access token (a refresh token must not
    // authenticate API calls).
    let claims = match auth.verify_access_token(&token) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("JWT verification failed: {}", e);
            return Err((StatusCode::UNAUTHORIZED, "Invalid token"));
        }
    };

    // Check if token is blacklisted
    match auth.is_token_blacklisted(&claims.jti).await {
        Ok(true) => {
            tracing::debug!("Token is blacklisted: {}", claims.jti);
            return Err((StatusCode::UNAUTHORIZED, "Token has been revoked"));
        }
        Ok(false) => {}
        Err(e) => {
            tracing::error!("Redis error checking blacklist: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Service temporarily unavailable",
            ));
        }
    }

    // Check if token is whitelisted
    match auth.is_token_whitelisted(&claims.jti).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!("Token is not whitelisted: {}", claims.jti);
            return Err((StatusCode::UNAUTHORIZED, "Token not found"));
        }
        Err(e) => {
            tracing::error!("Redis error checking whitelist: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Service temporarily unavailable",
            ));
        }
    }

    // Token is valid - insert claims into request extensions
    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

/// Optional JWT authentication middleware
///
/// Like jwt_auth_middleware but doesn't fail if no token is present.
/// Useful for endpoints that behave differently for authenticated vs anonymous users.
pub async fn optional_jwt_auth_middleware(
    State(auth): State<JwtRedisAuth>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    if let Some(token) = extract_token_from_request(&headers)
        && let Ok(claims) = auth.verify_access_token(&token)
    {
        // Check if token is valid (not blacklisted, is whitelisted)
        let is_blacklisted = auth.is_token_blacklisted(&claims.jti).await.unwrap_or(true);
        let is_whitelisted = auth
            .is_token_whitelisted(&claims.jti)
            .await
            .unwrap_or(false);

        if !is_blacklisted && is_whitelisted {
            request.extensions_mut().insert(claims);
        }
    }

    next.run(request).await
}
