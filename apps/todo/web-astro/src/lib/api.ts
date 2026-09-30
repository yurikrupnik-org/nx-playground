// Server-side JSON client for todo-api. Used by the htmx fragment endpoints,
// the /api/todos pass-through proxy and every SSR page — never shipped to the
// browser.
//
// todo-api is the single feature-flag evaluation point: it resolves the flag
// set per identity (Flagsmith server-side) and serves it at GET /api/flags.
// Every call from here carries the identity headers so the API can evaluate
// and enforce flags for the same user.
import type { CreateTodo, Todo } from '@domain/todo';

import {
  type IdentityCookies,
  identityHeaders,
  readIdentity,
} from './identity';

/** Upstream todo-api origin (axum service serving /api/todos). */
export const API_ORIGIN = process.env.TODO_API_URL ?? 'http://127.0.0.1:8080';

const BASE = `${API_ORIGIN}/api/todos`;
const JSON_HEADERS = { 'content-type': 'application/json' } as const;

/** Flag lookups must never hang a page render; defaults are always available. */
const FLAGS_TIMEOUT_MS = 1500;

async function expectOk(response: Response): Promise<Response> {
  if (!response.ok) {
    throw new Error(`todo-api ${response.status}: ${await response.text()}`);
  }
  return response;
}

/**
 * One frontend stack profile row (reference data in Postgres, table
 * `stack_profiles`). Mirrors the Rust `StackProfile` struct in
 * `apps/todo/api/src/stacks.rs` — keep the two in sync.
 */
export interface StackProfile {
  slug: string;
  name: string;
  language: string;
  is_default: boolean;
  js_kb: number;
  html_kb: number;
  requests: number;
  notes: string;
}

/** Stack profiles, cheapest first (the API orders by total transfer). */
export async function listStacks(
  identity: string | null,
): Promise<StackProfile[]> {
  return (
    await expectOk(
      await fetch(`${API_ORIGIN}/api/stacks`, {
        headers: identityHeaders(identity),
      }),
    )
  ).json();
}

export const todoApi = {
  list: async (identity: string | null): Promise<Todo[]> =>
    (
      await expectOk(
        await fetch(`${BASE}?limit=100000`, {
          headers: identityHeaders(identity),
        }),
      )
    ).json(),

  create: async (input: CreateTodo, identity: string | null): Promise<Todo> =>
    (
      await expectOk(
        await fetch(BASE, {
          method: 'POST',
          headers: { ...JSON_HEADERS, ...identityHeaders(identity) },
          body: JSON.stringify(input),
        }),
      )
    ).json(),

  toggle: async (
    id: string,
    completed: boolean,
    identity: string | null,
  ): Promise<Todo> =>
    (
      await expectOk(
        await fetch(`${BASE}/${id}/${completed ? 'uncomplete' : 'complete'}`, {
          method: 'POST',
          headers: identityHeaders(identity),
        }),
      )
    ).json(),

  remove: async (id: string, identity: string | null): Promise<void> => {
    await expectOk(
      await fetch(`${BASE}/${id}`, {
        method: 'DELETE',
        headers: identityHeaders(identity),
      }),
    );
  },
};

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

/** The flag catalogue. All six are always present in a resolved flag set. */
export const FLAG_KEYS = [
  'todo_app_web',
  'todo_app_htmx',
  'todo_app_astro',
  'todo_realtime',
  'todo_write',
  'todo_max_items',
] as const;

export type FlagKey = (typeof FLAG_KEYS)[number];

/** `value` is a passthrough of Flagsmith's `feature_state_value`. */
export interface FlagState {
  enabled: boolean;
  value: number | string | null;
}

/** Wire model of `GET /api/flags`. */
export interface FlagSet {
  identity: string | null;
  /** `defaults` = Flagsmith unconfigured or unreachable; not an error. */
  source: 'remote' | 'defaults';
  flags: Record<string, FlagState>;
}

/** Everything on, no item cap — the mandatory degradation target. */
export function defaultFlags(identity: string | null): FlagSet {
  return {
    identity,
    source: 'defaults',
    flags: {
      todo_app_web: { enabled: true, value: null },
      todo_app_htmx: { enabled: true, value: null },
      todo_app_astro: { enabled: true, value: null },
      todo_realtime: { enabled: true, value: null },
      todo_write: { enabled: true, value: null },
      todo_max_items: { enabled: true, value: -1 },
    },
  };
}

