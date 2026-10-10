/**
 * The todo loop as an Effect 4 program — the `/effect-v4` twin of
 * `todo-effect.ts` (Effect 3, `/effect`).
 *
 * Same state model and policies; two deliberate differences, both things v4
 * makes cheap:
 *
 * - The REST client is a `TodoApi` service, not a module import. Programs carry
 *   it in their requirements (`R`), the route provides `TodoApiLive`, and tests
 *   provide a fake layer — no `fetch` stubbing, and no promise hops between the
 *   program and the `TestClock`, so the retry schedule and the timeout can be
 *   asserted to the millisecond without waiting.
 * - Retry is `Effect.retry({ schedule, times })`; v4 dropped
 *   `Schedule.compose`, and the options form says the same thing directly.
 *
 * Nothing in this module touches Solid.
 */
import type { CreateTodo, Todo, TodoEvent, TodoPriority } from '@domain/todo';
import { Context, Effect, Layer, Schedule, SubscriptionRef } from 'effect';

import { applyTodoEvent, removeTodo, upsertTodo } from '../lib/realtime';
import { todoApi } from '../lib/todo-api';

export interface TodoState {
  todos: Todo[];
  status: 'loading' | 'ready' | 'error';
  error?: string;
  live: boolean;
}

export const initialState: TodoState = {
  todos: [],
  status: 'loading',
  live: false,
};

export type TodoStateRef = SubscriptionRef.SubscriptionRef<TodoState>;

export const makeStateRef = SubscriptionRef.make(initialState);

/** The REST surface the loop needs, as Effects. Failures carry the UI message. */
export interface TodoApiShape {
  readonly list: Effect.Effect<Todo[], Error>;
  readonly create: (input: CreateTodo) => Effect.Effect<Todo, Error>;
  readonly complete: (id: string) => Effect.Effect<Todo, Error>;
  readonly uncomplete: (id: string) => Effect.Effect<Todo, Error>;
  readonly remove: (id: string) => Effect.Effect<void, Error>;
}

export class TodoApi extends Context.Service<TodoApi, TodoApiShape>()(
  'todo-web/TodoApi',
) {}

// `tryPromise` hands the fiber's AbortSignal to the request, so interrupting the
// fiber (route unmount closes its FiberSet) actually aborts the HTTP call rather
// than merely abandoning its result. `todoApi` already maps 403/429 to the flag
// messages; keep its Error as-is so they reach the user verbatim.
const request = <A>(
  run: (signal: AbortSignal) => Promise<A>,
  fallback: string,
): Effect.Effect<A, Error> =>
  Effect.tryPromise({
    try: run,
    catch: (cause) => (cause instanceof Error ? cause : new Error(fallback)),
  });

/** `TodoApi` backed by the real REST client (`lib/todo-api.ts`). */
export const TodoApiLive = Layer.succeed(TodoApi, {
  list: request((signal) => todoApi.list(signal), 'failed to load todos'),
  create: (input) => request(() => todoApi.create(input), 'write failed'),
  complete: (id) => request(() => todoApi.complete(id), 'write failed'),
  uncomplete: (id) => request(() => todoApi.uncomplete(id), 'write failed'),
  remove: (id) => request(() => todoApi.remove(id), 'write failed'),
});

/** Give up rather than hammer a down API: retries at +200ms and +400ms. */
const loadPolicy = {
  schedule: Schedule.exponential('200 millis'),
  times: 2,
} as const;

/**
 * Load (or reload) the list. Retries transient failures, then degrades to an
 * error state instead of failing the fiber — a dead list must not take the page
 * down.
 */
export const load = (ref: TodoStateRef) =>
  TodoApi.use((api) => api.list).pipe(
    Effect.retry(loadPolicy),
    Effect.timeout('5 seconds'),
    Effect.matchEffect({
      onSuccess: (todos) =>
        SubscriptionRef.update(ref, (state) => ({
          ...state,
          todos,
          status: 'ready' as const,
        })),
      onFailure: () =>
        SubscriptionRef.update(ref, (state) => ({
          ...state,
          // Keep any rows already on screen; only the status degrades.
          status:
            state.todos.length > 0 ? ('ready' as const) : ('error' as const),
        })),
    }),
  );

/** Merge a database change with the same pure rules the other routes use. */
export const applyChange = (ref: TodoStateRef, event: TodoEvent) =>
  SubscriptionRef.update(ref, (state) => ({
    ...state,
    todos: applyTodoEvent(state.todos, event),
  }));

export const setLive = (ref: TodoStateRef, live: boolean) =>
  SubscriptionRef.update(ref, (state) => ({ ...state, live }));

/** Shared shape for the three writes: run it, then fold the result into state. */
const write = <A>(
  ref: TodoStateRef,
  run: (api: TodoApiShape) => Effect.Effect<A, Error>,
  merge: (state: TodoState, value: A) => TodoState,
) =>
  TodoApi.use(run).pipe(
    Effect.matchEffect({
      onSuccess: (value) =>
        SubscriptionRef.update(ref, (state) => ({
          ...merge(state, value),
          error: undefined,
        })),
      onFailure: (error) =>
        SubscriptionRef.update(ref, (state) => ({
          ...state,
          error: error.message,
        })),
    }),
  );

export const addTodo = (
  ref: TodoStateRef,
  input: { title: string; priority: TodoPriority },
) =>
  write(
    ref,
    (api) => api.create({ ...input, description: '' }),
    (state, todo) => ({ ...state, todos: upsertTodo(state.todos, todo) }),
  );

export const toggleTodo = (ref: TodoStateRef, todo: Todo) =>
  write(
    ref,
    (api) => (todo.completed ? api.uncomplete(todo.id) : api.complete(todo.id)),
    (state, updated) => ({ ...state, todos: upsertTodo(state.todos, updated) }),
  );

export const deleteTodo = (ref: TodoStateRef, id: string) =>
  write(
    ref,
    (api) => api.remove(id),
    (state) => ({ ...state, todos: removeTodo(state.todos, id) }),
  );
