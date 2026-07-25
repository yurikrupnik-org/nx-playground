// BFF auth client. The browser never holds tokens — it talks only to the terran
// API, which owns the Keycloak exchange and the server-side session cookie.

import { csrfHeaders } from './csrf';

const API_BASE_URL = '/api';

export type IdpHint = 'google' | 'github';

export interface Me {
  subject: string;
  email?: string | null;
  name?: string | null;
  roles: string[];
  org_id: string;
  role: string;
}

/** Current principal + resolved tenant; throws (→ unauthenticated) on non-2xx. */
export async function getCurrentUser(): Promise<Me> {
  const res = await fetch(`${API_BASE_URL}/auth/me`, {
    credentials: 'include',
  });
  if (!res.ok) {
    throw new Error(`not authenticated (${res.status})`);
  }
  return res.json();
}

/** Begin login by redirecting to the API, which 302s to Keycloak.
 *  `idp` deep-links a brokered social provider via `kc_idp_hint`. */
export function login(idp?: IdpHint): void {
  const query = idp ? `?idp=${encodeURIComponent(idp)}` : '';
  window.location.href = `${API_BASE_URL}/auth/login${query}`;
}

/** Native email+password sign-in via the BFF (Keycloak Direct Access Grant). On a
 *  2xx the session cookie is set; the caller should then refetch `/me`. Throws a
 *  user-facing message on bad credentials (401) or validation (400). */
export async function passwordLogin(
  email: string,
  password: string,
): Promise<void> {
  const res = await fetch(`${API_BASE_URL}/auth/login/password`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });
  if (!res.ok) {
    throw new Error(
      res.status === 401
        ? 'Invalid email or password'
        : `Sign-in failed (${res.status})`,
    );
  }
}

/** Revoke the server session; returns the IdP end-session URL to navigate to.
 *  RP-initiated logout is a top-level browser navigation (a `fetch()` can't follow
 *  the cross-origin redirect to Keycloak), so the caller assigns `window.location`
 *  to the returned URL rather than letting fetch chase it. */
export async function logout(): Promise<string> {
  const res = await fetch(`${API_BASE_URL}/auth/logout`, {
    method: 'POST',
    credentials: 'include',
    headers: { ...csrfHeaders() },
  });
  if (!res.ok) {
    throw new Error(`logout failed (${res.status})`);
  }
  const { logout_url } = (await res.json()) as { logout_url: string };
  return logout_url;
}
