// htmx fragment endpoint: list + create.
//
// POST consumes the form-encoded body htmx submits (title, priority) and
// answers with the refreshed <ul id="todo-list"> fragment.
import type { CreateTodo, TodoPriority } from '@domain/todo';
import type { APIRoute } from 'astro';

import { todoApi } from '../../lib/api';
import { fragment, PRIORITIES, renderTodoList } from '../../lib/fragments';

export const GET: APIRoute = async () =>
  fragment(renderTodoList(await todoApi.list()));

export const POST: APIRoute = async ({ request }) => {
  const form = await request.formData();
  const title = String(form.get('title') ?? '').trim();
  const priority = String(form.get('priority') ?? 'medium') as TodoPriority;
  if (!title || !PRIORITIES.includes(priority)) {
    return fragment('invalid todo', 422);
  }

  const input: CreateTodo = { title, description: '', priority };
  await todoApi.create(input);
  return fragment(renderTodoList(await todoApi.list()));
};
