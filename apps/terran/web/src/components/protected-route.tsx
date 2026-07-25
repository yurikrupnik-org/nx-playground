import { Navigate } from '@tanstack/solid-router';
import { type ParentComponent, Show } from 'solid-js';
import { useAuth } from '../lib/auth-context';

/// Renders children only for an authenticated session; otherwise redirects to /login.
/// The redirect is declarative (<Navigate>), not an imperative navigate() inside a
/// createEffect: the latter loops with the router when auth flips to unauthenticated
/// while this route is mounted (e.g. on logout / session expiry), freezing the page.
export const ProtectedRoute: ParentComponent = (props) => {
  const auth = useAuth();

  return (
    <Show
      when={!auth.isLoading()}
      fallback={
        <div class="flex min-h-screen items-center justify-center">
          <div class="h-8 w-8 animate-spin rounded-full border-b-2 border-gray-900" />
        </div>
      }
    >
      <Show when={auth.isAuthenticated()} fallback={<Navigate to="/login" />}>
        {props.children}
      </Show>
    </Show>
  );
};
