import { PasswordLogin } from '../components/password-login';
import { SocialLogin } from '../components/social-login';

export function LoginPage() {
  return (
    <div class="flex min-h-screen items-center justify-center bg-gray-50 px-4">
      <div class="w-full max-w-sm rounded-lg border border-gray-200 bg-white p-8 shadow-sm">
        <h1 class="text-center text-2xl font-semibold">terran</h1>
        <p class="mt-1 mb-6 text-center text-sm text-gray-500">
          Sign in to your enterprise observability platform
        </p>
        <PasswordLogin />
        <div class="my-6 flex items-center gap-3 text-xs text-gray-400">
          <span class="h-px flex-1 bg-gray-200" />
          OR
          <span class="h-px flex-1 bg-gray-200" />
        </div>
        <SocialLogin />
      </div>
    </div>
  );
}
