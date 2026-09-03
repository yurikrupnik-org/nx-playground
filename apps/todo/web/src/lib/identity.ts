/**
 * Per-user identity for feature-flag evaluation.
 *
 * todo-api is the single flag evaluation point: it resolves the caller's
 * identity (`X-Todo-Identity` header, `todo_identity` cookie, `?identity=`
 * query param) and asks Flagsmith for that user. The SPA therefore only has to
 * pick an identity, persist it, and attach it to everything it sends.
 *
 * Storage is `localStorage['todo_identity']`, with an in-memory fallback so a
 * blocked/absent storage (private mode, embedded webview) degrades instead of
 * throwing.
 */

/** App name reported to todo-api; becomes the Flagsmith trait `app`. */
export const TODO_APP = 'web';

export const IDENTITY_STORAGE_KEY = 'todo_identity';

/** The identity grammar todo-api accepts: 1..=64 chars of `[A-Za-z0-9_.@-]`. */
const IDENTITY_PATTERN = /^[A-Za-z0-9_.@-]{1,64}$/;

/** Survives a `localStorage` that throws or is missing. */
let inMemoryIdentity: string | undefined;

function readStored(): string | undefined {
  try {
    return globalThis.localStorage?.getItem(IDENTITY_STORAGE_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function writeStored(value: string): void {
  try {
    globalThis.localStorage?.setItem(IDENTITY_STORAGE_KEY, value);
  } catch {
    // Storage is unavailable; `inMemoryIdentity` already holds the value.
  }
}

export function isValidIdentity(value: string): boolean {
  return IDENTITY_PATTERN.test(value);
}

/** `anon-<8 lowercase hex>`, the default identity for a first-time visitor. */
function generateIdentity(): string {
  const bytes = new Uint8Array(4);
  crypto.getRandomValues(bytes);
  let hex = '';
  for (const byte of bytes) hex += byte.toString(16).padStart(2, '0');
  return `anon-${hex}`;
}

/**
 * The current identity, generating and persisting an anonymous one on first
 * use. A stored value wins over the in-memory one so another tab's switch is
 * picked up.
 */
export function getIdentity(): string {
  const stored = readStored();
  if (stored && isValidIdentity(stored)) {
    inMemoryIdentity = stored;
    return stored;
  }
  if (inMemoryIdentity) return inMemoryIdentity;

  const generated = generateIdentity();
  inMemoryIdentity = generated;
  writeStored(generated);
  return generated;
}

/**
 * Persist `value` as the identity. Returns the stored (trimmed) value, or
 * `null` when it does not match the identity grammar — in which case nothing
 * is written.
 */
export function setIdentity(value: string): string | null {
  const trimmed = value.trim();
  if (!isValidIdentity(trimmed)) return null;

  inMemoryIdentity = trimmed;
  writeStored(trimmed);
  return trimmed;
}

/** Headers every todo-api request must carry so flags resolve per user/app. */
export function identityHeaders(): Record<string, string> {
  return {
    'X-Todo-Identity': getIdentity(),
    'X-Todo-App': TODO_APP,
  };
}
