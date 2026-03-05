use super::{RateLimitTier, RateLimiter};
use crate::auth::jwt::JwtClaims;
use crate::errors::AppError;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

/// Extract the rate limit key from the request.
///
/// Strategy:
/// 1. Authenticated user ID from `JwtClaims` in extensions -> `user:<id>`
/// 2. `X-Real-Ip` header (set by nginx/ingress, single trusted value) -> `ip:<ip>`
/// 3. `X-Forwarded-For` rightmost IP (last entry = added by our proxy) -> `ip:<ip>`
/// 4. TCP socket peer address (`ConnectInfo`) -> `ip:<ip>`
/// 5. Fallback -> `ip:unknown`
fn extract_key(request: &Request) -> String {
    // Check for authenticated user (only populated if auth middleware ran first)
    if let Some(claims) = request.extensions().get::<JwtClaims>() {
        return format!("user:{}", claims.sub);
    }

    let headers = request.headers();

    // X-Real-Ip: single value set by trusted reverse proxy (nginx ingress default)
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() {
            return format!("ip:{}", ip);
        }
    }

    // X-Forwarded-For: take the RIGHTMOST entry (added by our trusted proxy).
    // The leftmost entry is client-controlled and trivially spoofable.
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(ip) = forwarded.rsplit(',').next().map(|s| s.trim()) {
            if !ip.is_empty() {
                return format!("ip:{}", ip);
            }
        }
    }

    // Fall back to TCP socket peer address
    if let Some(connect_info) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return format!("ip:{}", connect_info.0.ip());
    }

    tracing::warn!("Could not determine client IP for rate limiting - using shared 'ip:unknown' key");
    "ip:unknown".to_string()
}

/// Rate limiting middleware for Axum.
///
/// Identifies callers by user ID (if auth ran first) or client IP.
///
/// Behavior:
/// - **Allowed:** Adds `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset` headers
/// - **Denied:** Returns 429 with `Retry-After` header and JSON error body
/// - **Redis failure:** Fail-open (allow request), log warning
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    if !limiter.is_enabled() {
        return next.run(request).await;
    }

    // Routes without a RateLimitTier extension are exempt (e.g. auth endpoints).
    let tier = match request.extensions().get::<RateLimitTier>() {
        Some(t) => t.clone(),
        None => return next.run(request).await,
    };

    let key = extract_key(&request);
    let limit = tier.requests_per_window;

    match limiter
        .check_with_config(&key, &tier.name, tier.requests_per_window, tier.window_secs)
        .await
    {
        Ok(result) => {
            if result.allowed {
                let mut response = next.run(request).await;
                insert_rate_limit_headers(
                    response.headers_mut(),
                    limit,
                    result.remaining,
                    result.reset_at,
                );
                response
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock before epoch")
                    .as_secs();
                let retry_after = result.reset_at.saturating_sub(now);

                let mut response =
                    AppError::TooManyRequests("Rate limit exceeded".to_string())
                        .into_response();

                let headers = response.headers_mut();
                insert_rate_limit_headers(headers, limit, 0, result.reset_at);
                if let Ok(val) = HeaderValue::from_str(&retry_after.to_string()) {
                    headers.insert("retry-after", val);
                }

                response
            }
        }
        Err(err) => {
            // Fail-open: allow the request if Redis is down
            tracing::warn!(
                error = %err,
                "Rate limiter Redis error - failing open"
            );
            next.run(request).await
        }
    }
}

fn insert_rate_limit_headers(headers: &mut HeaderMap, limit: u64, remaining: u64, reset_at: u64) {
    if let Ok(val) = HeaderValue::from_str(&limit.to_string()) {
        headers.insert("x-ratelimit-limit", val);
    }
    if let Ok(val) = HeaderValue::from_str(&remaining.to_string()) {
        headers.insert("x-ratelimit-remaining", val);
    }
    if let Ok(val) = HeaderValue::from_str(&reset_at.to_string()) {
        headers.insert("x-ratelimit-reset", val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ConnectInfo;
    use axum::http::Request as HttpRequest;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn make_request() -> Request {
        HttpRequest::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[test]
    fn test_extract_key_jwt_claims() {
        let mut req = make_request();
        req.extensions_mut().insert(JwtClaims {
            sub: "user-123".to_string(),
            email: "test@example.com".to_string(),
            name: "Test".to_string(),
            roles: vec![],
            exp: 0,
            iat: 0,
            jti: "jti-1".to_string(),
        });
        assert_eq!(extract_key(&req), "user:user-123");
    }

    #[test]
    fn test_extract_key_x_real_ip() {
        let mut req = make_request();
        req.headers_mut()
            .insert("x-real-ip", HeaderValue::from_static("10.0.0.1"));
        assert_eq!(extract_key(&req), "ip:10.0.0.1");
    }

    #[test]
    fn test_extract_key_x_forwarded_for_rightmost() {
        let mut req = make_request();
        req.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("spoofed, 192.168.1.1"),
        );
        assert_eq!(extract_key(&req), "ip:192.168.1.1");
    }

    #[test]
    fn test_extract_key_connect_info() {
        let mut req = make_request();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 5)), 12345);
        req.extensions_mut().insert(ConnectInfo(addr));
        assert_eq!(extract_key(&req), "ip:172.16.0.5");
    }

    #[test]
    fn test_extract_key_fallback() {
        let req = make_request();
        assert_eq!(extract_key(&req), "ip:unknown");
    }
}
