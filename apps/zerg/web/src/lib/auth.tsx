// zerg's binding of the shared BFF auth client (@ui/web-auth): the readable CSRF
// cookie is `csrf_token` and `/auth/me` returns the user plus their active
// organization context.
//
// `sign-up` opens AuthKit's hosted sign-up screen (which also owns password reset).

import {
  createAuthApi,
  createAuthProvider,
  createProtectedRoute,
  csrfHeaders as csrfHeadersFor,
} from '@ui/web-auth';

/** Readable double-submit CSRF cookie issued by zerg-api. */
const CSRF_COOKIE = 'csrf_token';

/** CSRF header bundle for zerg's mutating fetches. */
export const csrfHeaders = (): Record<string, string> =>
  csrfHeadersFor(CSRF_COOKIE);

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

export const authApi = createAuthApi<UserResponse, IdpHint>({
  csrfCookieName: CSRF_COOKIE,
});

export const { AuthProvider, useAuth } = createAuthProvider(authApi);

/// Route guard: renders children only for an authenticated session, otherwise
/// redirects to /login declaratively (never an imperative window.location — that
/// storms the router when auth flips while the route is mounted).
export const ProtectedRoute = createProtectedRoute(useAuth, {
  fallback: (
    <div class="flex items-center justify-center min-h-screen">
      <div class="text-center">
        <div class="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-gray-900" />
        <p class="mt-4 text-gray-600">Loading...</p>
      </div>
    </div>
  ),
});
