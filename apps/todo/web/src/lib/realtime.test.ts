import type { Todo, TodoEvent, TodoEventKind } from '@domain/todo';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  applyTodoEvent,
  removeTodo,
  subscribeToTodoEvents,
  upsertTodo,
} from './realtime';

function todo(overrides: Partial<Todo> & Pick<Todo, 'id'>): Todo {
  return {
    title: 'a todo',
    description: '',
    completed: false,
    priority: 'medium',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function event(
  kind: TodoEventKind,
  todoId: string,
  snapshot?: Todo,
): TodoEvent {
  return {
    event_id: 'event-1',
    kind,
    todo_id: todoId,
    todo: snapshot ?? null,
    occurred_at: '2026-01-01T00:00:00Z',
  };
}

describe('applyTodoEvent', () => {
  it('adds a todo created by somebody else', () => {
    const incoming = todo({ id: 'b', created_at: '2026-01-02T00:00:00Z' });

    const next = applyTodoEvent(
      [todo({ id: 'a' })],
      event('created', 'b', incoming),
    );

    expect(next.map((t) => t.id)).toEqual(['b', 'a']);
  });

  it('keeps the newest-first order the API returns', () => {
    const list = [
      todo({ id: 'new', created_at: '2026-03-01T00:00:00Z' }),
      todo({ id: 'old', created_at: '2026-01-01T00:00:00Z' }),
    ];
    const middle = todo({ id: 'mid', created_at: '2026-02-01T00:00:00Z' });

    const next = applyTodoEvent(list, event('created', 'mid', middle));

    expect(next.map((t) => t.id)).toEqual(['new', 'mid', 'old']);
  });

  it('replaces the row in place when a todo is completed elsewhere', () => {
    const list = [todo({ id: 'a' }), todo({ id: 'b' })];
    const completed = todo({ id: 'a', completed: true });

    const next = applyTodoEvent(list, event('completed', 'a', completed));

    expect(next.map((t) => t.id)).toEqual(['a', 'b']);
    expect(next[0].completed).toBe(true);
  });

  it('drops a todo deleted elsewhere', () => {
    const list = [todo({ id: 'a' }), todo({ id: 'b' })];

    const next = applyTodoEvent(list, event('deleted', 'a'));

    expect(next.map((t) => t.id)).toEqual(['b']);
  });

  it('is idempotent, so a reconnect refetch cannot duplicate rows', () => {
    const incoming = todo({ id: 'a', title: 'once' });
    const created = event('created', 'a', incoming);

    const next = applyTodoEvent(applyTodoEvent([], created), created);

    expect(next).toHaveLength(1);
    expect(next[0].title).toBe('once');
  });

  it('tolerates a delete for a row it never had', () => {
    const list = [todo({ id: 'a' })];

    expect(applyTodoEvent(list, event('deleted', 'ghost'))).toEqual(list);
  });

  it('ignores a non-delete event with no snapshot instead of rendering a hole', () => {
    const list = [todo({ id: 'a' })];

    expect(applyTodoEvent(list, event('updated', 'a'))).toEqual(list);
  });

  it('does not mutate the array it was given', () => {
    const list = [todo({ id: 'a' })];
    const frozen = Object.freeze([...list]);

    applyTodoEvent(frozen as Todo[], event('created', 'b', todo({ id: 'b' })));
    applyTodoEvent(frozen as Todo[], event('deleted', 'a'));

    expect(list.map((t) => t.id)).toEqual(['a']);
  });
});

describe('upsertTodo / removeTodo', () => {
  it('upsert replaces an existing row without reordering', () => {
    const list = [todo({ id: 'a' }), todo({ id: 'b' })];

    const next = upsertTodo(list, todo({ id: 'b', title: 'renamed' }));

    expect(next.map((t) => t.id)).toEqual(['a', 'b']);
    expect(next[1].title).toBe('renamed');
  });

  it('remove is a no-op for an unknown id', () => {
    const list = [todo({ id: 'a' })];

    expect(removeTodo(list, 'nope')).toEqual(list);
  });
});

describe('subscribeToTodoEvents', () => {
  /** Minimal EventSource stub whose `open` we drive by hand. */
  class StubEventSource {
    static instances: StubEventSource[] = [];
    onopen: (() => void) | null = null;
    onerror: (() => void) | null = null;
    closed = false;
    constructor(readonly url: string) {
      StubEventSource.instances.push(this);
    }
    addEventListener() {}
    close() {
      this.closed = true;
    }
    open() {
      this.onopen?.();
    }
  }

  beforeEach(() => {
    StubEventSource.instances = [];
    vi.stubGlobal('EventSource', StubEventSource);
  });
  afterEach(() => vi.unstubAllGlobals());

  it('gives a subscriber that joins an already-open stream its resync callback', () => {
    const first = vi.fn();
    const unsubscribeFirst = subscribeToTodoEvents(() => {}, { onOpen: first });

    const stream = StubEventSource.instances.at(-1);
    stream?.open();
    expect(first).toHaveBeenCalledTimes(1);

    // A second view mounts later (route change, feed alongside a list). `onopen`
    // will never fire again, but it still owes a catch-up refetch.
    const late = vi.fn();
    const unsubscribeLate = subscribeToTodoEvents(() => {}, { onOpen: late });

    expect(late).toHaveBeenCalledTimes(1);
    expect(StubEventSource.instances).toHaveLength(1);

    unsubscribeLate();
    unsubscribeFirst();
  });

  it('does not fire onOpen for a stream that has not connected yet', () => {
    const onOpen = vi.fn();
    const unsubscribe = subscribeToTodoEvents(() => {}, { onOpen });

    expect(onOpen).not.toHaveBeenCalled();

    unsubscribe();
  });

  it('closes the stream once the last subscriber leaves', () => {
    const unsubscribe = subscribeToTodoEvents(() => {});
    const stream = StubEventSource.instances.at(-1);

    unsubscribe();

    expect(stream?.closed).toBe(true);
  });

  it('opens nothing when disabled by flag', () => {
    const unsubscribe = subscribeToTodoEvents(() => {}, { enabled: false });

    expect(StubEventSource.instances).toHaveLength(0);
    unsubscribe();
  });
});
