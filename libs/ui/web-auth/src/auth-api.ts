// BFF auth client. The browser never holds tokens — it talks only to its own API,
// which owns the IdP exchange (WorkOS for zerg, Keycloak for terran) and the
// server-side session cookie.

import { csrfHeaders } from './csrf';

const API_BASE_URL = '/api';

export interface AuthApiOptions {
  /** Name of the readable double-submit CSRF cookie this API issues. */
  csrfCookieName: string;
}

/**
 * The BFF auth surface, bound to one app's CSRF cookie.
 *
 * `TUser` is the app's `/auth/me` shape (they differ: zerg returns a user with an
 * org context, terran returns a principal plus tenant) and `TIdp` its accepted
 * `idp` hints.
 */
export interface AuthApi<TUser, TIdp extends string = string> {
  getCurrentUser(): Promise<TUser>;
  login(idp?: TIdp): void;
  passwordLogin(email: string, password: string): Promise<void>;
  logout(): Promise<string>;
}

export function createAuthApi<TUser, TIdp extends string = string>(
  options: AuthApiOptions,
): AuthApi<TUser, TIdp> {
  return {
    /** Current principal; throws (→ unauthenticated) on non-2xx. */
    async getCurrentUser(): Promise<TUser> {
      const res = await fetch(`${API_BASE_URL}/auth/me`, {
        credentials: 'include',
      });
      if (!res.ok) {
        throw new Error(`not authenticated (${res.status})`);
      }
      return res.json();
    },

    /** Begin login by redirecting to the API, which 302s to the IdP.
     *  `idp` deep-links a brokered social provider. */
    login(idp?: TIdp): void {
      const query = idp ? `?idp=${encodeURIComponent(idp)}` : '';
      window.location.href = `${API_BASE_URL}/auth/login${query}`;
    },

    /** Native email+password sign-in via the BFF. On a 2xx the session cookie is
     *  set; the caller should then refetch `/me`. Throws a user-facing message on
     *  bad credentials (401) or validation (400). */
    async passwordLogin(email: string, password: string): Promise<void> {
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
    },

    /** Revoke the server session; returns the IdP end-session URL to navigate to.
     *  RP-initiated logout is a top-level browser navigation (a `fetch()` can't
     *  follow the cross-origin redirect to the IdP), so the caller assigns
     *  `window.location` to the returned URL rather than letting fetch chase it. */
    async logout(): Promise<string> {
      const res = await fetch(`${API_BASE_URL}/auth/logout`, {
        method: 'POST',
        credentials: 'include',
        headers: { ...csrfHeaders(options.csrfCookieName) },
      });
      if (!res.ok) {
        throw new Error(`logout failed (${res.status})`);
      }
      const { logout_url } = (await res.json()) as { logout_url: string };
      return logout_url;
    },
  };
}
