// Shared BFF auth client for the Solid SPAs (zerg, terran). Both talk to their own
// API over first-party cookies; only the CSRF cookie name and the `/auth/me` shape
// differ, so those are parameters.

export {
  type AuthApi,
  type AuthApiOptions,
  createAuthApi,
} from './auth-api';
export { type AuthContextValue, createAuthProvider } from './auth-context';
export { csrfHeaders, csrfToken } from './csrf';
export {
  createProtectedRoute,
  type ProtectedRouteOptions,
} from './protected-route';
