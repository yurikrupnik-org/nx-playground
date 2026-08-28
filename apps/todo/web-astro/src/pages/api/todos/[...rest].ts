// Same-origin pass-through to todo-api for the Solid island.
//
// Keeps the browser talking to one origin in dev AND under the built node
// server (no vite proxy at runtime). Bodies are buffered — todo payloads are
// tiny — and only content-type is forwarded each way.
import type { APIRoute } from 'astro';

import { API_ORIGIN } from '../../../lib/api';

export const ALL: APIRoute = async ({ params, request }) => {
  const search = new URL(request.url).search;
  const path = params.rest ? `/${params.rest}` : '';
  const hasBody = !['GET', 'HEAD'].includes(request.method);

  const upstream = await fetch(`${API_ORIGIN}/api/todos${path}${search}`, {
    method: request.method,
    headers: hasBody
      ? {
          'content-type':
            request.headers.get('content-type') ?? 'application/json',
        }
      : undefined,
    body: hasBody ? await request.arrayBuffer() : undefined,
  });

  // 204/205/304 forbid a body — Response() throws otherwise (DELETE is 204).
  const nullBody = [204, 205, 304].includes(upstream.status);
  return new Response(nullBody ? null : await upstream.arrayBuffer(), {
    status: upstream.status,
    headers: {
      'content-type':
        upstream.headers.get('content-type') ?? 'application/json',
    },
  });
};
