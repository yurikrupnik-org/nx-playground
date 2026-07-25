/// <reference types="vitest" />
/// <reference types="vite/client" />

import path from 'node:path';
import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solidPlugin()],
  server: {
    port: 3100,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
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
        __dirname,
        '../../../libs/domains/todo/types/index.ts',
      ),
    },
  },
});
