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

import { AuthProvider } from '../lib/auth-context';
import { ProtectedRoute } from './protected-route';

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
    const me = {
      subject: 'u-1',
      email: 'a@b.c',
      name: 'Alice',
      roles: ['member'],
      org_id: 'org-1',
      role: 'member',
    };
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
});
