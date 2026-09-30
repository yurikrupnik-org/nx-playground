// Server-side field projection for the /api/todos pass-through, backed by Rust.
//
// `@native/field-selector` is an N-API addon over the `field_selector` crate, so
// the projection rules here are the *same code* the Rust services use — one
// implementation of "which fields may this caller read", not a TS reimplementation
// that drifts. It runs only under the node adapter (SSR); N-API is a Node ABI and
// never reaches the browser.
//
// Two jobs:
//
// 1. **Sparse fieldsets.** `GET /api/todos?fields=id,title` trims the response.
//    The island needs 4 of the 7 fields todo-api returns, so this is a real
//    payload cut in a vertical that publishes byte counts — not a demo.
// 2. **Read gating.** Fields above the caller's role are dropped, and an unknown
//    field name is rejected rather than silently ignored.

import type { Todo } from '@domain/todo';
import { type FieldRule, FieldSchema, type Role } from '@native/field-selector';

/**
 * Read rules for `Todo`, in the order the projection emits them.
 *
 * `description` and the timestamps are gated because nothing in the rendered UI
 * reads them (`SolidTodoApp.tsx` and `fragments.ts` use id/title/completed/
 * priority only) — so a normal caller has no reason to receive them.
 */
const TODO_FIELD_RULES: FieldRule[] = [
  { field: 'id' },
  { field: 'title' },
  { field: 'completed' },
  { field: 'priority' },
  { field: 'description', requiredRole: 'user' },
  { field: 'created_at', requiredRole: 'admin' },
  { field: 'updated_at', requiredRole: 'admin' },
];

/**
 * Built once and reused across requests — the addon exposes the schema as a
 * long-lived object precisely so the rules are parsed once per process.
 */
const todoSchema = new FieldSchema(TODO_FIELD_RULES);

/** Field names a caller may ever name in `?fields=`. */
export const TODO_FIELDS = TODO_FIELD_RULES.map((rule) => rule.field);

/**
 * The caller's privilege for this request.
 *
 * **This is the seam for real authorisation, and it must stay server-derived.**
 * It deliberately ignores the identity cookie: that value is client-settable, so
 * deriving privilege from it would let anyone request `admin`. Today it is a
 * constant; wire it to a verified session (or a Flagsmith trait resolved by
 * todo-api, which is already the single flag-evaluation point) when the astro
 * host gains auth.
 */
export function resolveRole(): Role {
  return 'user';
}

/** Raised when `?fields=` names something outside the schema. */
export class InvalidFieldsError extends Error {}

/**
 * Project a todo list. `fields` is the raw `?fields=` value (`null` = every field
 * the role may read); the `a,b,c` grammar is parsed in Rust so it matches the
 * services exactly.
 *
 * @throws {InvalidFieldsError} when a requested field is not in the schema.
 */
export function projectTodos(
  todos: unknown[],
  fields: string | null,
  role: Role = resolveRole(),
): unknown[] {
  try {
    return todoSchema.filterList(todos, fields ?? undefined, role);
  } catch (cause) {
    // The addon rejects unknown names; surface that as a 400, not a 500.
    throw new InvalidFieldsError(
      cause instanceof Error ? cause.message : 'invalid fields requested',
    );
  }
}

/** Single-object variant, for `GET /api/todos/:id`. */
export function projectTodo(
  todo: unknown,
  fields: string | null,
  role: Role = resolveRole(),
): unknown {
  try {
    return todoSchema.filter(todo, fields ?? undefined, role);
  } catch (cause) {
    throw new InvalidFieldsError(
      cause instanceof Error ? cause.message : 'invalid fields requested',
    );
  }
}

/** Fields the role may read, honouring an optional `?fields=` request. */
export function allowedTodoFields(
  fields: string | null,
  role: Role = resolveRole(),
): string[] {
  try {
    return todoSchema.allowedFields(fields ?? undefined, role);
  } catch (cause) {
    throw new InvalidFieldsError(
      cause instanceof Error ? cause.message : 'invalid fields requested',
    );
  }
}

/** Narrow a projected value back to a partial `Todo` for typed consumers. */
export type ProjectedTodo = Partial<Todo>;
