// Solid island variant. Client-rendered (client:only): fetches JSON from the
// same-origin /api/todos proxy and re-renders reactively. Same DOM contract as
// the htmx variant and the standalone todo-web SPA.
//
// Feature flags arrive as props, already resolved server-side by todo-api
// (GET /api/flags) — the island never calls Flagsmith and makes no extra
// round trip. The proxy attaches the identity headers upstream, so the island
// keeps talking to plain /api/todos.
import type { CreateTodo, Todo, TodoPriority } from '@domain/todo';
import { createResource, createSignal, For, Show } from 'solid-js';

const API_BASE = '/api/todos';
const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

/** Flags resolved by SSR and serialized into the island's props. */
export interface IslandFlags {
  /** `todo_app_astro` — false ⇒ render the disabled panel only. */
  appEnabled: boolean;
  /** `todo_write` — false ⇒ no add form, no checkbox, no delete button. */
  canWrite: boolean;
  /** `defaults` when Flagsmith/todo-api was unreachable. */
  source: 'remote' | 'defaults';
}

export interface SolidTodoAppProps {
  identity: string | null;
  flags: IslandFlags;
}

async function listTodos(): Promise<Todo[]> {
  const response = await fetch(`${API_BASE}?limit=100000`);
  if (!response.ok) throw new Error('Failed to fetch todos');
  return response.json();
}

export function SolidTodoApp(props: SolidTodoAppProps) {
  const [title, setTitle] = createSignal('');
  const [priority, setPriority] = createSignal<TodoPriority>('medium');
  const [todos, { refetch }] = createResource(listTodos);

  // Each mutation hits the API then refetches — the fetch-cache layer
  // (tanstack-query in todo-web) is intentionally omitted to keep the
  // island's JS payload honest for the comparison.
  const mutate = async (input: RequestInfo, init?: RequestInit) => {
    const response = await fetch(input, init);
    if (!response.ok) throw new Error('Mutation failed');
    await refetch();
  };

  const handleSubmit = (event: Event) => {
    event.preventDefault();
    const trimmed = title().trim();
    if (!trimmed) return;
    const input: CreateTodo = {
      title: trimmed,
      description: '',
      priority: priority(),
    };
    void mutate(API_BASE, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(input),
    });
    setTitle('');
  };

  return (
    <Show
      when={props.flags.appEnabled}
      fallback={
        <main class="todo-app" id="app-disabled">
          <header class="todo-app__header">
            <h1 class="todo-app__title">todo-web-astro is disabled</h1>
            <span class="todo-app__subtitle">feature flag</span>
          </header>
          <p class="todo-error" role="alert">
            todo-web-astro is disabled by feature flag{' '}
            <code>todo_app_astro</code> for identity{' '}
            <strong>{props.identity ?? 'anonymous'}</strong>.
          </p>
        </main>
      }
    >
      <main class="todo-app">
        <header class="todo-app__header">
          <h1 class="todo-app__title">Todos</h1>
          <span class="todo-app__subtitle">
            Solid island · {props.identity ?? 'anonymous'} · flags from{' '}
            {props.flags.source}
          </span>
        </header>

        <Show when={props.flags.canWrite}>
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
        <Show when={!props.flags.canWrite}>
          <p class="todo-status">
            Read-only: writes are disabled by feature flag{' '}
            <code>todo_write</code>.
          </p>
        </Show>

        <Show when={todos.loading}>
          <p class="todo-status">Loading todos…</p>
        </Show>
        <Show when={todos.error}>
          <p class="todo-error" role="alert">
            Failed to load todos.
          </p>
        </Show>

        <ul class="todo-list">
          <For each={todos() ?? []}>
            {(todo) => (
              <li class="todo-item">
                <Show when={props.flags.canWrite}>
                  <input
                    class="todo-checkbox"
                    type="checkbox"
                    aria-label={`toggle ${todo.title}`}
                    checked={todo.completed}
                    onChange={() =>
                      void mutate(
                        `${API_BASE}/${todo.id}/${todo.completed ? 'uncomplete' : 'complete'}`,
                        { method: 'POST' },
                      )
                    }
                  />
                </Show>
                <span
                  classList={{
                    'todo-item__title': true,
                    'todo-item__title--done': todo.completed,
                  }}
                >
                  {todo.title}
                </span>
                <span class={`badge badge--${todo.priority}`}>
                  {todo.priority}
                </span>
                <Show when={props.flags.canWrite}>
                  <button
                    class="btn btn--danger"
                    type="button"
                    aria-label={`delete ${todo.title}`}
                    onClick={() =>
                      void mutate(`${API_BASE}/${todo.id}`, {
                        method: 'DELETE',
                      })
                    }
                  >
                    Delete
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </main>
    </Show>
  );
}
