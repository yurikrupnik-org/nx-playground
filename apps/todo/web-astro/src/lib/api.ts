// Server-side JSON client for todo-api. Used by the htmx fragment endpoints
// and the /api/todos pass-through proxy — never shipped to the browser.
import type { CreateTodo, Todo } from '@domain/todo';

/** Upstream todo-api origin (axum service serving /api/todos). */
export const API_ORIGIN = process.env.TODO_API_URL ?? 'http://127.0.0.1:8080';

const BASE = `${API_ORIGIN}/api/todos`;
const JSON_HEADERS = { 'content-type': 'application/json' } as const;

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
export async function listStacks(): Promise<StackProfile[]> {
  return (await expectOk(await fetch(`${API_ORIGIN}/api/stacks`))).json();
}

export const todoApi = {
  list: async (): Promise<Todo[]> =>
    (await expectOk(await fetch(`${BASE}?limit=100000`))).json(),

  create: async (input: CreateTodo): Promise<Todo> =>
    (
      await expectOk(
        await fetch(BASE, {
          method: 'POST',
          headers: JSON_HEADERS,
          body: JSON.stringify(input),
        }),
      )
    ).json(),

  toggle: async (id: string, completed: boolean): Promise<Todo> =>
    (
      await expectOk(
        await fetch(`${BASE}/${id}/${completed ? 'uncomplete' : 'complete'}`, {
          method: 'POST',
        }),
      )
    ).json(),

  remove: async (id: string): Promise<void> => {
    await expectOk(await fetch(`${BASE}/${id}`, { method: 'DELETE' }));
  },
};
