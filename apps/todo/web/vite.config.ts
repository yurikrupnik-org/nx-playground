import path from 'node:path';
import solidPlugin from '@solidjs/vite-plugin';
import { defineConfig } from 'vitest/config';

export default defineConfig({
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
    watch: false,
  },
  resolve: {
    conditions: ['development', 'browser'],
    alias: {
      '@domain/todo': path.resolve(
        import.meta.dirname,
        '../../../libs/domains/todo/types/index.ts',
      ),
    },
  },
});
