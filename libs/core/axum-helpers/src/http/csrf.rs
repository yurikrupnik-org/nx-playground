//! CSRF protection for cookie-authenticated, state-changing requests
//! (stateless double-submit token).
//!
//! - **Safe methods** (GET/HEAD/OPTIONS/TRACE) are exempt.
//! - **Bearer/machine** requests are exempt — they carry no ambient cookie and a
//!   cross-site page cannot set the `Authorization` header (it triggers a blocked
//!   CORS preflight), so they cannot be forged.
//! - **Cookie-authed mutations** must echo the readable CSRF cookie in a request
//!   header. A forged cross-site request cannot read the victim's cookie
//!   (same-origin policy), so it cannot produce a matching header.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

/// Default header the SPA echoes the CSRF token in.
pub const DEFAULT_HEADER: &str = "x-csrf-token";

/// Configuration for [`csrf_protect`].
#[derive(Clone)]
pub struct CsrfConfig {
    /// Name of the readable double-submit cookie (e.g. `zerg_csrf`).
    pub cookie_name: Arc<str>,
    /// Header the token must be echoed in (compared case-insensitively by the map).
    pub header: Arc<str>,
}

impl CsrfConfig {
    /// Config with the default `x-csrf-token` header.
    pub fn new(cookie_name: impl Into<Arc<str>>) -> Self {
        Self {
            cookie_name: cookie_name.into(),
            header: Arc::from(DEFAULT_HEADER),
        }
    }
}

/// Generate a fresh CSRF token (256-bit, hex) for the double-submit cookie.
pub fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Build the **readable** (non-`HttpOnly`) double-submit cookie: `SameSite=Lax`,
/// `Path=/`; `Secure` follows `secure` or a `__Host-`/`__Secure-` prefix. Unlike a
/// session cookie it intentionally omits `HttpOnly` so the SPA can echo the token.
pub fn build_cookie(name: &str, value: &str, max_age: u64, secure: bool) -> String {
    let secure = secure || name.starts_with("__Host-") || name.starts_with("__Secure-");
    let mut c = format!("{name}={value}; Path=/; SameSite=Lax; Max-Age={max_age}");
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Middleware enforcing the double-submit token on cookie-authed, state-changing requests.
pub async fn csrf_protect(State(cfg): State<CsrfConfig>, req: Request, next: Next) -> Response {
    let safe = is_safe(req.method());
    let bearer = req.headers().contains_key(header::AUTHORIZATION);
    let ok = {
        let cookie_tok = parse_cookie(req.headers(), &cfg.cookie_name);
        let header_tok = req
            .headers()
            .get(&*cfg.header)
            .and_then(|v| v.to_str().ok());
        allowed(safe, bearer, cookie_tok, header_tok)
    };
    if ok {
        next.run(req).await
    } else {
        tracing::debug!("csrf token missing or invalid on state-changing request");
        (StatusCode::FORBIDDEN, "csrf token missing or invalid").into_response()
    }
}

fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Read a cookie value by name from a request's `Cookie` header.
fn parse_cookie<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// Pure decision: allow safe/bearer requests; otherwise require a non-empty token that
/// matches between cookie and header.
fn allowed(safe: bool, bearer: bool, cookie_tok: Option<&str>, header_tok: Option<&str>) -> bool {
    if safe || bearer {
        return true;
    }
    matches!((cookie_tok, header_tok), (Some(c), Some(h)) if !c.is_empty() && c == h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_methods_and_bearer_bypass() {
        assert!(
            allowed(true, false, None, None),
            "safe method allowed without token"
        );
        assert!(
            allowed(false, true, None, None),
            "bearer request allowed without token"
        );
    }

    #[test]
    fn mismatch_or_missing_is_rejected() {
        assert!(
            !allowed(false, false, Some("abc"), Some("xyz")),
            "mismatch rejected"
        );
        assert!(
            !allowed(false, false, Some("abc"), None),
            "missing header rejected"
        );
        assert!(
            !allowed(false, false, None, Some("abc")),
            "missing cookie rejected"
        );
        assert!(
            !allowed(false, false, Some(""), Some("")),
            "empty token rejected"
        );
    }

    #[test]
    fn matching_double_submit_is_allowed() {
        assert!(allowed(false, false, Some("tok"), Some("tok")), "match ok");
    }

    #[test]
    fn readable_cookie_has_no_httponly() {
        let c = build_cookie("zerg_csrf", "tok", 60, false);
        assert!(
            !c.contains("HttpOnly"),
            "csrf cookie must be readable by the SPA"
        );
        assert!(c.contains("SameSite=Lax"));
    }
}
