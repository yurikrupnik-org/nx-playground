import type { Todo, TodoEvent } from '@domain/todo';
import { afterEach, describe, expect, it, vi } from '@effect/vitest';
import { Effect, Fiber, Layer, Stream, SubscriptionRef } from 'effect';
import { TestClock } from 'effect/testing';

import {
  addTodo,
  applyChange,
  deleteTodo,
  initialState,
  load,
  TodoApi,
  TodoApiLive,
  type TodoApiShape,
  type TodoState,
  toggleTodo,
} from './todo-effect-v4';

function todo(id: string, overrides: Partial<Todo> = {}): Todo {
  return {
    id,
    title: `todo ${id}`,
    description: '',
    completed: false,
    priority: 'medium',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function changed(
  kind: TodoEvent['kind'],
  id: string,
  snapshot?: Todo,
): TodoEvent {
  return {
    event_id: `evt-${id}`,
    kind,
    todo_id: id,
    todo: snapshot ?? null,
    occurred_at: '2026-01-01T00:00:00Z',
  };
}

const ready = (todos: Todo[], extra: Partial<TodoState> = {}): TodoState => ({
  ...initialState,
  status: 'ready',
  todos,
  ...extra,
});

/** A `TodoApi` whose unlisted operations die: a test touching one is a bug. */
const fakeApi = (overrides: Partial<TodoApiShape>) =>
  Layer.succeed(TodoApi, {
    list: Effect.die('unexpected list'),
    create: () => Effect.die('unexpected create'),
    complete: () => Effect.die('unexpected complete'),
    uncomplete: () => Effect.die('unexpected uncomplete'),
    remove: () => Effect.die('unexpected remove'),
    ...overrides,
  });

/** Fails the first `failures` calls, then succeeds with `todos`; counts calls. */
const flakyList = (failures: number, todos: Todo[] = []) => {
  const calls = { count: 0 };
  const list = Effect.suspend(() => {
    calls.count += 1;
    return calls.count <= failures
      ? Effect.fail(new Error('Failed to fetch todos'))
      : Effect.succeed(todos);
  });
  return { calls, list };
};

describe('load', () => {
  it.effect('retries at +200ms and +400ms, then reports error', () =>
    Effect.gen(function* () {
      const { calls, list } = flakyList(Number.POSITIVE_INFINITY);
      const ref = yield* SubscriptionRef.make(initialState);
      const fiber = yield* load(ref).pipe(
        Effect.provide(fakeApi({ list })),
        Effect.forkChild,
      );

      yield* TestClock.adjust('199 millis');
      expect(calls.count).toBe(1);
      yield* TestClock.adjust('1 millis');
      expect(calls.count).toBe(2);
      yield* TestClock.adjust('399 millis');
      expect(calls.count).toBe(2);
      yield* TestClock.adjust('1 millis');
      yield* Fiber.join(fiber);

      expect(calls.count).toBe(3);
      expect((yield* SubscriptionRef.get(ref)).status).toBe('error');
    }),
  );

  it.effect('recovers when a retry succeeds', () =>
    Effect.gen(function* () {
      const { calls, list } = flakyList(1, [todo('a')]);
      const ref = yield* SubscriptionRef.make(initialState);
      const fiber = yield* load(ref).pipe(
        Effect.provide(fakeApi({ list })),
        Effect.forkChild,
      );

      yield* TestClock.adjust('200 millis');
      yield* Fiber.join(fiber);

      const state = yield* SubscriptionRef.get(ref);
      expect(calls.count).toBe(2);
      expect(state.status).toBe('ready');
      expect(state.todos.map((t) => t.id)).toEqual(['a']);
    }),
  );

  it.effect('gives up on a hung request after 5 seconds', () =>
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(initialState);
      const fiber = yield* load(ref).pipe(
        Effect.provide(fakeApi({ list: Effect.never })),
        Effect.forkChild,
      );

      yield* TestClock.adjust('4999 millis');
      expect((yield* SubscriptionRef.get(ref)).status).toBe('loading');
      yield* TestClock.adjust('1 millis');
      yield* Fiber.join(fiber);

      expect((yield* SubscriptionRef.get(ref)).status).toBe('error');
    }),
  );

  it.effect('keeps rows already on screen when a refetch fails', () =>
    Effect.gen(function* () {
      const { list } = flakyList(Number.POSITIVE_INFINITY);
      const ref = yield* SubscriptionRef.make(ready([todo('a')]));
      const fiber = yield* load(ref).pipe(
        Effect.provide(fakeApi({ list })),
        Effect.forkChild,
      );

      yield* TestClock.adjust('600 millis');
      yield* Fiber.join(fiber);

      // A failed resync must not blank a working list.
      const state = yield* SubscriptionRef.get(ref);
      expect(state.status).toBe('ready');
      expect(state.todos.map((t) => t.id)).toEqual(['a']);
    }),
  );
});

