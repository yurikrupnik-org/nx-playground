import path from 'node:path';
import solidPlugin from '@solidjs/vite-plugin';
import { defineConfig } from 'vitest/config';

// `command` gates the `development` export condition: solid-js resolves
// `browser.development` to `dist/dev.js` (reactivity warnings, debug hooks), so
// applying it unconditionally shipped that dev runtime in production builds.
export default defineConfig(({ command }) => ({
  plugins: [solidPlugin()],
  server: {
    port: 3100,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
        // Required for the WebSocket example (/api/events/ws).
        ws: true,
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
      '@domain/todo': path.resolve(
        import.meta.dirname,
        '../../../libs/domains/todo/types/index.ts',
      ),
    },
  },
}));
