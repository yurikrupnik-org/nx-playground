import { afterEach, describe, expect, it, vi } from 'vitest';
import * as authApi from './auth-api';

describe('authApi.logout', () => {
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
    await expect(authApi.logout()).resolves.toBe(logoutUrl);
  });

  it('throws when the server rejects the logout', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 403 })),
    );

    await expect(authApi.logout()).rejects.toThrow('logout failed (403)');
  });
});
