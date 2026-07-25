import { useAuth } from '../lib/auth-context';

const BTN =
  'w-full rounded-md border border-gray-300 px-4 py-2 text-sm font-medium hover:bg-gray-50';

/// Redirect launchers. Google/GitHub deep-link through Keycloak via `kc_idp_hint`;
/// the plain option lands on Keycloak's hosted page (enterprise SSO, plus self-serve
/// account creation and password reset). Native email/password is the separate form.
export function SocialLogin() {
  const auth = useAuth();
  return (
    <div class="flex flex-col gap-3">
      <button type="button" class={BTN} onClick={() => auth.login('google')}>
        Continue with Google
      </button>
      <button type="button" class={BTN} onClick={() => auth.login('github')}>
        Continue with GitHub
      </button>
      <button
        type="button"
        class="w-full rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800"
        onClick={() => auth.login()}
      >
        Continue with single sign-on
      </button>
    </div>
  );
}
