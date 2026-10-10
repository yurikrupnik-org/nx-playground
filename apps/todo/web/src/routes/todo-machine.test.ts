import type { Todo, TodoEvent } from '@domain/todo';
import { describe, expect, it, vi } from 'vitest';
import { type Actor, createActor, fromPromise, waitFor } from 'xstate';

import {
  type TodoSnapshot,
  todoMachine,
  type WriteInput,
  type WriteResult,
} from './todo-machine';

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

type LoadStub = () => Promise<Todo[]>;
type WriteStub = (args: { input: WriteInput }) => Promise<WriteResult>;

/** Machine with both async actors stubbed — no network, no wall-clock waits. */
function harness(
  options: { load?: LoadStub; write?: WriteStub } = {},
): Actor<typeof todoMachine> {
  const load: LoadStub = options.load ?? (async () => []);
  const write: WriteStub =
    options.write ?? (async () => ({ kind: 'remove', id: 'x' }));

  const actor = createActor(
    todoMachine.provide({
      actors: {
        loadTodos: fromPromise<Todo[]>(load),
        write: fromPromise(write),
      },
    }),
  );
  actor.start();
  return actor;
}

/** Await a state instead of a guessed delay. */
const reach = (
  actor: Actor<typeof todoMachine>,
  match: Parameters<TodoSnapshot['matches']>[0],
) => waitFor(actor, (snapshot) => snapshot.matches(match));

describe('todoMachine load lifecycle', () => {
  it('reaches ready with the loaded list', async () => {
    const actor = harness({ load: async () => [todo('a')] });
    expect(actor.getSnapshot().matches('loading')).toBe(true);

    await reach(actor, { ready: 'idle' });

    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toEqual(['a']);
  });

  it('goes to failed on error and can retry', async () => {
    let attempt = 0;
    const actor = harness({
      load: async () => {
        attempt += 1;
        if (attempt === 1) throw new Error('boom');
        return [todo('a')];
      },
    });

    await reach(actor, 'failed');
    actor.send({ type: 'retry' });
    await reach(actor, { ready: 'idle' });

    expect(attempt).toBe(2);
  });

  it('treats a stream reconnect as a reason to retry a failed load', async () => {
    let attempt = 0;
    const actor = harness({
      load: async () => {
        attempt += 1;
        if (attempt === 1) throw new Error('boom');
        return [];
      },
    });

    await reach(actor, 'failed');
    actor.send({ type: 'streamOpen' });
    await reach(actor, { ready: 'idle' });

    expect(attempt).toBe(2);
    expect(actor.getSnapshot().context.live).toBe(true);
  });

  it('records a stream that connects before the first load finishes', async () => {
    // The common race in practice: SSE opens while `loading` is still invoked.
    // Without a root handler this event is dropped and the UI shows "off" forever.
    const gate = Promise.withResolvers<Todo[]>();
    const actor = harness({ load: () => gate.promise });

    expect(actor.getSnapshot().matches('loading')).toBe(true);
    actor.send({ type: 'streamOpen' });
    expect(actor.getSnapshot().context.live).toBe(true);

    gate.resolve([todo('a')]);
    await reach(actor, { ready: 'idle' });
    expect(actor.getSnapshot().context.live).toBe(true);
  });
});

describe('todoMachine database changes', () => {
  it('applies inserts and deletes from the stream', async () => {
    const actor = harness({ load: async () => [todo('a')] });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'change', event: changed('created', 'b', todo('b')) });
    expect(
      actor
        .getSnapshot()
        .context.todos.map((t) => t.id)
        .sort(),
    ).toEqual(['a', 'b']);

    actor.send({ type: 'change', event: changed('deleted', 'a') });
    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toEqual(['b']);
  });

  it('still applies changes while a write is in flight', async () => {
    const gate = Promise.withResolvers<WriteResult>();
    const actor = harness({ load: async () => [], write: () => gate.promise });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'remove', id: 'gone' });
    expect(actor.getSnapshot().matches({ ready: 'saving' })).toBe(true);

    // A change arriving mid-write must not be dropped.
    actor.send({ type: 'change', event: changed('created', 'c', todo('c')) });
    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toEqual(['c']);

    gate.resolve({ kind: 'remove', id: 'gone' });
    await reach(actor, { ready: 'idle' });
    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toEqual(['c']);
  });
});

describe('todoMachine writes', () => {
  it('serialises writes: a second click during saving is ignored', async () => {
    const gate = Promise.withResolvers<WriteResult>();
    const write = vi.fn<WriteStub>(() => gate.promise);
    const actor = harness({ load: async () => [todo('a')], write });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'remove', id: 'a' });
    actor.send({ type: 'remove', id: 'a' });

    expect(write).toHaveBeenCalledTimes(1);

    gate.resolve({ kind: 'remove', id: 'a' });
    await reach(actor, { ready: 'idle' });
    expect(write).toHaveBeenCalledTimes(1);
  });

  it('surfaces a write failure without leaving the saving state', async () => {
    const actor = harness({
      load: async () => [todo('a')],
      write: async () => {
        throw new Error('403 forbidden');
      },
    });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'toggle', todo: todo('a') });
    await waitFor(actor, (snapshot) => snapshot.context.error !== undefined);

    expect(actor.getSnapshot().matches({ ready: 'idle' })).toBe(true);
    expect(actor.getSnapshot().context.error).toBe('403 forbidden');
  });

  it('clears a previous error when the next write starts', async () => {
    let fail = true;
    const actor = harness({
      load: async () => [],
      write: async () => {
        if (fail) throw new Error('nope');
        return { kind: 'remove', id: 'a' };
      },
    });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'remove', id: 'a' });
    await waitFor(actor, (snapshot) => snapshot.context.error !== undefined);

    fail = false;
    actor.send({ type: 'remove', id: 'a' });
    await waitFor(actor, (snapshot) => snapshot.context.error === undefined);

    expect(actor.getSnapshot().context.error).toBeUndefined();
  });
});

describe('todoMachine resync obligation', () => {
  it('refetches on stream reconnect, because NOTIFY has no backlog', async () => {
    const load = vi
      .fn<LoadStub>()
      .mockResolvedValueOnce([todo('a')])
      .mockResolvedValueOnce([todo('a'), todo('missed-while-offline')]);
    const actor = harness({ load });
    await reach(actor, { ready: 'idle' });
    expect(load).toHaveBeenCalledTimes(1);

    actor.send({ type: 'streamOpen' });
    await waitFor(actor, (snapshot) => snapshot.context.todos.length === 2);

    expect(load).toHaveBeenCalledTimes(2);
    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toContain(
      'missed-while-offline',
    );
    expect(actor.getSnapshot().context.live).toBe(true);
  });

  it('keeps the current list if the resync fetch fails', async () => {
    const load = vi
      .fn<LoadStub>()
      .mockResolvedValueOnce([todo('a')])
      .mockRejectedValueOnce(new Error('offline again'));
    const actor = harness({ load });
    await reach(actor, { ready: 'idle' });

    actor.send({ type: 'streamOpen' });
    await reach(actor, { ready: 'resyncing' });
    await reach(actor, { ready: 'idle' });

    expect(actor.getSnapshot().context.todos.map((t) => t.id)).toEqual(['a']);
  });
});