/** Unknown/missing flags read as enabled: a gap must never disable the app. */
export function isEnabled(flags: FlagSet, key: string): boolean {
  return flags.flags[key]?.enabled ?? true;
}

/** Integer payload of a flag; tolerates a numeric string from Flagsmith. */
export function intValue(
  flags: FlagSet,
  key: string,
  fallback: number,
): number {
  const raw = flags.flags[key]?.value;
  if (typeof raw === 'number') {
    return Number.isFinite(raw) ? Math.trunc(raw) : fallback;
  }
  if (typeof raw === 'string' && raw.trim() !== '') {
    const parsed = Number(raw);
    if (Number.isFinite(parsed)) return Math.trunc(parsed);
  }
  return fallback;
}

/** Coerce an untrusted `GET /api/flags` payload; missing keys fall back. */
function parseFlagSet(payload: unknown, identity: string | null): FlagSet {
  if (
    typeof payload !== 'object' ||
    payload === null ||
    !('flags' in payload)
  ) {
    throw new Error('todo-api /api/flags: missing flags');
  }
  const rawFlags = payload.flags;
  if (typeof rawFlags !== 'object' || rawFlags === null) {
    throw new Error('todo-api /api/flags: flags is not an object');
  }

  const resolved = defaultFlags(identity);
  // `object` alone yields `any` values from Object.entries; keep them unknown.
  for (const [key, state] of Object.entries(
    rawFlags as Record<string, unknown>,
  )) {
    if (typeof state !== 'object' || state === null) continue;
    const enabled = 'enabled' in state ? state.enabled : true;
    const value = 'value' in state ? state.value : null;
    resolved.flags[key] = {
      enabled: enabled !== false,
      value:
        typeof value === 'number' || typeof value === 'string' ? value : null,
    };
  }

  const claimed = 'identity' in payload ? payload.identity : null;
  return {
    identity: typeof claimed === 'string' ? claimed : identity,
    source:
      'source' in payload && payload.source === 'remote'
        ? 'remote'
        : 'defaults',
    flags: resolved.flags,
  };
}

/** Resolved flag set for `identity`. Throws when todo-api is unreachable. */
export async function fetchFlags(identity: string | null): Promise<FlagSet> {
  const response = await expectOk(
    await fetch(`${API_ORIGIN}/api/flags`, {
      headers: identityHeaders(identity),
      signal: AbortSignal.timeout(FLAGS_TIMEOUT_MS),
    }),
  );
  return parseFlagSet(await response.json(), identity);
}

/**
 * Flag set that never fails: any error, timeout or malformed payload degrades
 * to all-on defaults so the app keeps working with Flagsmith (or todo-api)
 * down. `source` then reads `defaults`, which the UI surfaces.
 */
export async function fetchFlagsOrDefaults(
  identity: string | null,
): Promise<FlagSet> {
  try {
    return await fetchFlags(identity);
  } catch {
    return defaultFlags(identity);
  }
}

/** Everything a page or endpoint needs to gate itself, resolved per request. */
export interface FlagContext {
  identity: string | null;
  flags: FlagSet;
  /** `todo_app_astro` — off ⇒ respond 503. */
  appEnabled: boolean;
  /** `todo_write` — off ⇒ refuse mutations, render read-only. */
  canWrite: boolean;
  /** `todo_realtime` — off ⇒ do not subscribe to /api/events/*. */
  realtime: boolean;
  /** `todo_max_items`, `-1` = unlimited. */
  maxItems: number;
}

/** Read the identity cookie, resolve flags, and derive the gates. */
export async function flagContext(
  cookies: IdentityCookies,
): Promise<FlagContext> {
  const identity = readIdentity(cookies);
  const flags = await fetchFlagsOrDefaults(identity);
  return {
    identity,
    flags,
    appEnabled: isEnabled(flags, 'todo_app_astro'),
    canWrite: isEnabled(flags, 'todo_write'),
    realtime: isEnabled(flags, 'todo_realtime'),
    maxItems: intValue(flags, 'todo_max_items', -1),
  };
}
