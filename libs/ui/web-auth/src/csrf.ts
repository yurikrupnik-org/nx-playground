// Double-submit CSRF token. Each API issues a readable CSRF cookie alongside the
// session; the SPA echoes it in the `x-csrf-token` header on state-changing
// requests (same-origin only — the cookie is intentionally not HttpOnly).
//
// The cookie name is per-app (`csrf_token` for zerg, `terran_csrf` for terran),
// so it is a parameter rather than a constant.

const CSRF_HEADER = 'x-csrf-token';

/** Read the CSRF token from the readable cookie, or `null` if unauthenticated. */
export function csrfToken(cookieName: string): string | null {
  const prefix = `${cookieName}=`;
  const match = document.cookie.split('; ').find((c) => c.startsWith(prefix));
  return match ? decodeURIComponent(match.slice(prefix.length)) : null;
}

/** Header object to spread into a mutating `fetch`; empty when no token is present. */
export function csrfHeaders(cookieName: string): Record<string, string> {
  const token = csrfToken(cookieName);
  return token ? { [CSRF_HEADER]: token } : {};
}
