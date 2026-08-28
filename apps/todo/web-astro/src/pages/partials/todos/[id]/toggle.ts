// htmx fragment endpoint: flip completion, answer with the refreshed list.
//
// The checkbox posts here without client state; the current completion flag
// is read from the API so the toggle is idempotent per rendered state.
import type { APIRoute } from 'astro';

import { todoApi } from '../../../../lib/api';
import { fragment, renderTodoList } from '../../../../lib/fragments';

export const POST: APIRoute = async ({ params }) => {
  const id = params.id ?? '';
  const todos = await todoApi.list();
  const todo = todos.find((t) => t.id === id);
  if (!todo) return fragment('unknown todo', 404);

  await todoApi.toggle(id, todo.completed);
  return fragment(renderTodoList(await todoApi.list()));
};
