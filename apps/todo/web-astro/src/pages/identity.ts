// Identity switcher endpoint.
//
// Backs the plain-HTML form in the flag strip: POST identity=<value>, the
// cookie is set (or cleared when blank/invalid), then a 303 sends the browser
// back to the page it came from. No JS involved, so it also works on the 503
// "app disabled" page.
import type { APIRoute } from 'astro';

import { setIdentityCookie } from '../lib/identity';

/** Same-origin path to return to; anything else falls back to `/`. */
function returnPath(request: Request): string {
  const referer = request.headers.get('referer');
  if (!referer) return '/';
  try {
    const url = new URL(referer, request.url);
    if (url.origin !== new URL(request.url).origin) return '/';
    return `${url.pathname}${url.search}`;
  } catch {
    return '/';
  }
}

export const POST: APIRoute = async ({ request, cookies }) => {
  const form = await request.formData();
  setIdentityCookie(cookies, String(form.get('identity') ?? ''));
  return new Response(null, {
    status: 303,
    headers: { location: returnPath(request) },
  });
};
