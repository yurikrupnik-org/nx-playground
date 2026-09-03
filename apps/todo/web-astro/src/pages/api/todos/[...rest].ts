// Same-origin pass-through to todo-api for the Solid island.
//
// Keeps the browser talking to one origin in dev AND under the built node
// server (no vite proxy at runtime). Bodies are buffered — todo payloads are
// tiny — and only content-type is forwarded each way.
//
// On successful JSON reads the response is projected through
// `@native/field-selector` (Rust, via N-API): `?fields=` trims to a sparse
// fieldset and fields above the caller's role are dropped. Writes and error
// responses pass through untouched.
import type { APIRoute } from 'astro';

import { API_ORIGIN } from '../../../lib/api';
import {
  InvalidFieldsError,
  projectTodo,
  projectTodos,
} from '../../../lib/projection';

const JSON_HEADERS = { 'content-type': 'application/json' } as const;

export const ALL: APIRoute = async ({ params, request }) => {
  const url = new URL(request.url);
  const search = url.search;
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
  const contentType =
    upstream.headers.get('content-type') ?? 'application/json';

  // Only reads get projected: a write's response is the row the caller just
  // sent, and an error body is not a todo.
  const projectable =
    !nullBody &&
    upstream.ok &&
    request.method === 'GET' &&
    contentType.includes('application/json');

  if (!projectable) {
    return new Response(nullBody ? null : await upstream.arrayBuffer(), {
      status: upstream.status,
      headers: { 'content-type': contentType },
    });
  }

  const payload = await upstream.json();
  const fields = url.searchParams.get('fields');

  try {
    const projected = Array.isArray(payload)
      ? projectTodos(payload, fields)
      : projectTodo(payload, fields);
    return Response.json(projected, { status: upstream.status });
  } catch (cause) {
    if (cause instanceof InvalidFieldsError) {
      // The Rust schema rejected a field name; that is the caller's mistake.
      return new Response(JSON.stringify({ error: cause.message }), {
        status: 400,
        headers: JSON_HEADERS,
      });
    }
    throw cause;
  }
};
