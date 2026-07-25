/// <reference types="vitest" />
/// <reference types="vite/client" />

import tailwindcss from '@tailwindcss/vite';
import devtools from 'solid-devtools/vite';
import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

export default defineConfig({
  plugins: [devtools(), solidPlugin(), tailwindcss()],
  server: {
    port: 3001,
    proxy: {
      // BFF: the browser only ever talks to the terran api; auth cookies stay first-party.
      '/api': {
        target: 'http://localhost:8081',
        changeOrigin: true,
        headers: {
          'x-forwarded-host': 'localhost:3001',
          'x-forwarded-proto': 'http',
        },
      },
    },
  },
  build: {
    target: 'esnext',
  },
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
  resolve: {
    conditions: ['development', 'browser'],
  },
});
