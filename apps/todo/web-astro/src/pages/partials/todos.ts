// htmx fragment endpoint: list + create.
//
// POST consumes the form-encoded body htmx submits (title, priority) and
// answers with the refreshed <ul id="todo-list"> fragment.
//
// Flag-gated per identity: `todo_app_astro` off ⇒ 503, `todo_write` off ⇒ 403
// without touching upstream (todo-api would reject it anyway).
import type { CreateTodo, TodoPriority } from '@domain/todo';
import type { APIRoute } from 'astro';

import { flagContext, todoApi } from '../../lib/api';
import {
  APP_DISABLED_FRAGMENT,
  fragment,
  PRIORITIES,
  renderTodoList,
  WRITE_DISABLED_FRAGMENT,
} from '../../lib/fragments';

export const GET: APIRoute = async ({ cookies }) => {
  const { identity, appEnabled, canWrite } = await flagContext(cookies);
  if (!appEnabled) return fragment(APP_DISABLED_FRAGMENT, 503);
  return fragment(renderTodoList(await todoApi.list(identity), { canWrite }));
};

export const POST: APIRoute = async ({ cookies, request }) => {
  const { identity, appEnabled, canWrite } = await flagContext(cookies);
  if (!appEnabled) return fragment(APP_DISABLED_FRAGMENT, 503);
  if (!canWrite) return fragment(WRITE_DISABLED_FRAGMENT, 403);

  const form = await request.formData();
  const title = String(form.get('title') ?? '').trim();
  const priority = String(form.get('priority') ?? 'medium') as TodoPriority;
  if (!title || !PRIORITIES.includes(priority)) {
    return fragment('invalid todo', 422);
  }

  const input: CreateTodo = { title, description: '', priority };
  await todoApi.create(input, identity);
  return fragment(renderTodoList(await todoApi.list(identity), { canWrite }));
};
