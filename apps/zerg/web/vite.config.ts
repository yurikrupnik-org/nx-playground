/// <reference types="vitest" />
/// <reference types="vite/client" />

import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import devtools from 'solid-devtools/vite';
import solidPlugin from 'vite-plugin-solid';
import { defineConfig } from 'vitest/config';

// `command` gates the `development` export condition: solid-js resolves
// `browser.development` to `dist/dev.js` (reactivity warnings, debug hooks), so
// applying it unconditionally shipped that dev runtime in production builds.
export default defineConfig(({ command }) => ({
  plugins: [devtools(), solidPlugin(), tailwindcss()],
  // base: '/web/',
  server: {
    port: 3000,
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
        headers: {
          'x-forwarded-host': 'localhost:3000',
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
    setupFiles: ['./vitest.setup.ts'],
    watch: false,
  },
  resolve: {
    conditions: command === 'serve' ? ['development', 'browser'] : ['browser'],
    alias: {
      '@contract/tasks': path.resolve(
        import.meta.dirname,
        '../../../libs/contracts/tasks/types/index.ts',
      ),
      '@ui/web-auth': path.resolve(
        import.meta.dirname,
        '../../../libs/ui/web-auth/src/index.ts',
      ),
    },
  },
}));
