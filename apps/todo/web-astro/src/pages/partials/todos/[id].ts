// htmx fragment endpoint: delete a todo, answer with the refreshed list.
import type { APIRoute } from 'astro';

import { todoApi } from '../../../lib/api';
import { fragment, renderTodoList } from '../../../lib/fragments';

export const DELETE: APIRoute = async ({ params }) => {
  await todoApi.remove(params.id ?? '');
  return fragment(renderTodoList(await todoApi.list()));
};
