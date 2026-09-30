import { afterEach, describe, expect, it, vi } from 'vitest';
import { createAuthApi } from './auth-api';

interface Me {
  subject: string;
}

const api = createAuthApi<Me>({ csrfCookieName: 'test_csrf' });

describe('createAuthApi logout', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('returns the IdP end-session URL from the JSON body', async () => {
    const logoutUrl =
      'http://localhost:8088/realms/terran/protocol/openid-connect/logout?client_id=terran-api';
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify({ logout_url: logoutUrl }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
      ),
    );

    // The SPA navigates top-level to this URL; if logout() resolved to `void`
    // (the old redirect-following contract) the redirect would never happen.
    await expect(api.logout()).resolves.toBe(logoutUrl);
  });

  it('throws when the server rejects the logout', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 403 })),
    );

    await expect(api.logout()).rejects.toThrow('logout failed (403)');
  });

  it('sends the app CSRF token from its own cookie', async () => {
    const fetchSpy = vi.fn(
      async (_url: string, _init?: RequestInit) =>
        new Response(JSON.stringify({ logout_url: '/done' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    );
    vi.stubGlobal('fetch', fetchSpy);
    // Stub the getter rather than assigning document.cookie: no cookie leaks into
    // the other tests in this file.
    vi.spyOn(document, 'cookie', 'get').mockReturnValue('test_csrf=tok-123');

    await api.logout();

    const headers = fetchSpy.mock.calls[0][1]?.headers as Record<
      string,
      string
    >;
    expect(headers['x-csrf-token']).toBe('tok-123');

    vi.restoreAllMocks();
  });
});

describe('createAuthApi getCurrentUser', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('throws on a 401 so the query resolves to unauthenticated', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 401 })),
    );

    await expect(api.getCurrentUser()).rejects.toThrow(
      'not authenticated (401)',
    );
  });
});

describe('createAuthApi passwordLogin', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('maps a 401 to a user-facing credentials message', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 401 })),
    );

    await expect(api.passwordLogin('a@b.c', 'nope')).rejects.toThrow(
      'Invalid email or password',
    );
  });

  it('reports other failures with their status', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 400 })),
    );

    await expect(api.passwordLogin('a@b.c', 'x')).rejects.toThrow(
      'Sign-in failed (400)',
    );
  });
});
