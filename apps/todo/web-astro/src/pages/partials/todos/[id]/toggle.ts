// htmx fragment endpoint: flip completion, answer with the refreshed list.
//
// The checkbox posts here without client state; the current completion flag
// is read from the API so the toggle is idempotent per rendered state.
//
// Flag-gated per identity: `todo_app_astro` off ⇒ 503, `todo_write` off ⇒ 403
// without touching upstream.
import type { APIRoute } from 'astro';

import { flagContext, todoApi } from '../../../../lib/api';
import {
  APP_DISABLED_FRAGMENT,
  fragment,
  renderTodoList,
  WRITE_DISABLED_FRAGMENT,
} from '../../../../lib/fragments';

export const POST: APIRoute = async ({ cookies, params }) => {
  const { identity, appEnabled, canWrite } = await flagContext(cookies);
  if (!appEnabled) return fragment(APP_DISABLED_FRAGMENT, 503);
  if (!canWrite) return fragment(WRITE_DISABLED_FRAGMENT, 403);

  const id = params.id ?? '';
  const todos = await todoApi.list(identity);
  const todo = todos.find((t) => t.id === id);
  if (!todo) return fragment('unknown todo', 404);

  await todoApi.toggle(id, todo.completed, identity);
  return fragment(renderTodoList(await todoApi.list(identity), { canWrite }));
};
