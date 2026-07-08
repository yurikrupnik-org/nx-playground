import { Link } from '@tanstack/solid-router';
import { createMemo, createSignal, createUniqueId, Show } from 'solid-js';
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

export function RegisterPage() {
  const [name, setName] = createSignal('');
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [confirmPassword, setConfirmPassword] = createSignal('');
  const [error, setError] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);

  const nameId = createUniqueId();
  const emailId = createUniqueId();
  const passwordId = createUniqueId();
  const confirmPasswordId = createUniqueId();

  const auth = useAuth();

  // Password validation
  const passwordRequirements = createMemo(() => ({
    length: password().length >= 8,
    uppercase: /[A-Z]/.test(password()),
    lowercase: /[a-z]/.test(password()),
    digit: /\d/.test(password()),
    special: /[!@#$%^&*()_+\-=[\]{}|;:,.<>?]/.test(password()),
  }));

  const isPasswordValid = createMemo(() => {
    const reqs = passwordRequirements();
    return (
      reqs.length &&
      reqs.uppercase &&
      reqs.lowercase &&
      reqs.digit &&
      reqs.special
    );
  });

  const passwordsMatch = createMemo(() => {
    return password() === confirmPassword() && password().length > 0;
  });

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError('');

    if (!isPasswordValid()) {
      setError('Password does not meet requirements');
      return;
    }

    if (!passwordsMatch()) {
      setError('Passwords do not match');
      return;
    }

    setIsLoading(true);

    try {
      await auth.register({
        name: name(),
        email: email(),
        password: password(),
      });
      // Hard redirect on auth success (see login.tsx): avoids the SPA-navigate storm.
      window.location.href = '/tasks';
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Registration failed');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div class="flex min-h-screen items-center justify-center p-4">
      <Card class="w-full max-w-md">
        <CardHeader>
          <CardTitle class="text-2xl text-center">Create Account</CardTitle>
          <CardDescription class="text-center">
            Sign up to get started with your account
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
              <Label for={nameId}>Name</Label>
              <Input
                id={nameId}
                type="text"
                placeholder="Your name"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                required
              />
            </div>

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
                placeholder="Create a password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
                required
              />
            </div>

            <Show when={password().length > 0}>
              <div class="rounded-md bg-muted p-3 space-y-2">
                <p class="text-sm font-semibold">Password requirements:</p>
                <ul class="space-y-1 text-sm">
                  <li
                    class={
                      passwordRequirements().length
                        ? 'text-green-600'
                        : 'text-muted-foreground'
                    }
                  >
                    <span class="mr-2">
                      {passwordRequirements().length ? '✓' : '○'}
                    </span>
                    At least 8 characters
                  </li>
                  <li
                    class={
                      passwordRequirements().uppercase
                        ? 'text-green-600'
                        : 'text-muted-foreground'
                    }
                  >
                    <span class="mr-2">
                      {passwordRequirements().uppercase ? '✓' : '○'}
                    </span>
                    One uppercase letter
                  </li>
                  <li
                    class={
                      passwordRequirements().lowercase
                        ? 'text-green-600'
                        : 'text-muted-foreground'
                    }
                  >
                    <span class="mr-2">
                      {passwordRequirements().lowercase ? '✓' : '○'}
                    </span>
                    One lowercase letter
                  </li>
                  <li
                    class={
                      passwordRequirements().digit
                        ? 'text-green-600'
                        : 'text-muted-foreground'
                    }
                  >
                    <span class="mr-2">
                      {passwordRequirements().digit ? '✓' : '○'}
                    </span>
                    One number
                  </li>
                  <li
                    class={
                      passwordRequirements().special
                        ? 'text-green-600'
                        : 'text-muted-foreground'
                    }
                  >
                    <span class="mr-2">
                      {passwordRequirements().special ? '✓' : '○'}
                    </span>
                    One special character
                  </li>
                </ul>
              </div>
            </Show>

            <div class="space-y-2">
              <Label for={confirmPasswordId}>Confirm Password</Label>
              <Input
                id={confirmPasswordId}
                type="password"
                placeholder="Confirm your password"
                value={confirmPassword()}
                onInput={(e) => setConfirmPassword(e.currentTarget.value)}
                required
              />
              <Show when={confirmPassword().length > 0 && !passwordsMatch()}>
                <p class="text-sm text-red-600">Passwords do not match</p>
              </Show>
            </div>

            <Button
              type="submit"
              class="w-full"
              disabled={!isPasswordValid() || !passwordsMatch() || isLoading()}
            >
              {isLoading() ? 'Creating account...' : 'Create Account'}
            </Button>
          </form>
        </CardContent>

        <CardFooter class="flex justify-center">
          <p class="text-sm text-muted-foreground">
            Already have an account?{' '}
            <Link href="/login" class="text-primary hover:underline">
              Sign in
            </Link>
          </p>
        </CardFooter>
      </Card>
    </div>
  );
}
