// terran's binding of the shared BFF auth client (@ui/web-auth): the readable
// CSRF cookie is `terran_csrf` and `/auth/me` returns a Keycloak principal plus
// the resolved tenant.

import {
  createAuthApi,
  createAuthProvider,
  createProtectedRoute,
  csrfHeaders as csrfHeadersFor,
} from '@ui/web-auth';

/** Readable double-submit CSRF cookie issued by terran-api. */
const CSRF_COOKIE = 'terran_csrf';

/** CSRF header bundle for terran's mutating fetches. */
export const csrfHeaders = (): Record<string, string> =>
  csrfHeadersFor(CSRF_COOKIE);

export type IdpHint = 'google' | 'github';

export interface Me {
  subject: string;
  email?: string | null;
  name?: string | null;
  roles: string[];
  org_id: string;
  role: string;
}

export const authApi = createAuthApi<Me, IdpHint>({
  csrfCookieName: CSRF_COOKIE,
});

export const { AuthProvider, useAuth } = createAuthProvider(authApi);

/// Route guard: renders children only for an authenticated session, otherwise
/// redirects to /login declaratively (never an imperative navigate — that loops
/// with the router when auth flips while the route is mounted).
export const ProtectedRoute = createProtectedRoute(useAuth);
