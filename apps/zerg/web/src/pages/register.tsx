import { Link } from '@tanstack/solid-router';
import { login } from '../lib/auth-api';
import { Button } from '../components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '../components/ui/card';

/** Account creation lives on WorkOS's hosted AuthKit (which also owns password
 *  reset and email verification); this page just launches its sign-up screen. */
export function RegisterPage() {
  return (
    <div class="flex min-h-screen items-center justify-center p-4">
      <Card class="w-full max-w-md">
        <CardHeader>
          <CardTitle class="text-2xl text-center">Create Account</CardTitle>
          <CardDescription class="text-center">
            Sign-up is handled on our secure hosted login page
          </CardDescription>
        </CardHeader>

        <CardContent>
          <Button type="button" class="w-full" onClick={() => login('sign-up')}>
            Continue to Sign Up
          </Button>
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
