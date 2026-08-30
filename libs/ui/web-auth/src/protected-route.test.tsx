import { render, waitFor } from '@solidjs/testing-library';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import type { ParentComponent } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Capture the declarative redirect target without a real router.
const { navigateSpy } = vi.hoisted(() => ({ navigateSpy: vi.fn() }));
vi.mock('@tanstack/solid-router', () => ({
  Navigate: (props: { to: string }) => {
    navigateSpy(props.to);
    return null;
  },
}));

import { createAuthApi } from './auth-api';
import { createAuthProvider } from './auth-context';
import { createProtectedRoute } from './protected-route';

interface Me {
  subject: string;
  email?: string | null;
}

const api = createAuthApi<Me>({ csrfCookieName: 'test_csrf' });
const { AuthProvider, useAuth } = createAuthProvider(api);
const ProtectedRoute = createProtectedRoute(useAuth);

const harness: ParentComponent = (props) => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <ProtectedRoute>{props.children}</ProtectedRoute>
      </AuthProvider>
    </QueryClientProvider>
  );
};

describe('ProtectedRoute', () => {
  beforeEach(() => navigateSpy.mockClear());
  afterEach(() => vi.unstubAllGlobals());

  it('redirects to /login when /api/auth/me is unauthorized', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 401 })),
    );
    const { queryByText } = render(() =>
      harness({ children: <div>SECRET</div> }),
    );

    await waitFor(() => expect(navigateSpy).toHaveBeenCalledWith('/login'));
    expect(queryByText('SECRET')).toBeNull();
  });

  it('renders children for an authenticated session', async () => {
    const me = { subject: 'u-1', email: 'a@b.c' };
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify(me), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
      ),
    );
    const { findByText } = render(() =>
      harness({ children: <div>SECRET</div> }),
    );

    expect(await findByText('SECRET')).toBeTruthy();
    expect(navigateSpy).not.toHaveBeenCalled();
  });

  it('honours a custom redirect target', async () => {
    const CustomGuard = createProtectedRoute(useAuth, {
      redirectTo: '/sign-in',
    });
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 401 })),
    );
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });

    render(() => (
      <QueryClientProvider client={queryClient}>
        <AuthProvider>
          <CustomGuard>
            <div>SECRET</div>
          </CustomGuard>
        </AuthProvider>
      </QueryClientProvider>
    ));

    await waitFor(() => expect(navigateSpy).toHaveBeenCalledWith('/sign-in'));
  });
});
