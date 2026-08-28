// Solid island variant. Client-rendered (client:only): fetches JSON from the
// same-origin /api/todos proxy and re-renders reactively. Same DOM contract as
// the htmx variant and the standalone todo-web SPA.
import type { CreateTodo, Todo, TodoPriority } from '@domain/todo';
import { createResource, createSignal, For, Show } from 'solid-js';

const API_BASE = '/api/todos';
const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

async function listTodos(): Promise<Todo[]> {
  const response = await fetch(`${API_BASE}?limit=100000`);
  if (!response.ok) throw new Error('Failed to fetch todos');
  return response.json();
}

export function SolidTodoApp() {
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
    <main class="todo-app">
      <header class="todo-app__header">
        <h1 class="todo-app__title">Todos</h1>
        <span class="todo-app__subtitle">Solid island</span>
      </header>

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
              <button
                class="btn btn--danger"
                type="button"
                aria-label={`delete ${todo.title}`}
                onClick={() =>
                  void mutate(`${API_BASE}/${todo.id}`, { method: 'DELETE' })
                }
              >
                Delete
              </button>
            </li>
          )}
        </For>
      </ul>
    </main>
  );
}
