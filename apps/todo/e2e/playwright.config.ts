// End-to-end suite for the todo vertical: ONE todo-api, every delivery option.
//
// Playwright owns the whole stack through `webServer`: a throwaway Postgres
// (docker), todo-api (cargo, migrations applied on its readiness path), and
// the three frontends, each on a dedicated port so a developer's normal
// `just run todo-api` / `bun run dev` on :8080 / :3100 / :3200 / :3300 is
// never reused or disturbed. Servers start in array order, each awaited.
//
// Tests run serially on purpose: the suite shares one database, one event bus
// and one api, and the realtime specs assert ordering across processes.
//
//   bun nx e2e todo-e2e            # or: cd apps/todo/e2e && bun run e2e
//   TODO_E2E_HEADED=1 …            # watch the browsers
import { defineConfig, devices } from '@playwright/test';

import { PORTS, STACK, URLS } from './tests/stack';

const root = new URL('../../../', import.meta.url).pathname;
const isCI = Boolean(process.env.CI);

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  workers: 1,
  retries: isCI ? 1 : 0,
  forbidOnly: isCI,
  reporter: isCI ? [['github'], ['list']] : 'list',
  timeout: 30_000,
  expect: { timeout: 10_000 },
  globalTeardown: './tests/teardown',
  use: {
    trace: 'retain-on-failure',
    headless: !process.env.TODO_E2E_HEADED,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: [
    {
      command: 'bash scripts/postgres.sh',
      port: PORTS.postgres,
      env: STACK.env,
      timeout: 120_000,
      reuseExistingServer: false,
      stdout: 'ignore',
      stderr: 'pipe',
    },
    {
      // Compiles on first run: the timeout is for a cold cargo build.
      command: 'bash scripts/todo-api.sh',
      url: `${URLS.api}/healthz`,
      env: {
        ...STACK.env,
        DATABASE_URL: STACK.databaseUrl,
        // No NATS in this suite: todo-api degrades to a no-op publisher and
        // a passthrough cache (see apps/todo/api/src/main.rs).
        NATS_URL: 'nats://127.0.0.1:1',
        APP_ENV: 'development',
        HOST: '127.0.0.1',
        PORT: String(PORTS.api),
        RUST_LOG: 'warn',
      },
      timeout: 600_000,
      reuseExistingServer: false,
      stdout: 'ignore',
      stderr: 'pipe',
    },
    {
      // The bins directly, not `bun run dev`: bun puts its child in a new
      // process group, which outlives Playwright's group kill (an orphaned
      // dev server holding the port fails the next run).
      command: `node_modules/.bin/vite --port ${PORTS.spa} --strictPort`,
      cwd: `${root}apps/todo/web`,
      url: URLS.spa,
      env: { ...STACK.env, TODO_API_URL: URLS.api },
      timeout: 120_000,
      reuseExistingServer: false,
      stdout: 'ignore',
      stderr: 'pipe',
    },
    {
      // The production build + the @astrojs/node standalone server, not
      // `astro dev`: Astro 7 daemonizes dev when it detects an AI-agent
      // environment (detached child, survives Playwright's teardown), and the
      // built server is what the image ships anyway. Needs the
      // `@native/field-selector` addon built (nx dependsOn).
      command: `node_modules/.bin/astro build && HOST=127.0.0.1 PORT=${PORTS.astro} node dist/server/entry.mjs`,
      cwd: `${root}apps/todo/web-astro`,
      url: `${URLS.astro}/health`,
      env: { ...STACK.env, TODO_API_URL: URLS.api },
      timeout: 120_000,
      reuseExistingServer: false,
      stdout: 'ignore',
      stderr: 'pipe',
    },
    {
      command: 'cargo run -q -p todo_web_htmx',
      cwd: root,
      url: `${URLS.htmx}/healthz`,
      env: {
        ...STACK.env,
        TODO_API_URL: URLS.api,
        TODO_WEB_HTMX_PORT: String(PORTS.htmx),
        HOST: '127.0.0.1',
        APP_ENV: 'development',
        RUST_LOG: 'warn',
      },
      timeout: 600_000,
      reuseExistingServer: false,
      stdout: 'ignore',
      stderr: 'pipe',
    },
  ],
});
