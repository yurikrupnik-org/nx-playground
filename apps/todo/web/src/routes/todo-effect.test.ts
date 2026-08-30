import type { Todo, TodoEvent } from '@domain/todo';
import { Chunk, Effect, Fiber, Stream, SubscriptionRef } from 'effect';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  addTodo,
  applyChange,
  deleteTodo,
  initialState,
  load,
  setLive,
  type TodoState,
  type TodoStateRef,
  toggleTodo,
} from './todo-effect';

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

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });

/** Run a program against a fresh ref and hand back the final state. */
const withRef = <A>(
  program: (ref: TodoStateRef) => Effect.Effect<A, never>,
  seed: TodoState = initialState,
) =>
  Effect.runPromise(
    Effect.gen(function* () {
      const ref = yield* SubscriptionRef.make(seed);
      yield* program(ref);
      return yield* SubscriptionRef.get(ref);
    }),
  );

afterEach(() => vi.unstubAllGlobals());

describe('load', () => {
  it('fills the list and reports ready', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json([todo('a')])),
    );

    const state = await withRef((ref) => load(ref));

    expect(state.status).toBe('ready');
    expect(state.todos.map((t) => t.id)).toEqual(['a']);
  });

  it('retries a failing fetch before giving up', async () => {
    const fetchSpy = vi.fn(async () => json({}, 500));
    vi.stubGlobal('fetch', fetchSpy);

    const state = await withRef((ref) => load(ref));

    // One attempt plus two retries, per the exponential schedule.
    expect(fetchSpy).toHaveBeenCalledTimes(3);
    expect(state.status).toBe('error');
  });

  it('recovers when a retry succeeds', async () => {
    const fetchSpy = vi
      .fn()
      .mockResolvedValueOnce(json({}, 500))
      .mockResolvedValueOnce(json([todo('a')]));
    vi.stubGlobal('fetch', fetchSpy);

    const state = await withRef((ref) => load(ref));

    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(state.status).toBe('ready');
    expect(state.todos.map((t) => t.id)).toEqual(['a']);
  });

  it('keeps rows already on screen when a refetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json({}, 500)),
    );

    const state = await withRef((ref) => load(ref), {
      ...initialState,
      status: 'ready',
      todos: [todo('a')],
    });

    // A failed resync must not blank a working list.
    expect(state.status).toBe('ready');
    expect(state.todos.map((t) => t.id)).toEqual(['a']);
  });
});

describe('applyChange', () => {
  it('merges database events with the shared rules', async () => {
    const state = await withRef(
      (ref) =>
        Effect.andThen(
          applyChange(ref, changed('created', 'b', todo('b'))),
          applyChange(ref, changed('deleted', 'a')),
        ),
      { ...initialState, status: 'ready', todos: [todo('a')] },
    );

    expect(state.todos.map((t) => t.id)).toEqual(['b']);
  });
});

describe('writes', () => {
  it('upserts the created todo', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json(todo('new'))),
    );

    const state = await withRef((ref) =>
      addTodo(ref, { title: 'new', priority: 'high' }),
    );

    expect(state.todos.map((t) => t.id)).toEqual(['new']);
    expect(state.error).toBeUndefined();
  });

  it('surfaces the flag-specific 403 message without dropping the list', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json({}, 403)),
    );

    const state = await withRef(
      (ref) => addTodo(ref, { title: 'x', priority: 'low' }),
      { ...initialState, status: 'ready', todos: [todo('a')] },
    );

    // todo-api enforces the flag catalogue, so the reason reaches the user verbatim.
    expect(state.error).toBe(
      'writes are disabled by feature flag (todo_write)',
    );
    expect(state.todos.map((t) => t.id)).toEqual(['a']);
  });

  it('surfaces the 429 cap message', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json({}, 429)),
    );

    const state = await withRef(
      (ref) => addTodo(ref, { title: 'x', priority: 'low' }),
      { ...initialState, status: 'ready', todos: [todo('a')] },
    );

    expect(state.error).toBe('todo limit reached (todo_max_items)');
  });

  it('clears a previous error on the next successful write', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json(todo('a', { completed: true }))),
    );

    const state = await withRef((ref) => toggleTodo(ref, todo('a')), {
      ...initialState,
      status: 'ready',
      error: 'stale failure',
      todos: [todo('a')],
    });

    expect(state.error).toBeUndefined();
    expect(state.todos[0].completed).toBe(true);
  });

  it('removes the deleted row', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(null, { status: 204 })),
    );

    const state = await withRef((ref) => deleteTodo(ref, 'a'), {
      ...initialState,
      status: 'ready',
      todos: [todo('a'), todo('b')],
    });

    expect(state.todos.map((t) => t.id)).toEqual(['b']);
  });
});

describe('setLive', () => {
  it('tracks stream connectivity', async () => {
    const state = await withRef((ref) => setLive(ref, true));
    expect(state.live).toBe(true);
  });
});

describe('SubscriptionRef.changes', () => {
  it('publishes each state transition to a subscriber, newest last', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => json([todo('a')])),
    );

    const statuses = await Effect.runPromise(
      Effect.gen(function* () {
        const ref = yield* SubscriptionRef.make(initialState);

        // The mirror fiber the route uses to feed a Solid signal. `changes`
        // replays the current value first, so two emissions = initial + loaded.
        const collector = yield* Effect.fork(
          Stream.runCollect(Stream.take(ref.changes, 2)),
        );

        yield* load(ref);

        const collected = yield* Fiber.join(collector);
        return Chunk.toReadonlyArray(collected).map((state) => state.status);
      }),
    );

    expect(statuses).toEqual(['loading', 'ready']);
  });
});
