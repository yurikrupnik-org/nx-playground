// HTML fragment renderers for the htmx variant (and the shared flag/identity
// chrome every page renders).
//
// The hypermedia layer: turns todo-api JSON into the same DOM (same class
// names, same shared theme) the Solid island renders. Every mutation endpoint
// responds with the full list fragment; the client swaps #todo-list wholesale
// (`hx-swap="outerHTML"`) — boring, stateless, always consistent.
//
// Feature flags arrive resolved from todo-api (GET /api/flags). When
// `todo_write` is off the list renders read-only — no checkbox, no delete
// button, no add form — mirroring the 403 the API itself returns.
import type { Todo, TodoPriority } from '@domain/todo';

import { FLAG_KEYS, type FlagSet, intValue, isEnabled } from './api';

export const PRIORITIES: TodoPriority[] = ['low', 'medium', 'high'];

/** Flag that gates this whole app; off ⇒ every route answers 503. */
export const APP_FLAG = 'todo_app_astro';

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

/** Write-gated renderers take the resolved `todo_write` value. */
export interface WriteOptions {
  /** `todo_write`; defaults to on so a flag gap never breaks the UI. */
  canWrite?: boolean;
}

export function renderTodoItem(
  todo: Todo,
  { canWrite = true }: WriteOptions = {},
): string {
  const title = escapeHtml(todo.title);
  const done = todo.completed ? ' todo-item__title--done' : '';
  const toggle = canWrite
    ? `  <input class="todo-checkbox" type="checkbox" aria-label="toggle ${title}"${todo.completed ? ' checked' : ''}
    hx-post="/partials/todos/${todo.id}/toggle" ${SWAP} />\n`
    : '';
  const remove = canWrite
    ? `
  <button class="btn btn--danger" type="button" aria-label="delete ${title}"
    hx-delete="/partials/todos/${todo.id}" ${SWAP}>Delete</button>`
    : '';
  return `<li class="todo-item">
${toggle}  <span class="todo-item__title${done}">${title}</span>
  <span class="badge badge--${todo.priority}">${todo.priority}</span>${remove}
</li>`;
}

export function renderTodoList(
  todos: Todo[],
  options: WriteOptions = {},
): string {
  return `<ul class="todo-list" id="todo-list">
${todos.map((todo) => renderTodoItem(todo, options)).join('\n')}
</ul>`;
}

/** Create form; rendered only while `todo_write` is on. */
export function renderAddForm(): string {
  const options = PRIORITIES.map(
    (value) =>
      `<option value="${value}"${value === 'medium' ? ' selected' : ''}>${value}</option>`,
  ).join('');
  return `<form class="todo-form" id="todo-add"
  hx-post="/partials/todos" ${SWAP}
  hx-on::after-request="if (event.detail.successful) this.reset()">
  <input class="todo-input" type="text" name="title" placeholder="new todo title"
    aria-label="new todo title" required />
  <select class="todo-select" name="priority" aria-label="priority">${options}</select>
  <button class="btn btn--primary" type="submit">Add</button>
</form>`;
}

/**
 * Identity switcher: a plain HTML POST to /identity (no htmx, no JS) so it
 * keeps working on the 503 page and with the island unmounted. Submitting an
 * empty value clears the cookie = anonymous.
 */
export function renderIdentityForm(identity: string | null): string {
  const value = escapeHtml(identity ?? '');
  return `<form class="todo-form" id="identity-form" method="post" action="/identity">
  <input class="todo-input" type="text" name="identity" value="${value}"
    placeholder="identity (blank = anonymous)" aria-label="flag identity"
    pattern="[A-Za-z0-9_.@\\-]{0,64}" />
  <button class="btn btn--primary" type="submit">Switch user</button>
</form>`;
}

/**
 * Flag/status strip: current identity, where the flag set came from
 * (`remote` vs `defaults` when Flagsmith is unreachable) and every flag.
 */
export function renderFlagStrip(flags: FlagSet): string {
  const identity = flags.identity
    ? `<strong>${escapeHtml(flags.identity)}</strong>`
    : '<strong>anonymous</strong>';
  const badges = FLAG_KEYS.map((key) => {
    if (key === 'todo_max_items') {
      const cap = intValue(flags, key, -1);
      return `<span class="badge badge--medium" data-flag="${key}">${key} ${cap < 0 ? '∞' : cap}</span>`;
    }
    const on = isEnabled(flags, key);
    return `<span class="badge badge--${on ? 'low' : 'high'}" data-flag="${key}">${key} ${on ? 'on' : 'off'}</span>`;
  }).join('\n  ');
  return `<p class="todo-status" id="flag-strip" data-source="${flags.source}">
  identity ${identity} · flags from <strong>${escapeHtml(flags.source)}</strong>
  ${badges}
</p>`;
}

/** 503 body for a page: the app itself is switched off for this identity. */
export function renderDisabledPanel(identity: string | null): string {
  const who = identity ? escapeHtml(identity) : 'anonymous';
  return `<main class="todo-app" id="app-disabled">
  <header class="todo-app__header">
    <h1 class="todo-app__title">todo-web-astro is disabled</h1>
    <span class="todo-app__subtitle">feature flag</span>
  </header>
  <p class="todo-error" role="alert">
    todo-web-astro is disabled by feature flag <code>${APP_FLAG}</code> for identity <strong>${who}</strong>.
  </p>
  <p class="todo-status">
    Enable <code>${APP_FLAG}</code> in Flagsmith, or switch identity above.
  </p>
</main>`;
}

/** Standard fragment response (htmx swaps on 2xx only). */
export function fragment(html: string, status = 200): Response {
  return new Response(html, {
    status,
    headers: { 'content-type': 'text/html; charset=utf-8' },
  });
}

/** 503 fragment for endpoints while `todo_app_astro` is off. */
export const APP_DISABLED_FRAGMENT = `<p class="todo-error" role="alert">todo-web-astro is disabled by feature flag ${APP_FLAG}.</p>`;

/** 403 fragment for endpoints while `todo_write` is off. */
export const WRITE_DISABLED_FRAGMENT = `<p class="todo-error" role="alert">Writes are disabled by feature flag todo_write.</p>`;
