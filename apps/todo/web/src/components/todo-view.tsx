import type { Todo, TodoPriority } from '@domain/todo';
import { createSignal, For, Show } from 'solid-js';

const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

export type TodoViewStatus = 'loading' | 'ready' | 'error';

export interface TodoViewProps {
  /** Which implementation is driving this view, shown in the header. */
  variant: string;
  /** One-line description of how state is held. */
  summary: string;
  todos: Todo[];
  status: TodoViewStatus;
  /** Non-fatal message (a failed mutation), rendered above the list. */
  error?: string;
  /** True while a write is in flight, to disable the form. */
  busy?: boolean;
  /** Connection state of the database event stream. */
  live?: boolean;
  onAdd: (input: { title: string; priority: TodoPriority }) => void;
  onToggle: (todo: Todo) => void;
  onRemove: (id: string) => void;
}

/**
 * Presentation only — no fetching, no stores, no library imports.
 *
 * The `/xstate` and `/effect` routes share this component so the *only*
 * difference between those implementations is how state is held and updated.
 * Keeping the markup here is what makes the comparison honest (and keeps their
 * route chunks down to the library plus glue).
 */
export function TodoView(props: TodoViewProps) {
  const [title, setTitle] = createSignal('');
  const [priority, setPriority] = createSignal<TodoPriority>('medium');

  const submit = (event: Event) => {
    event.preventDefault();
    const trimmed = title().trim();
    if (!trimmed) return;
    props.onAdd({ title: trimmed, priority: priority() });
    setTitle('');
  };

  return (
    <main class="todo-app">
      <header class="todo-app__header">
        <h1 class="todo-app__title">Todos</h1>
        <span class="todo-app__subtitle">{props.variant}</span>
      </header>

      <p class="todo-status">
        {props.summary}
        {' · stream '}
        <span class={['badge', props.live ? 'badge--medium' : 'badge--high']}>
          {props.live ? 'live' : 'off'}
        </span>
      </p>

      <form class="todo-form" onSubmit={submit}>
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
        <button class="btn btn--primary" type="submit" disabled={props.busy}>
          Add
        </button>
      </form>

      <Show when={props.status === 'loading'}>
        <p class="todo-status">Loading todos…</p>
      </Show>
      <Show when={props.status === 'error'}>
        <p class="todo-error" role="alert">
          Failed to load todos.
        </p>
      </Show>
      <Show when={props.error}>
        {(message) => (
          <p class="todo-error" role="alert">
            {message()}
          </p>
        )}
      </Show>

      <ul class="todo-list">
        <For each={props.todos}>
          {(todo) => (
            <li class="todo-item">
              <input
                class="todo-checkbox"
                type="checkbox"
                aria-label={`toggle ${todo.title}`}
                checked={todo.completed}
                onChange={() => props.onToggle(todo)}
              />
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
              <button
                class="btn btn--danger"
                type="button"
                aria-label={`delete ${todo.title}`}
                onClick={() => props.onRemove(todo.id)}
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
