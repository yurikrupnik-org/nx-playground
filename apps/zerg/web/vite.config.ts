/// <reference types="vitest" />
/// <reference types="vite/client" />

import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import devtools from 'solid-devtools/vite';
import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

export default defineConfig({
  plugins: [devtools(), solidPlugin(), tailwindcss()],
  // base: '/web/',
  server: {
    port: 3000,
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
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
  // test: {
  //   watch: false,
  //   globals: true,
  //   environment: 'jsdom',
  //   include: ['src/**/*.{test,spec}.{js,mjs,cjs,ts,mts,cts,jsx,tsx}'],
  //   reporters: ['default'],
  //   coverage: {
  //     reportsDirectory: '../../../coverage/apps/playground/my-solid-app',
  //     provider: 'v8',
  //   },
  // },
  resolve: {
    conditions: ['development', 'browser'],
    alias: {
      '@domain/tasks': path.resolve(
        __dirname,
        '../../../libs/domains/tasks/types/index.ts',
      ),
    },
  },
});
