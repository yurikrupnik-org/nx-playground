// Liveness/readiness/startup probe endpoint.
//
// Deliberately dependency-free: it answers from the process alone, so a slow or
// down todo-api never gets this pod restarted or pulled out of the Service.
import type { APIRoute } from 'astro';

export const GET: APIRoute = () =>
  new Response(JSON.stringify({ status: 'ok' }), {
    status: 200,
    headers: {
      'content-type': 'application/json',
      'cache-control': 'no-store',
    },
  });
