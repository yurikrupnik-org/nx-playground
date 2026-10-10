// The stack every spec runs against. Ports are deliberately off the dev
// defaults (8080 / 3100 / 3200 / 3300 / 5432) so the suite never collides
// with, or silently reuses, a developer's running servers.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { expect, type Locator, type Page } from '@playwright/test';

const exec = promisify(execFile);

export const PORTS = {
  postgres: 55433,
  api: 18090,
  spa: 3110,
  astro: 3210,
  htmx: 3310,
} as const;

export const URLS = {
  api: `http://127.0.0.1:${PORTS.api}`,
  spa: `http://127.0.0.1:${PORTS.spa}`,
  astro: `http://127.0.0.1:${PORTS.astro}`,
  htmx: `http://127.0.0.1:${PORTS.htmx}`,
} as const;

const PG_CONTAINER = 'todo-e2e-pg';

export const STACK = {
  container: PG_CONTAINER,
  databaseUrl: `postgres://todo:todo@127.0.0.1:${PORTS.postgres}/todo`,
  env: {
    TODO_E2E_PG_CONTAINER: PG_CONTAINER,
    TODO_E2E_PG_PORT: String(PORTS.postgres),
  },
} as const;

/** Unique, greppable title so concurrent leftovers never collide. */
export function uniqueTitle(tag: string): string {
  return `e2e-${tag}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

/**
 * Write straight to Postgres, bypassing every app: the realtime specs assert
 * that a change made by "any writer" reaches the browsers, and `psql` is the
 * most foreign writer available. Runs inside the container (no host psql).
 */
export async function psql(sql: string): Promise<string> {
  const { stdout } = await exec('docker', [
    'exec',
    '-i',
    PG_CONTAINER,
    'psql',
    '-qtA',
    '-v',
    'ON_ERROR_STOP=1',
    '-U',
    'todo',
    '-d',
    'todo',
    '-c',
    sql,
  ]);
  return stdout.trim();
}

/** The gRPC consumer shipped with todo-api, run against this suite's api. */
export async function runGrpcClient(): Promise<string> {
  const { stdout } = await exec(
    'cargo',
    ['run', '-q', '-p', 'todo_api', '--example', 'grpc_client', URLS.api],
    {
      cwd: new URL('../../../../', import.meta.url).pathname,
      timeout: 600_000,
    },
  );
  return stdout;
}

// ---------------------------------------------------------------------------
// The shared DOM contract. All three frontends render the same markup
// (libs/ui/todo-theme): `main.todo-app`, an add form whose input is labelled
// "new todo title", `ul.todo-list > li.todo-item` rows with a `toggle <title>`
// checkbox and a `delete <title>` button. One page object drives them all,
// which is itself the assertion that the contract holds.
// ---------------------------------------------------------------------------

export class TodoPage {
  constructor(readonly page: Page) {}

  row(title: string): Locator {
    return this.page.locator('li.todo-item', {
      has: this.page.locator('.todo-item__title', { hasText: title }),
    });
  }

  async add(title: string, priority: 'low' | 'medium' | 'high' = 'medium') {
    await this.page.getByLabel('new todo title').fill(title);
    await this.page.getByLabel('priority').selectOption(priority);
    await this.page.getByRole('button', { name: 'Add' }).click();
    await expect(this.row(title)).toBeVisible();
  }

  async toggle(title: string) {
    await this.page.getByLabel(`toggle ${title}`).click();
  }

  async remove(title: string) {
    await this.page.getByLabel(`delete ${title}`).click();
    await expect(this.row(title)).toHaveCount(0);
  }

  async expectDone(title: string, done: boolean) {
    const marker = this.row(title).locator('.todo-item__title');
    if (done) {
      await expect(marker).toHaveClass(/todo-item__title--done/);
    } else {
      await expect(marker).not.toHaveClass(/todo-item__title--done/);
    }
    await expect(this.page.getByLabel(`toggle ${title}`)).toBeChecked({
      checked: done,
    });
  }
}
