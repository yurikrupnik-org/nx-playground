import type { CreateTodo, Todo, TodoPriority } from '@domain/todo';
import {
  createMutation,
  createQuery,
  useQueryClient,
} from '@tanstack/solid-query';
import { createSignal, For, Show } from 'solid-js';

import { todoApi } from './lib/todo-api';

const TODOS_KEY = ['todos'] as const;
const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

export function TodoApp() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal('');
  const [priority, setPriority] = createSignal<TodoPriority>('medium');

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: TODOS_KEY });

  const todosQuery = createQuery(() => ({
    queryKey: TODOS_KEY,
    queryFn: todoApi.list,
  }));

  const addMutation = createMutation(() => ({
    mutationFn: (input: CreateTodo) => todoApi.create(input),
    onSuccess: invalidate,
  }));

  const toggleMutation = createMutation(() => ({
    mutationFn: (todo: Todo) =>
      todo.completed ? todoApi.uncomplete(todo.id) : todoApi.complete(todo.id),
    onSuccess: invalidate,
  }));

  const removeMutation = createMutation(() => ({
    mutationFn: (id: string) => todoApi.remove(id),
    onSuccess: invalidate,
  }));

  const handleSubmit = (event: Event) => {
    event.preventDefault();
    const trimmed = title().trim();
    if (!trimmed) return;
    addMutation.mutate({ title: trimmed, description: '', priority: priority() });
    setTitle('');
  };

  return (
    <main class="todo-app">
      <header class="todo-app__header">
        <h1 class="todo-app__title">Todos</h1>
        <span class="todo-app__subtitle">SolidJS</span>
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

      <Show when={todosQuery.isPending}>
        <p class="todo-status">Loading todos…</p>
      </Show>
      <Show when={todosQuery.isError}>
        <p class="todo-error" role="alert">
          Failed to load todos.
        </p>
      </Show>

      <ul class="todo-list">
        <For each={todosQuery.data ?? []}>
          {(todo) => (
            <li class="todo-item">
              <input
                class="todo-checkbox"
                type="checkbox"
                aria-label={`toggle ${todo.title}`}
                checked={todo.completed}
                onChange={() => toggleMutation.mutate(todo)}
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
                onClick={() => removeMutation.mutate(todo.id)}
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
