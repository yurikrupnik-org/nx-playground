/**
 * Feature flags, resolved by todo-api.
 *
 * The SPA never talks to Flagsmith: it reads `GET /api/flags`, which todo-api
 * evaluates for the caller's identity. The response is advisory for the UI —
 * todo-api enforces the same flags server-side — so any failure here degrades
 * to "everything on" rather than blocking the app.
 */
import { identityHeaders } from './identity';

const FLAGS_URL = '/api/flags';

export interface FlagState {
  enabled: boolean;
  /** JSON passthrough of Flagsmith's `feature_state_value`. */
  value: unknown;
}

export type FlagSource = 'remote' | 'defaults';

export interface FlagsResponse {
  /** `null` when the request carried no identity. */
  identity: string | null;
  source: FlagSource;
  flags: Record<string, FlagState>;
}

export const FLAG_NAMES = [
  'todo_app_web',
  'todo_app_htmx',
  'todo_app_astro',
  'todo_realtime',
  'todo_write',
  'todo_max_items',
] as const;

export type FlagName = (typeof FLAG_NAMES)[number];

/** Fail-open catalogue defaults: every app and feature on, no item cap. */
export const DEFAULT_FLAGS: Readonly<Record<FlagName, FlagState>> =
  Object.freeze({
    todo_app_web: { enabled: true, value: null },
    todo_app_htmx: { enabled: true, value: null },
    todo_app_astro: { enabled: true, value: null },
    todo_realtime: { enabled: true, value: null },
    todo_write: { enabled: true, value: null },
    todo_max_items: { enabled: true, value: -1 },
  });

export function defaultFlagsResponse(): FlagsResponse {
  return { identity: null, source: 'defaults', flags: { ...DEFAULT_FLAGS } };
}

/**
 * Coerce an untrusted payload into a `FlagsResponse`, keeping unknown flags and
 * backfilling any the server omitted so all six catalogue flags are present.
 */
function normalize(body: unknown): FlagsResponse {
  if (typeof body !== 'object' || body === null) return defaultFlagsResponse();

  const raw = body as Partial<FlagsResponse>;
  const flags: Record<string, FlagState> = { ...DEFAULT_FLAGS };

  if (typeof raw.flags === 'object' && raw.flags !== null) {
    for (const [name, state] of Object.entries(raw.flags)) {
      if (typeof state !== 'object' || state === null) continue;
      const { enabled, value } = state as Partial<FlagState>;
      if (typeof enabled !== 'boolean') continue;
      flags[name] = { enabled, value: value ?? null };
    }
  }

  return {
    identity: typeof raw.identity === 'string' ? raw.identity : null,
    source: raw.source === 'remote' ? 'remote' : 'defaults',
    flags,
  };
}

/**
 * Fetch the flag set for the current identity. Never rejects: a network error,
 * a non-2xx status or an unparseable body all yield the built-in defaults with
 * `source: 'defaults'`.
 */
export async function fetchFlags(): Promise<FlagsResponse> {
  try {
    const response = await fetch(FLAGS_URL, { headers: identityHeaders() });
    if (!response.ok) return defaultFlagsResponse();
    return normalize(await response.json());
  } catch {
    return defaultFlagsResponse();
  }
}

/** A flag's state, falling back to the catalogue default for unknown keys. */
export function isEnabled(
  flags: Record<string, FlagState>,
  name: FlagName,
): boolean {
  return (flags[name] ?? DEFAULT_FLAGS[name]).enabled;
}

/**
 * A flag's integer payload. Flagsmith returns `feature_state_value` as either a
 * number or a string depending on how the value was typed in the dashboard, so
 * both are accepted; anything else falls back.
 */
export function intValue(
  flags: Record<string, FlagState>,
  name: FlagName,
  fallback: number,
): number {
  const value = (flags[name] ?? DEFAULT_FLAGS[name]).value;

  if (typeof value === 'number') {
    return Number.isFinite(value) ? Math.trunc(value) : fallback;
  }
  if (typeof value === 'string' && /^-?\d+$/.test(value.trim())) {
    return Number.parseInt(value.trim(), 10);
  }
  return fallback;
}
