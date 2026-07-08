import { Link } from '@tanstack/solid-router';
import { createSignal, createUniqueId, Show } from 'solid-js';
import { SocialLogin } from '../components/SocialLogin';
import { Button } from '../components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { useAuth } from '../lib/auth-context';

export function LoginPage() {
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [error, setError] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);

  const emailId = createUniqueId();
  const passwordId = createUniqueId();

  const auth = useAuth();

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError('');
    setIsLoading(true);

    try {
      await auth.login({
        email: email(),
        password: password(),
      });
      // Hard redirect on auth success: a full reload gives a clean app/query/router
      // state and avoids the SPA-navigate reactive storm at the auth boundary.
      window.location.href = '/tasks';
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div class="flex min-h-screen items-center justify-center p-4">
      <Card class="w-full max-w-md">
        <CardHeader>
          <CardTitle class="text-2xl text-center">Sign In</CardTitle>
          <CardDescription class="text-center">
            Enter your credentials to access your account
          </CardDescription>
        </CardHeader>

        <CardContent class="space-y-4">
          <Show when={error()}>
            <div class="rounded-md bg-red-50 p-3 border border-red-200">
              <p class="text-sm text-red-800">{error()}</p>
            </div>
          </Show>

          <form onSubmit={handleSubmit} class="space-y-4">
            <div class="space-y-2">
              <Label for={emailId}>Email</Label>
              <Input
                id={emailId}
                type="email"
                placeholder="your@email.com"
                value={email()}
                onInput={(e) => setEmail(e.currentTarget.value)}
                required
              />
            </div>

            <div class="space-y-2">
              <Label for={passwordId}>Password</Label>
              <Input
                id={passwordId}
                type="password"
                placeholder="Enter your password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
                required
              />
            </div>

            <Button type="submit" class="w-full" disabled={isLoading()}>
              {isLoading() ? 'Signing in...' : 'Sign In'}
            </Button>
          </form>

          <SocialLogin />
        </CardContent>

        <CardFooter class="flex justify-center">
          <p class="text-sm text-muted-foreground">
            Don't have an account?{' '}
            <Link href="/register" class="text-primary hover:underline">
              Sign up
            </Link>
          </p>
        </CardFooter>
      </Card>
    </div>
  );
}