describe('applyChange', () => {
  it.effect('merges database events with the shared rules', () =>
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(ready([todo('a')]));

      yield* applyChange(ref, changed('created', 'b', todo('b')));
      yield* applyChange(ref, changed('deleted', 'a'));

      const state = yield* SubscriptionRef.get(ref);
      expect(state.todos.map((t) => t.id)).toEqual(['b']);
    }),
  );
});

describe('writes', () => {
  it.effect('upserts the created todo', () =>
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(initialState);

      yield* addTodo(ref, { title: 'new', priority: 'high' }).pipe(
        Effect.provide(
          fakeApi({ create: (input) => Effect.succeed(todo('new', input)) }),
        ),
      );

      const state = yield* SubscriptionRef.get(ref);
      expect(state.todos.map((t) => [t.id, t.priority])).toEqual([
        ['new', 'high'],
      ]);
      expect(state.error).toBeUndefined();
    }),
  );

  it.effect('surfaces a failed write without dropping the list', () =>
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(ready([todo('a')]));

      yield* addTodo(ref, { title: 'x', priority: 'low' }).pipe(
        Effect.provide(
          fakeApi({ create: () => Effect.fail(new Error('denied')) }),
        ),
      );

      const state = yield* SubscriptionRef.get(ref);
      expect(state.error).toBe('denied');
      expect(state.todos.map((t) => t.id)).toEqual(['a']);
    }),
  );

  it.effect('toggles through uncomplete and clears a previous error', () =>
    Effect.gen(function* () {
      const done = todo('a', { completed: true });
      const ref = yield* SubscriptionRef.make(
        ready([done], { error: 'stale failure' }),
      );

      yield* toggleTodo(ref, done).pipe(
        Effect.provide(
          fakeApi({
            uncomplete: (id) => Effect.succeed(todo(id, { completed: false })),
          }),
        ),
      );

      const state = yield* SubscriptionRef.get(ref);
      expect(state.error).toBeUndefined();
      expect(state.todos[0].completed).toBe(false);
    }),
  );

  it.effect('removes the deleted row', () =>
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(ready([todo('a'), todo('b')]));

      yield* deleteTodo(ref, 'a').pipe(
        Effect.provide(fakeApi({ remove: () => Effect.void })),
      );

      const state = yield* SubscriptionRef.get(ref);
      expect(state.todos.map((t) => t.id)).toEqual(['b']);
    }),
  );
});

describe('SubscriptionRef.changes', () => {
  it.effect(
    'publishes each state transition to a subscriber, newest last',
    () =>
      Effect.gen(function* () {
        const ref = yield* SubscriptionRef.make(initialState);

        // The mirror fiber the route uses to feed a Solid signal. `changes`
        // replays the current value first, so two emissions = initial + loaded.
        const collector = yield* Stream.runCollect(
          Stream.take(SubscriptionRef.changes(ref), 2),
        ).pipe(Effect.forkChild({ startImmediately: true }));

        yield* load(ref).pipe(
          Effect.provide(fakeApi({ list: Effect.succeed([todo('a')]) })),
        );

        const states = yield* Fiber.join(collector);
        expect(states.map((state) => state.status)).toEqual([
          'loading',
          'ready',
        ]);
      }),
  );
});

/**
 * The live layer is the one place real `fetch` is involved: it must keep the
 * REST client's messages and wire the fiber's AbortSignal into the request.
 */
describe('TodoApiLive', () => {
  afterEach(() => vi.unstubAllGlobals());

  it.effect('passes the flag-specific 403 message through', () =>
    Effect.gen(function* () {
      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response('{}', { status: 403 })),
      );
      const ref = yield* SubscriptionRef.make(ready([todo('a')]));

      yield* addTodo(ref, { title: 'x', priority: 'low' }).pipe(
        Effect.provide(TodoApiLive),
      );

      const state = yield* SubscriptionRef.get(ref);
      expect(state.error).toBe(
        'writes are disabled by feature flag (todo_write)',
      );
      expect(state.todos.map((t) => t.id)).toEqual(['a']);
    }),
  );

  it.effect('aborts the in-flight request when the fiber is interrupted', () =>
    Effect.gen(function* () {
      let signal: AbortSignal | undefined;
      vi.stubGlobal(
        'fetch',
        vi.fn((_url: string, init?: RequestInit) => {
          signal = init?.signal ?? undefined;
          return new Promise<Response>(() => {});
        }),
      );
      const ref = yield* SubscriptionRef.make(initialState);
      const fiber = yield* load(ref).pipe(
        Effect.provide(TodoApiLive),
        Effect.forkChild({ startImmediately: true }),
      );

      expect(signal?.aborted).toBe(false);
      yield* Fiber.interrupt(fiber);
      expect(signal?.aborted).toBe(true);
    }),
  );
});
