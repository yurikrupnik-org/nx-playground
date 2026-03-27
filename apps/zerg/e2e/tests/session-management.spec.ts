import { test, expect } from "@playwright/test";
import { TEST_USER } from "../fixtures/test-user";

test.describe("Session management", () => {
  test("refresh preserves authentication", async ({ page }) => {
    await page.goto("/tasks");
    await expect(page).toHaveURL(/\/tasks/);

    // Hard refresh
    await page.reload();

    // Should still be on tasks (not redirected to login)
    await expect(page).toHaveURL(/\/tasks/);

    // API should still work
    const meRes = await page.request.get("/api/auth/me");
    expect(meRes.status()).toBe(200);
  });

  test("direct navigation to protected route works when authenticated", async ({
    page,
  }) => {
    // Create a task to get an ID
    const createRes = await page.request.post("/api/tasks", {
      data: {
        title: `Session test ${Date.now()}`,
        priority: "medium",
        status: "todo",
      },
    });
    const task = await createRes.json();

    // Navigate directly to task detail
    await page.goto(`/tasks/${task.id}`);
    await expect(page).toHaveURL(new RegExp(`/tasks/${task.id}`));

    // Should show the task, not login page
    await expect(page.locator("text=Loading...")).toBeHidden({
      timeout: 10000,
    });
    await expect(page.locator("h1")).toContainText("Session test");

    // Cleanup
    await page.request.delete(`/api/tasks/${task.id}`);
  });

  test("multiple sequential API calls maintain session", async ({ page }) => {
    await page.goto("/tasks");

    // Make multiple API calls in sequence
    for (let i = 0; i < 5; i++) {
      const res = await page.request.get("/api/auth/me");
      expect(res.status()).toBe(200);
    }
  });

  test("concurrent API calls don't corrupt session", async ({ page }) => {
    await page.goto("/tasks");

    // Fire 10 concurrent requests
    const requests = Array.from({ length: 10 }, () =>
      page.request.get("/api/auth/me")
    );

    const responses = await Promise.all(requests);

    // All should succeed
    for (const res of responses) {
      expect(res.status()).toBe(200);
      const body = await res.json();
      expect(body.user.email).toBe(TEST_USER.email);
    }
  });
});
