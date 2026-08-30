import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import {
  getIdentity,
  IDENTITY_STORAGE_KEY,
  identityHeaders,
  isValidIdentity,
  setIdentity,
} from './identity';

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

beforeEach(() => {
  vi.stubGlobal('localStorage', memoryStorage());
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('isValidIdentity', () => {
  test('accepts the documented grammar', () => {
    expect(isValidIdentity('yuri')).toBe(true);
    expect(isValidIdentity('anon-1a2b3c4d')).toBe(true);
    expect(isValidIdentity('user.name_1@example-corp')).toBe(true);
    expect(isValidIdentity('a')).toBe(true);
    expect(isValidIdentity('x'.repeat(64))).toBe(true);
  });

  test('rejects empty, over-long and out-of-alphabet values', () => {
    expect(isValidIdentity('')).toBe(false);
    expect(isValidIdentity('x'.repeat(65))).toBe(false);
    expect(isValidIdentity('has space')).toBe(false);
    expect(isValidIdentity('slash/es')).toBe(false);
    expect(isValidIdentity('semi;colon')).toBe(false);
  });
});

describe('getIdentity', () => {
  test('generates and persists anon-<8 hex> on first use', () => {
    const identity = getIdentity();
    expect(identity).toMatch(/^anon-[0-9a-f]{8}$/);
    expect(localStorage.getItem(IDENTITY_STORAGE_KEY)).toBe(identity);
  });

  test('prefers a stored identity', () => {
    vi.stubGlobal('localStorage', memoryStorage({ todo_identity: 'yuri' }));
    expect(getIdentity()).toBe('yuri');
  });

  test('regenerates when the stored value is not a valid identity', () => {
    vi.stubGlobal('localStorage', memoryStorage({ todo_identity: 'not ok!' }));
    expect(getIdentity()).toMatch(/^anon-[0-9a-f]{8}$/);
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

    const first = getIdentity();
    expect(first).toMatch(/^anon-[0-9a-f]{8}$/);
    expect(getIdentity()).toBe(first);
  });
});

describe('setIdentity', () => {
  test('trims, persists and returns the stored value', () => {
    expect(setIdentity('  yuri  ')).toBe('yuri');
    expect(localStorage.getItem(IDENTITY_STORAGE_KEY)).toBe('yuri');
    expect(getIdentity()).toBe('yuri');
  });

  test('returns null and writes nothing for an invalid identity', () => {
    setIdentity('yuri');
    expect(setIdentity('no spaces allowed')).toBeNull();
    expect(localStorage.getItem(IDENTITY_STORAGE_KEY)).toBe('yuri');
  });
});

describe('identityHeaders', () => {
  test('carries the identity and the app name', () => {
    setIdentity('yuri');
    expect(identityHeaders()).toEqual({
      'X-Todo-Identity': 'yuri',
      'X-Todo-App': 'web',
    });
  });
});
