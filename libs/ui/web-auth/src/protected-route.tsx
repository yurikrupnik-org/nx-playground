import { Navigate } from '@tanstack/solid-router';
import { type JSX, type ParentComponent, Show } from 'solid-js';

/** Minimal slice of the auth context this component needs. */
interface AuthGate {
  isLoading: () => boolean;
  isAuthenticated: () => boolean;
}

export interface ProtectedRouteOptions {
  /** Where to send unauthenticated visitors. Defaults to `/login`. */
  redirectTo?: string;
  /** Shown while `/auth/me` is in flight. */
  fallback?: JSX.Element;
}

/**
 * Builds a route guard that renders children only for an authenticated session.
 *
 * The redirect is declarative (`<Navigate>`), never an imperative `navigate()` or
 * `window.location` assignment inside a `createEffect`: those loop with the router
 * when auth flips to unauthenticated while the route is mounted (logout, session
 * expiry), freezing the page.
 */
export function createProtectedRoute(
  useAuth: () => AuthGate,
  options: ProtectedRouteOptions = {},
): ParentComponent {
  const redirectTo = options.redirectTo ?? '/login';

  return (props) => {
    const auth = useAuth();

    return (
      <Show
        when={!auth.isLoading()}
        fallback={
          options.fallback ?? (
            <div class="flex min-h-screen items-center justify-center">
              <div class="h-8 w-8 animate-spin rounded-full border-b-2 border-gray-900" />
            </div>
          )
        }
      >
        <Show
          when={auth.isAuthenticated()}
          fallback={<Navigate to={redirectTo} />}
        >
          {props.children}
        </Show>
      </Show>
    );
  };
}
