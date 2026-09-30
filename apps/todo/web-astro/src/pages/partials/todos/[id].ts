// htmx fragment endpoint: delete a todo, answer with the refreshed list.
//
// Flag-gated per identity: `todo_app_astro` off ⇒ 503, `todo_write` off ⇒ 403
// without touching upstream.
import type { APIRoute } from 'astro';

import { flagContext, todoApi } from '../../../lib/api';
import {
  APP_DISABLED_FRAGMENT,
  fragment,
  renderTodoList,
  WRITE_DISABLED_FRAGMENT,
} from '../../../lib/fragments';

export const DELETE: APIRoute = async ({ cookies, params }) => {
  const { identity, appEnabled, canWrite } = await flagContext(cookies);
  if (!appEnabled) return fragment(APP_DISABLED_FRAGMENT, 503);
  if (!canWrite) return fragment(WRITE_DISABLED_FRAGMENT, 403);

  await todoApi.remove(params.id ?? '', identity);
  return fragment(renderTodoList(await todoApi.list(identity), { canWrite }));
};
