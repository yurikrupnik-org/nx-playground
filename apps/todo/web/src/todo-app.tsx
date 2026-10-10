import type { CreateTodo, Todo, TodoPriority } from '@domain/todo';
import {
  createMutation,
  createQuery,
  useQueryClient,
} from '@tanstack/solid-query';
import { createEffect, createSignal, For, Show } from 'solid-js';

import { EventFeed } from './event-feed';
import { DEFAULT_FLAGS, fetchFlags, intValue, isEnabled } from './lib/flags';
import { getIdentity, setIdentity } from './lib/identity';
import {
  applyTodoEvent,
  removeTodo,
  subscribeToTodoEvents,
  upsertTodo,
} from './lib/realtime';
import { todoApi } from './lib/todo-api';

const TODOS_KEY = ['todos'] as const;
const FLAGS_KEY = ['flags'] as const;
const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

/** Flags are cheap to re-evaluate but shouldn't refetch on every focus. */
const FLAGS_STALE_TIME = 30_000;

/**
 * Switch the Flagsmith identity the whole app is evaluated as. Present in both
 * the normal and the flagged-off view, so a demo can always switch back.
 */
function IdentitySwitcher(props: {
  identity: string;
  onSwitch: (identity: string) => void;
}) {
  // Writable derived: resets to the applied identity whenever it changes.
  const [draft, setDraft] = createSignal(() => props.identity);
  const [rejected, setRejected] = createSignal(false);

  const submit = (event: Event) => {
    event.preventDefault();
    const stored = setIdentity(draft());
    setRejected(stored === null);
    if (stored !== null) props.onSwitch(stored);
  };

  return (
    <form class="todo-form" onSubmit={submit}>
      <input
        class="todo-input"
        type="text"
        aria-label="identity"
        placeholder="identity"
        value={draft()}
        onInput={(event) => setDraft(event.currentTarget.value)}
      />
      <button class="btn btn--primary" type="submit">
        Switch user
      </button>
      <Show when={rejected()}>
        <span class="todo-error" role="alert">
          1–64 chars of A–Z a–z 0–9 _ . @ -
        </span>
      </Show>
    </form>
  );
}

