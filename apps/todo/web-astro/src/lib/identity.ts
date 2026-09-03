// Per-user identity for feature-flag evaluation.
//
// The Astro variant is server-rendered, so identity lives in a plain cookie
// (`todo_identity`) that the server reads on every request and forwards to
// todo-api as `X-Todo-Identity`. todo-api is the only flag evaluation point;
// this module just carries the identity to it.

/** Cookie carrying the flag identity (also readable by the browser). */
export const IDENTITY_COOKIE = 'todo_identity';

/** This frontend's Flagsmith trait value (`app`), sent as `X-Todo-App`. */
export const APP_NAME = 'astro';

/** One year, matching the other todo frontends. */
export const IDENTITY_MAX_AGE = 31536000;

/** Valid identity: 1..=64 chars of `[A-Za-z0-9_.@-]`. */
const IDENTITY_PATTERN = /^[A-Za-z0-9_.@-]{1,64}$/;

/**
 * Structural view of `Astro.cookies` (`AstroCookies` satisfies it). Keeps this
 * module unit-testable without booting an Astro request.
 */
export interface IdentityCookies {
  get(key: string): { value: string } | undefined;
  set(
    key: string,
    value: string,
    options?: { path?: string; sameSite?: 'lax'; maxAge?: number },
  ): void;
  delete(key: string, options?: { path?: string }): void;
}

/** Trim + validate; anything unusable collapses to `null` (= anonymous). */
export function normalizeIdentity(
  value: string | null | undefined,
): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return IDENTITY_PATTERN.test(trimmed) ? trimmed : null;
}

/** Identity carried by the current request, or `null` when absent/invalid. */
export function readIdentity(cookies: IdentityCookies): string | null {
  return normalizeIdentity(cookies.get(IDENTITY_COOKIE)?.value);
}

/**
 * Headers every upstream todo-api call must carry. The identity header is
 * omitted entirely when there is none, so todo-api reports `identity: null`
 * rather than evaluating flags for an empty string.
 */
export function identityHeaders(
  identity: string | null,
): Record<string, string> {
  const headers: Record<string, string> = { 'X-Todo-App': APP_NAME };
  if (identity) headers['X-Todo-Identity'] = identity;
  return headers;
}

/**
 * Persist (or clear) the identity cookie, returning what is now in effect.
 * An empty/invalid value deletes it — that is how the identity form switches
 * back to anonymous.
 */
export function setIdentityCookie(
  cookies: IdentityCookies,
  value: string,
): string | null {
  const identity = normalizeIdentity(value);
  if (!identity) {
    cookies.delete(IDENTITY_COOKIE, { path: '/' });
    return null;
  }
  cookies.set(IDENTITY_COOKIE, identity, {
    path: '/',
    sameSite: 'lax',
    maxAge: IDENTITY_MAX_AGE,
  });
  return identity;
}
