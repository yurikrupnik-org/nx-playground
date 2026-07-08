import { createEffect, type ParentComponent, Show } from 'solid-js';
import { useAuth } from '../lib/auth-context';

export const ProtectedRoute: ParentComponent = (props) => {
  const auth = useAuth();

  createEffect(() => {
    // Redirect unauthenticated users to login. Use a hard redirect (not the SPA
    // router): flipping to a redirect via `navigate` while this route is still mounted
    // (logout, or an access-token expiry that 401s `/me`) storms the main thread and
    // freezes the tab. A full reload gives clean state and lands on /login once.
    if (!auth.isLoading() && !auth.isAuthenticated()) {
      window.location.href = '/login';
    }
  });

  return (
    <Show
      when={!auth.isLoading()}
      fallback={
        <div class="flex items-center justify-center min-h-screen">
          <div class="text-center">
            <div class="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-gray-900"></div>
            <p class="mt-4 text-gray-600">Loading...</p>
          </div>
        </div>
      }
    >
      <Show when={auth.isAuthenticated()} fallback={null}>
        {props.children}
      </Show>
    </Show>
  );
};
