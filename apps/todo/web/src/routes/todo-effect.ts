/**
 * The todo loop as an Effect program.
 *
 * State lives in a `SubscriptionRef` — a `Ref` whose `changes` is a `Stream`, so
 * the UI subscribes to state instead of being pushed to. Async work is described
 * as `Effect`s and only *then* given a policy: `todoApi.list` is retried with
 * exponential backoff and a timeout, declaratively, which is the thing Effect
 * buys over hand-rolled promises here.
 *
 * Nothing in this module touches Solid. `ManagedRuntime` in the route decides
 * when the program runs and guarantees every fiber it spawned is interrupted on
 * unmount — no dangling requests after navigating away.
 */
import type { Todo, TodoEvent, TodoPriority } from '@domain/todo';
import { Duration, Effect, Schedule, SubscriptionRef } from 'effect';

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

/** Give up rather than hammer a down API; three tries over ~1.4s. */
const loadPolicy = Schedule.exponential(Duration.millis(200)).pipe(
  Schedule.compose(Schedule.recurs(2)),
);

// `tryPromise` hands the fiber's AbortSignal to the request, so interrupting the
// fiber (route unmount, `ManagedRuntime.dispose()`) actually aborts the HTTP
// call rather than merely abandoning its result.
const fetchList = Effect.tryPromise({
  try: (signal) => todoApi.list(signal),
  catch: (cause) =>
    new Error(cause instanceof Error ? cause.message : 'failed to load todos'),
});

/**
 * Load (or reload) the list. Retries transient failures, then degrades to an
 * error state instead of failing the fiber — a dead list must not take the page
 * down.
 */
export const load = (ref: TodoStateRef) =>
  fetchList.pipe(
    Effect.retry(loadPolicy),
    Effect.timeout(Duration.seconds(5)),
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
  run: () => Promise<A>,
  merge: (state: TodoState, value: A) => TodoState,
) =>
  Effect.tryPromise({
    try: run,
    catch: (cause) =>
      new Error(cause instanceof Error ? cause.message : 'write failed'),
  }).pipe(
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
    () => todoApi.create({ ...input, description: '' }),
    (state, todo) => ({ ...state, todos: upsertTodo(state.todos, todo) }),
  );

export const toggleTodo = (ref: TodoStateRef, todo: Todo) =>
  write(
    ref,
    () =>
      todo.completed ? todoApi.uncomplete(todo.id) : todoApi.complete(todo.id),
    (state, updated) => ({ ...state, todos: upsertTodo(state.todos, updated) }),
  );

export const deleteTodo = (ref: TodoStateRef, id: string) =>
  write(
    ref,
    () => todoApi.remove(id),
    (state) => ({ ...state, todos: removeTodo(state.todos, id) }),
  );
