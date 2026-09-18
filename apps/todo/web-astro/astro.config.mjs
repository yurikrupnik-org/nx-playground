// Astro SSR host for the todo comparison app.
//
// - `output: 'server'`: every page/endpoint renders per-request. The htmx
//   variant depends on this (fragment endpoints under /partials), and the
//   Solid variant uses the /api/todos pass-through proxy.
// - Node standalone adapter so `astro build` yields a runnable server.
import node from '@astrojs/node';
import solid from '@astrojs/solid-js';
import { defineConfig } from 'astro/config';

export default defineConfig({
  output: 'server',
  adapter: node({ mode: 'standalone' }),
  integrations: [solid()],
  server: { port: 3200 },
  vite: {
    ssr: {
      // N-API addon: rollup cannot inline a `.node` binary, and it must stay a
      // runtime `require` resolved from node_modules. Server-only by nature —
      // nothing here reaches the browser bundle.
      external: ['@native/field-selector'],
    },
  },
});