export function TodoApp() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal('');
  const [priority, setPriority] = createSignal<TodoPriority>('medium');
  const [identity, setActiveIdentity] = createSignal(getIdentity());

  // todo-api is the single flag evaluation point; `fetchFlags` never rejects,
  // falling back to the everything-on defaults so a Flagsmith outage or an
  // unreachable /api/flags cannot blank the page.
  const flagsQuery = createQuery(() => ({
    queryKey: [...FLAGS_KEY, identity()],
    queryFn: fetchFlags,
    staleTime: FLAGS_STALE_TIME,
    retry: false,
    throwOnError: false,
  }));

  const flags = () => flagsQuery.data?.flags ?? DEFAULT_FLAGS;
  const flagSource = () => flagsQuery.data?.source ?? 'defaults';
  // Rendering gates fail open on the built-in defaults, so a slow or broken
  // /api/flags shows the working UI instead of an empty page.
  const appEnabled = () => isEnabled(flags(), 'todo_app_web');
  const writeEnabled = () => isEnabled(flags(), 'todo_write');
  const maxItems = () => intValue(flags(), 'todo_max_items', -1);
  // Connections are the exception: opening a stream todo-api may answer 403 to,
  // only to close it a tick later, is worse than waiting one round trip.
  const realtimeEnabled = () =>
    flagsQuery.isFetched && isEnabled(flags(), 'todo_realtime');

  // Held until flags resolve: a `todo_app_web`-disabled SPA must not fetch.
  const todosQuery = createQuery(() => ({
    queryKey: TODOS_KEY,
    queryFn: ({ signal }) => todoApi.list(signal),
    enabled: flagsQuery.isFetched && appEnabled(),
  }));

  /** Patch the cached list in place; no refetch. */
  const patch = (update: (todos: Todo[]) => Todo[]) =>
    queryClient.setQueryData<Todo[]>(TODOS_KEY, (current) =>
      current ? update(current) : current,
    );

  // The list is driven by database change events, so it also reflects writes made
  // by anything else touching the table (another replica, todo-worker, the CLI,
  // psql). Mutations below patch the same cache from their own response, so a
  // local edit still lands instantly if the stream is momentarily down.
  //
  // Re-runs when `todo_realtime` flips or the identity changes; the returned
  // unsubscribe is the effect's cleanup, so the stream is torn down on both.
  createEffect(
    () => ({ enabled: realtimeEnabled(), identity: identity() }),
    (config) =>
      subscribeToTodoEvents(
        (event) => patch((todos) => applyTodoEvent(todos, event)),
        {
          enabled: config.enabled,
          // NOTIFY has no backlog: refetch after a reconnect to pick up whatever
          // was committed while the stream was down.
          onOpen: () => {
            void queryClient.invalidateQueries({ queryKey: TODOS_KEY });
          },
        },
      ),
  );

  const addMutation = createMutation(() => ({
    mutationFn: (input: CreateTodo) => todoApi.create(input),
    onSuccess: (todo) => patch((todos) => upsertTodo(todos, todo)),
  }));

  const toggleMutation = createMutation(() => ({
    mutationFn: (todo: Todo) =>
      todo.completed ? todoApi.uncomplete(todo.id) : todoApi.complete(todo.id),
    onSuccess: (todo) => patch((todos) => upsertTodo(todos, todo)),
  }));

  const removeMutation = createMutation(() => ({
    mutationFn: (id: string) => todoApi.remove(id),
    onSuccess: (_result, id) => patch((todos) => removeTodo(todos, id)),
  }));

  // todo-api answers 403 (todo_write off) and 429 (todo_max_items reached);
  // surface those instead of letting a click do nothing.
  const mutationError = () =>
    addMutation.error?.message ??
    toggleMutation.error?.message ??
    removeMutation.error?.message;

  const switchIdentity = (next: string) => {
    setActiveIdentity(next);
    void queryClient.invalidateQueries({ queryKey: FLAGS_KEY });
    void queryClient.invalidateQueries({ queryKey: TODOS_KEY });
  };

  const handleSubmit = (event: Event) => {
    event.preventDefault();
    const trimmed = title().trim();
    if (!trimmed) return;
    addMutation.mutate({
      title: trimmed,
      description: '',
      priority: priority(),
    });
    setTitle('');
  };

  const statusStrip = () => (
    <p class="todo-status">
      Identity <span class="badge badge--low">{identity()}</span> flags{' '}
      <span
        class={[
          'badge',
          flagSource() === 'remote' ? 'badge--medium' : 'badge--high',
        ]}
      >
        {flagSource()}
      </span>
      <Show when={maxItems() >= 0}>
        {' '}
        cap <span class="badge badge--high">{maxItems()}</span>
      </Show>
    </p>
  );

  return (
    <Show
      when={appEnabled()}
      fallback={
        <main class="todo-app">
          <header class="todo-app__header">
            <h1 class="todo-app__title">Unavailable</h1>
            <span class="todo-app__subtitle">SolidJS</span>
          </header>
          <p class="todo-error" role="alert">
            todo-web is disabled by feature flag <strong>todo_app_web</strong>{' '}
            for identity <strong>{identity()}</strong>.
          </p>
          {statusStrip()}
          <IdentitySwitcher identity={identity()} onSwitch={switchIdentity} />
        </main>
      }
    >
      <main class="todo-app">
        <header class="todo-app__header">
          <h1 class="todo-app__title">Todos</h1>
          <span class="todo-app__subtitle">SolidJS</span>
        </header>

        {statusStrip()}
        <IdentitySwitcher identity={identity()} onSwitch={switchIdentity} />

        <Show when={writeEnabled()}>
          <form class="todo-form" onSubmit={handleSubmit}>
            <input
              class="todo-input"
              type="text"
              placeholder="new todo title"
              aria-label="new todo title"
              value={title()}
              onInput={(event) => setTitle(event.currentTarget.value)}
            />
            <select
              class="todo-select"
              aria-label="priority"
              value={priority()}
              onChange={(event) =>
                setPriority(event.currentTarget.value as TodoPriority)
              }
            >
              <For each={PRIORITIES}>
                {(value) => <option value={value}>{value}</option>}
              </For>
            </select>
            <button class="btn btn--primary" type="submit">
              Add
            </button>
          </form>
        </Show>
        <Show when={!writeEnabled()}>
          <p class="todo-status">
            Read-only: writes are disabled by feature flag{' '}
            <strong>todo_write</strong>.
          </p>
        </Show>

        <Show when={todosQuery.isPending}>
          <p class="todo-status">Loading todos…</p>
        </Show>
        <Show when={todosQuery.isError}>
          <p class="todo-error" role="alert">
            Failed to load todos.
          </p>
        </Show>
        <Show when={mutationError()}>
          {(message) => (
            <p class="todo-error" role="alert">
              {message()}
            </p>
          )}
        </Show>

        <ul class="todo-list">
          <For each={todosQuery.data ?? []}>
            {(todo) => (
              <li class="todo-item">
                <Show when={writeEnabled()}>
                  <input
                    class="todo-checkbox"
                    type="checkbox"
                    aria-label={`toggle ${todo.title}`}
                    checked={todo.completed}
                    onChange={() => toggleMutation.mutate(todo)}
                  />
                </Show>
                <span
                  class={[
                    'todo-item__title',
                    { 'todo-item__title--done': todo.completed },
                  ]}
                >
                  {todo.title}
                </span>
                <span class={`badge badge--${todo.priority}`}>
                  {todo.priority}
                </span>
                <Show when={writeEnabled()}>
                  <button
                    class="btn btn--danger"
                    type="button"
                    aria-label={`delete ${todo.title}`}
                    onClick={() => removeMutation.mutate(todo.id)}
                  >
                    Delete
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>

        <Show when={realtimeEnabled()}>
          <EventFeed />
        </Show>
      </main>
    </Show>
  );
}
