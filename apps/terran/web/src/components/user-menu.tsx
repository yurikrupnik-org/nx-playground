import { Show } from 'solid-js';
import { useAuth } from '../lib/auth-context';

export function UserMenu() {
  const auth = useAuth();
  return (
    <Show
      when={auth.isAuthenticated()}
      fallback={
        <a
          href="/login"
          class="text-sm font-medium text-gray-700 hover:text-gray-900"
        >
          Sign in
        </a>
      }
    >
      <div class="flex items-center gap-3">
        <span class="text-sm text-gray-600">
          {auth.user()?.email ?? auth.user()?.subject}
        </span>
        <button
          type="button"
          class="rounded-md border border-gray-300 px-3 py-1 text-sm hover:bg-gray-50"
          onClick={() => void auth.logout()}
        >
          Sign out
        </button>
      </div>
    </Show>
  );
}
