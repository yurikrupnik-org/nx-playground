// The claims docs/realtime-todo.md and docs/todo-delivery-options.md make,
// exercised across processes: a change made by ANY writer reaches the SPA
// without a reload, and every transport is a view of one backend.
import { expect, test } from '@playwright/test';

import { psql, runGrpcClient, TodoPage, URLS, uniqueTitle } from './stack';

test.describe('realtime (Postgres NOTIFY → SSE → SPA)', () => {
  test('a row inserted with psql appears in the open SPA and follows SQL updates', async ({
    page,
  }) => {
    const title = uniqueTitle('psql');
    await page.goto(`${URLS.spa}/`);
    const todos = new TodoPage(page);
    await expect(page.locator('ul.todo-list')).toBeAttached();

    // No app, no API, no HTTP: the most foreign writer there is.
    const id = await psql(
      `INSERT INTO todos (id, title, priority) VALUES (gen_random_uuid(), '${title}', 'high') RETURNING id`,
    );
    await expect(todos.row(title)).toBeVisible();
    await todos.expectDone(title, false);

    await psql(`UPDATE todos SET completed = true WHERE id = '${id}'`);
    await todos.expectDone(title, true);

    await psql(`DELETE FROM todos WHERE id = '${id}'`);
    await expect(todos.row(title)).toHaveCount(0);
  });

  test('a write on the htmx frontend reaches the SPA in another tab without a reload', async ({
    browser,
  }) => {
    const title = uniqueTitle('cross');
    const spa = await browser.newPage();
    const htmx = await browser.newPage();
    try {
      await spa.goto(`${URLS.spa}/`);
      await expect(spa.locator('ul.todo-list')).toBeAttached();

      await htmx.goto(`${URLS.htmx}/`);
      const htmxTodos = new TodoPage(htmx);
      await htmxTodos.add(title);

      const spaTodos = new TodoPage(spa);
      await expect(spaTodos.row(title)).toBeVisible();

      await spaTodos.remove(title);
      // The htmx page is request/response: it only learns on its next render.
      await htmx.reload();
      await expect(htmxTodos.row(title)).toHaveCount(0);
    } finally {
      await spa.close();
      await htmx.close();
    }
  });
});

test.describe('gRPC on the same port', () => {
  test('the grpc_client example completes against the REST/SSE listener and its events reach the SPA', async ({
    page,
  }) => {
    await page.goto(`${URLS.spa}/`);
    await expect(page.locator('ul.todo-list')).toBeAttached();

    const output = await runGrpcClient();
    expect(output).toContain(`connected ${URLS.api}`);
    expect(output).toMatch(/event Created .* snapshot=true/);
    expect(output).toMatch(/event Completed .* snapshot=true/);
    expect(output).toMatch(/event Deleted .* snapshot=false/);

    // The example deletes its own row; the SPA saw it come and go.
    await expect(
      page.locator('li.todo-item', { hasText: 'from grpc_client example' }),
    ).toHaveCount(0);
  });
});

test('the comparison landing page renders the DB-backed stack profiles', async ({
  page,
}) => {
  await page.goto(`${URLS.astro}/`);
  // Seeded by manifests/db/todo/migrations/20260827000000_stack_profiles.sql,
  // served by todo-api at /api/stacks, ordered cheapest first. The first
  // table is the DB-backed one; the `cheapest` badge only renders when rows
  // actually came back from Postgres.
  const verdict = page.getByRole('table').first();
  // Order matters: /api/stacks sorts by js_kb + html_kb ascending.
  await expect(verdict).toContainText(
    /Static HTML[\s\S]*Solid island[\s\S]*HTML \+ htmx/,
  );
  await expect(verdict.getByText('cheapest')).toBeVisible();
  await expect(verdict.getByText('default')).toBeVisible();
});
