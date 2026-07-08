// Double-submit CSRF token. The API issues a readable `csrf_token` cookie alongside the
// session; the SPA echoes it in the `x-csrf-token` header on state-changing requests to
// the business routers (same-origin only — the cookie is intentionally not HttpOnly).

const CSRF_COOKIE = 'csrf_token';
const CSRF_HEADER = 'x-csrf-token';

/** Read the CSRF token from the readable cookie, or `null` if unauthenticated. */
export function csrfToken(): string | null {
  const prefix = `${CSRF_COOKIE}=`;
  const match = document.cookie.split('; ').find((c) => c.startsWith(prefix));
  return match ? decodeURIComponent(match.slice(prefix.length)) : null;
}

/** Header object to spread into a mutating `fetch`; empty when no token is present. */
export function csrfHeaders(): Record<string, string> {
  const token = csrfToken();
  return token ? { [CSRF_HEADER]: token } : {};
}
