/**
 * Realtime todo changes, sourced from the database.
 *
 * todo-api streams every committed change over SSE (`/api/events/sse`). The
 * producer is a Postgres trigger, not the API process, so this stream also
 * carries writes made by other replicas, todo-worker, the todo CLI and plain
 * `psql` — see `docs/realtime-todo.md`.
 *
 * One `EventSource` is shared by every subscriber in the page: the list view uses
 * it to patch its cache, the event feed to render a log. Opening one connection
 * per component would multiply idle server-side subscribers for no benefit.
 */
import type { Todo, TodoEvent, TodoEventKind } from '@domain/todo';

import { getIdentity } from './identity';

export const TODO_EVENT_KINDS: TodoEventKind[] = [
  'created',
  'updated',
  'completed',
  'uncompleted',
  'deleted',
];

/** Newest first, matching the API's `created_at DESC` ordering. */
const newestFirst = (a: Todo, b: Todo) =>
  b.created_at.localeCompare(a.created_at);

/**
 * Insert or replace `todo` by id, keeping the list ordered.
 *
 * Idempotent: replaying the same todo must not duplicate a row, which matters
 * because a reconnect refetch can overlap with in-flight events.
 */
export function upsertTodo(todos: Todo[], todo: Todo): Todo[] {
  const index = todos.findIndex((existing) => existing.id === todo.id);
  if (index === -1) {
    return [...todos, todo].sort(newestFirst);
  }
  const next = [...todos];
  next[index] = todo;
  return next;
}

/** Drop the row with `id`, if present. */
export function removeTodo(todos: Todo[], id: string): Todo[] {
  return todos.filter((todo) => todo.id !== id);
}

/**
 * Apply one change event to a cached list, returning a new array.
 *
 * Pure, so the merge rules are unit-testable without a server and identical for
 * every transport: `deleted` drops the row, every other kind upserts the event's
 * snapshot.
 */
export function applyTodoEvent(todos: Todo[], event: TodoEvent): Todo[] {
  if (event.kind === 'deleted') {
    return removeTodo(todos, event.todo_id);
  }

  // Non-delete kinds always carry a snapshot; ignore the event rather than
  // rendering a hole if one is missing.
  return event.todo ? upsertTodo(todos, event.todo) : todos;
}

type Listener = (event: TodoEvent) => void;

interface SubscribeOptions {
  /**
   * Called on every successful (re)connect.
   *
   * `NOTIFY` has no backlog, so changes committed while disconnected are lost:
   * callers refetch here to resynchronise.
   */
  onOpen?: () => void;
  /** Called when the stream drops; `EventSource` retries on its own. */
  onError?: () => void;
  /**
   * When `false`, nothing is registered and no connection is opened — the
   * returned function is a no-op. Used by the `todo_realtime` feature flag,
   * which todo-api also enforces by answering 403 on `/api/events/*`.
   */
  enabled?: boolean;
}

// Function-keyed, added/removed at runtime, iterated, and `.size` decides
// teardown — a Set, not a Record.
const listeners = new Set<Listener>();
const openHandlers = new Set<() => void>();
const errorHandlers = new Set<() => void>();

/** Shared unsubscribe handed back when subscription is flagged off. */
const noop = () => {};

let source: EventSource | undefined;
/** Identity `source` was opened for, so a user switch reconnects. */
let sourceIdentity: string | undefined;

/**
 * `EventSource` cannot set headers, so the identity travels as a query param —
 * the third resolution order todo-api accepts.
 */
export function todoEventsSseUrl(): string {
  return `/api/events/sse?identity=${encodeURIComponent(getIdentity())}`;
}

/** Same identity plumbing for the WebSocket transport. */
export function todoEventsWebSocketUrl(): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const identity = encodeURIComponent(getIdentity());
  return `${protocol}//${location.host}/api/events/ws?identity=${identity}`;
}

/**
 * Whether the current `source` has fired `open` and not since errored.
 *
 * Needed because subscribers arrive at different times: one that joins an
 * already-open stream never sees `onopen` again, and `onOpen` carries the
 * resync obligation (`NOTIFY` has no backlog). Without this it would silently
 * skip its catch-up refetch.
 */
let sourceOpen = false;

function ensureSource(): void {
  const identity = getIdentity();
  if (source) {
    if (sourceIdentity === identity) return;
    // Flags are evaluated per identity, so the stream must be re-established
    // as the new user; listeners are kept and re-attached below.
    source.close();
    source = undefined;
    sourceOpen = false;
  }

  const created = new EventSource(todoEventsSseUrl());
  created.onopen = () => {
    sourceOpen = true;
    for (const handler of openHandlers) handler();
  };
  created.onerror = () => {
    sourceOpen = false;
    for (const handler of errorHandlers) handler();
  };

  for (const kind of TODO_EVENT_KINDS) {
    created.addEventListener(kind, (message) => {
      let event: TodoEvent;
      try {
        event = JSON.parse((message as MessageEvent<string>).data);
      } catch {
        // A malformed frame must not tear the stream down.
        return;
      }
      for (const listener of listeners) listener(event);
    });
  }

  source = created;
  sourceIdentity = identity;
}

/**
 * Subscribe to database change events. Returns an unsubscribe function; the
 * shared connection closes once the last subscriber leaves.
 */
export function subscribeToTodoEvents(
  listener: Listener,
  options: SubscribeOptions = {},
): () => void {
  if (options.enabled === false) return noop;

  listeners.add(listener);
  if (options.onOpen) openHandlers.add(options.onOpen);
  if (options.onError) errorHandlers.add(options.onError);
  ensureSource();

  // Joining a stream that is already open still owes this subscriber its
  // catch-up refetch, and `onopen` will not fire again for it.
  if (sourceOpen && options.onOpen) options.onOpen();

  return () => {
    listeners.delete(listener);
    if (options.onOpen) openHandlers.delete(options.onOpen);
    if (options.onError) errorHandlers.delete(options.onError);

    if (listeners.size === 0 && source) {
      source.close();
      source = undefined;
      sourceIdentity = undefined;
      sourceOpen = false;
    }
  };
}
