// BFF auth client. The browser never holds tokens — it talks only to the zerg
// API, which owns the WorkOS exchange and the server-side session cookie.

import { csrfHeaders } from './csrf';

const API_BASE_URL = '/api';

/** `google`/`github` deep-link the social provider; `sign-up` opens AuthKit's
 *  sign-up screen (hosted AuthKit also owns password reset). */
export type IdpHint = 'google' | 'github' | 'sign-up';

export interface OrgContext {
  id: string;
  external_id: string;
  name: string;
  role: string;
  is_personal: boolean;
}

export interface UserResponse {
  id: string;
  email: string;
  name: string;
  roles: string[];
  email_verified: boolean;
  created_at: string;
  updated_at: string;
  avatar_url?: string | null;
  last_login_at?: string | null;
  /** Active organization context (personal workspace for B2C users). */
  org: OrgContext;
}

/** Current user; throws (→ unauthenticated) on non-2xx. */
export async function getCurrentUser(): Promise<UserResponse> {
  const res = await fetch(`${API_BASE_URL}/auth/me`, {
    credentials: 'include',
  });
  if (!res.ok) {
    throw new Error(`not authenticated (${res.status})`);
  }
  return res.json();
}

/** Begin login by redirecting to the API, which 302s to WorkOS AuthKit. */
export function login(idp?: IdpHint): void {
  const query = idp ? `?idp=${encodeURIComponent(idp)}` : '';
  window.location.href = `${API_BASE_URL}/auth/login${query}`;
}

/** Native email+password sign-in via the BFF (WorkOS password grant). On a 2xx
 *  the session cookie is set; the caller should then refetch `/me`. Throws a
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
 *  the cross-origin redirect to WorkOS), so the caller assigns `window.location`
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
