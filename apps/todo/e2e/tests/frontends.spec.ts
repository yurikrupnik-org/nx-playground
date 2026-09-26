// The same create → complete → delete loop on every frontend, through the
// one page object. Passing on all four surfaces is the proof that they share
// a DOM contract AND a backend: each spec also checks the row through
// todo-api's REST endpoint, which is what the other frontends read.
import { expect, test } from '@playwright/test';

import { TodoPage, URLS, uniqueTitle } from './stack';

const SURFACES = [
  {
    name: 'Solid SPA (client-rendered)',
    url: `${URLS.spa}/`,
    badge: 'SolidJS',
  },
  {
    name: 'Astro SSR — Solid island',
    url: `${URLS.astro}/solid`,
    badge: 'Solid island',
  },
  {
    name: 'Astro SSR — HTML + htmx',
    url: `${URLS.astro}/htmx`,
    badge: 'HTML + htmx',
  },
  { name: 'axum SSR — HTML + htmx', url: `${URLS.htmx}/`, badge: null },
] as const;

for (const surface of SURFACES) {
  test.describe(surface.name, () => {
    test('create, complete, delete a todo', async ({ page, request }) => {
      const title = uniqueTitle('ui');
      await page.goto(surface.url);
      await expect(page.locator('main.todo-app')).toBeVisible();
      if (surface.badge) {
        await expect(page.locator('.todo-app__subtitle')).toContainText(
          surface.badge,
        );
      }

      const todos = new TodoPage(page);
      await todos.add(title, 'high');
      await expect(todos.row(title).locator('.badge')).toHaveText('high');

      // The backend, not the DOM, is the source of truth.
      const created = await (
        await request.get(`${URLS.api}/api/todos?limit=100000`)
      ).json();
      const row = created.find((t: { title: string }) => t.title === title);
      expect(row, 'todo-api must hold the row the UI created').toBeDefined();
      expect(row.priority).toBe('high');
      expect(row.completed).toBe(false);

      await todos.toggle(title);
      await todos.expectDone(title, true);
      await expect
        .poll(
          async () =>
            (
              await (
                await request.get(`${URLS.api}/api/todos/${row.id}`)
              ).json()
            ).completed,
        )
        .toBe(true);

      await todos.toggle(title);
      await todos.expectDone(title, false);

      await todos.remove(title);
      await expect
        .poll(async () =>
          (await request.get(`${URLS.api}/api/todos/${row.id}`)).status(),
        )
        .toBe(404);
    });
  });
}

test('server-rendered pages carry the list in the initial HTML; the SPA does not', async ({
  request,
}) => {
  const title = uniqueTitle('ssr');
  const created = await request.post(`${URLS.api}/api/todos`, {
    data: { title, priority: 'low' },
  });
  expect(created.ok()).toBe(true);
  const { id } = await created.json();

  try {
    // No JavaScript runs here: `request` fetches raw documents.
    for (const url of [`${URLS.astro}/htmx`, `${URLS.htmx}/`]) {
      const html = await (await request.get(url)).text();
      expect(html, `${url} must server-render the row`).toContain(title);
      expect(html).toContain('id="todo-list"');
    }
    const shell = await (await request.get(`${URLS.spa}/`)).text();
    expect(shell, 'the SPA shell is empty until JS runs').not.toContain(title);
    expect(shell).toContain('<div id="root">');
  } finally {
    await request.delete(`${URLS.api}/api/todos/${id}`);
  }
});
