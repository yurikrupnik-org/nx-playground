// HTML fragment renderers for the htmx variant.
//
// The hypermedia layer: turns todo-api JSON into the same DOM (same class
// names, same shared theme) the Solid island renders. Every mutation endpoint
// responds with the full list fragment; the client swaps #todo-list wholesale
// (`hx-swap="outerHTML"`) — boring, stateless, always consistent.
import type { Todo, TodoPriority } from '@domain/todo';

export const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

/** Minimal HTML entity escaping for text and attribute values. */
export function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

/** Shared htmx attributes: every mutation re-renders the whole list. */
const SWAP = 'hx-target="#todo-list" hx-swap="outerHTML"';

export function renderTodoItem(todo: Todo): string {
  const title = escapeHtml(todo.title);
  const done = todo.completed ? ' todo-item__title--done' : '';
  return `<li class="todo-item">
  <input class="todo-checkbox" type="checkbox" aria-label="toggle ${title}"${todo.completed ? ' checked' : ''}
    hx-post="/partials/todos/${todo.id}/toggle" ${SWAP} />
  <span class="todo-item__title${done}">${title}</span>
  <span class="badge badge--${todo.priority}">${todo.priority}</span>
  <button class="btn btn--danger" type="button" aria-label="delete ${title}"
    hx-delete="/partials/todos/${todo.id}" ${SWAP}>Delete</button>
</li>`;
}

export function renderTodoList(todos: Todo[]): string {
  return `<ul class="todo-list" id="todo-list">
${todos.map(renderTodoItem).join('\n')}
</ul>`;
}

/** Standard fragment response (htmx swaps on 2xx only). */
export function fragment(html: string, status = 200): Response {
  return new Response(html, {
    status,
    headers: { 'content-type': 'text/html; charset=utf-8' },
  });
}
