import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import type * as Identity from './identity';

/** Minimal `Storage` stand-in so each test starts from a known slate. */
function memoryStorage(seed: Record<string, string> = {}) {
  const entries = new Map(Object.entries(seed));
  return {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => {
      entries.set(key, value);
    },
    removeItem: (key: string) => {
      entries.delete(key);
    },
    clear: () => entries.clear(),
    key: () => null,
    get length() {
      return entries.size;
    },
    entries,
  };
}

/**
 * `identity.ts` caches the identity in module scope (the in-memory fallback),
 * so a static import would carry one test's identity into the next: a test
 * that seeds an invalid `localStorage` value would read the *previous* test's
 * identity instead of generating one.
 *
 * Hence the dynamic import — `vi.resetModules()` only affects subsequent
 * imports, so a fresh instance per test is unreachable with a static one.
 */
let identity: typeof Identity;

beforeEach(async () => {
  vi.resetModules();
  vi.stubGlobal('localStorage', memoryStorage());
  identity = await import('./identity');
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('isValidIdentity', () => {
  test('accepts the documented grammar', () => {
    expect(identity.isValidIdentity('yuri')).toBe(true);
    expect(identity.isValidIdentity('anon-1a2b3c4d')).toBe(true);
    expect(identity.isValidIdentity('user.name_1@example-corp')).toBe(true);
    expect(identity.isValidIdentity('a')).toBe(true);
    expect(identity.isValidIdentity('x'.repeat(64))).toBe(true);
  });

  test('rejects empty, over-long and out-of-alphabet values', () => {
    expect(identity.isValidIdentity('')).toBe(false);
    expect(identity.isValidIdentity('x'.repeat(65))).toBe(false);
    expect(identity.isValidIdentity('has space')).toBe(false);
    expect(identity.isValidIdentity('slash/es')).toBe(false);
    expect(identity.isValidIdentity('semi;colon')).toBe(false);
  });
});

describe('getIdentity', () => {
  test('generates and persists anon-<8 hex> on first use', () => {
    const value = identity.getIdentity();
    expect(value).toMatch(/^anon-[0-9a-f]{8}$/);
    expect(localStorage.getItem(identity.IDENTITY_STORAGE_KEY)).toBe(value);
  });

  test('prefers a stored identity', () => {
    vi.stubGlobal('localStorage', memoryStorage({ todo_identity: 'yuri' }));
    expect(identity.getIdentity()).toBe('yuri');
  });

  test('regenerates when the stored value is not a valid identity', () => {
    vi.stubGlobal('localStorage', memoryStorage({ todo_identity: 'not ok!' }));
    expect(identity.getIdentity()).toMatch(/^anon-[0-9a-f]{8}$/);
  });

  test('keeps the identity it already handed out when storage turns invalid', () => {
    const first = identity.getIdentity();
    vi.stubGlobal('localStorage', memoryStorage({ todo_identity: 'not ok!' }));
    expect(identity.getIdentity()).toBe(first);
  });

  test('falls back to memory when localStorage throws', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('access denied');
      },
      setItem: () => {
        throw new Error('access denied');
      },
    });

    const first = identity.getIdentity();
    expect(first).toMatch(/^anon-[0-9a-f]{8}$/);
    expect(identity.getIdentity()).toBe(first);
  });
});

describe('setIdentity', () => {
  test('trims, persists and returns the stored value', () => {
    expect(identity.setIdentity('  yuri  ')).toBe('yuri');
    expect(localStorage.getItem(identity.IDENTITY_STORAGE_KEY)).toBe('yuri');
    expect(identity.getIdentity()).toBe('yuri');
  });

  test('returns null and writes nothing for an invalid identity', () => {
    identity.setIdentity('yuri');
    expect(identity.setIdentity('no spaces allowed')).toBeNull();
    expect(localStorage.getItem(identity.IDENTITY_STORAGE_KEY)).toBe('yuri');
  });
});

describe('identityHeaders', () => {
  test('carries the identity and the app name', () => {
    identity.setIdentity('yuri');
    expect(identity.identityHeaders()).toEqual({
      'X-Todo-Identity': 'yuri',
      'X-Todo-App': 'web',
    });
  });
});
