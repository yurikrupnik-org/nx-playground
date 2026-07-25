import { useNavigate } from '@tanstack/solid-router';
import { createSignal, Show } from 'solid-js';
import { useAuth } from '../lib/auth-context';

const FIELD =
  'w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-gray-900 focus:outline-none';

/// Native email+password sign-in. Posts to the BFF (which performs the Keycloak
/// Direct Access Grant and sets the session cookie); on success we land in the app.
/// Sign-up and password reset are handled by Keycloak's hosted pages (reachable via
/// the "Create account" / "Forgot password?" links, which start the redirect flow).
export function PasswordLogin() {
  const auth = useAuth();
  const navigate = useNavigate();
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  const onSubmit = async (e: SubmitEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      await auth.passwordLogin(email(), password());
      navigate({ to: '/assets' });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Sign-in failed');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form class="flex flex-col gap-3" onSubmit={onSubmit}>
      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium text-gray-700">Email</span>
        <input
          type="email"
          name="email"
          autocomplete="username"
          required
          class={FIELD}
          value={email()}
          onInput={(e) => setEmail(e.currentTarget.value)}
        />
      </label>
      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium text-gray-700">Password</span>
        <input
          type="password"
          name="password"
          autocomplete="current-password"
          required
          class={FIELD}
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
        />
      </label>

      <Show when={error()}>
        <p class="text-sm text-red-600" role="alert">
          {error()}
        </p>
      </Show>

      <button
        type="submit"
        disabled={submitting()}
        class="w-full rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-60"
      >
        {submitting() ? 'Signing in…' : 'Sign in'}
      </button>

      <div class="flex justify-between text-xs text-gray-500">
        <button
          type="button"
          class="hover:underline"
          onClick={() => auth.login()}
        >
          Forgot password?
        </button>
        <button
          type="button"
          class="hover:underline"
          onClick={() => auth.login()}
        >
          Create account
        </button>
      </div>
    </form>
  );
}
