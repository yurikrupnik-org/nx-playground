//! Cookie helpers shared by the BFF auth flow and the session middleware, so the
//! attributes (`HttpOnly`, `SameSite=Lax`, `Path=/`, `Secure`) are defined once.

use axum::http::HeaderMap;
use axum::http::header;

/// Build a `Set-Cookie` value with the auth defaults: `HttpOnly`, `SameSite=Lax`,
/// `Path=/`. `Secure` is added when `secure` is set and is **forced** for `__Host-` /
/// `__Secure-` prefixed names, which browsers reject without it.
pub fn build(name: &str, value: &str, max_age: u64, secure: bool) -> String {
    let mut c = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure || has_secure_prefix(name) {
        c.push_str("; Secure");
    }
    c
}

/// Build a `Set-Cookie` that immediately clears `name` (same attributes as [`build`]).
pub fn clear(name: &str, secure: bool) -> String {
    let mut c = format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if secure || has_secure_prefix(name) {
        c.push_str("; Secure");
    }
    c
}

/// Read a cookie value by name from a request's `Cookie` header.
pub fn parse<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// `__Host-` and `__Secure-` cookie prefixes mandate the `Secure` attribute.
fn has_secure_prefix(name: &str) -> bool {
    name.starts_with("__Host-") || name.starts_with("__Secure-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sets_baseline_attributes() {
        let c = build("terran_session", "abc", 60, false);
        assert!(c.contains("terran_session=abc"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("Max-Age=60"));
        assert!(!c.contains("Secure"), "no Secure when not requested");
    }

    #[test]
    fn build_forces_secure_for_host_prefix() {
        // `__Host-` cookies are invalid without Secure; build must add it regardless.
        let c = build("__Host-terran", "abc", 60, false);
        assert!(c.contains("; Secure"), "Secure forced for __Host- prefix");
    }

    #[test]
    fn parse_reads_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "a=1; terran_session=xyz; b=2".parse().unwrap(),
        );
        assert_eq!(parse(&headers, "terran_session"), Some("xyz"));
        assert_eq!(parse(&headers, "missing"), None);
    }
}
