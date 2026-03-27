import { test, expect } from "@playwright/test";

test.describe("Tasks CRUD", () => {
  let createdTaskTitle: string;

  test.beforeEach(async ({ page }) => {
    // Create a task via API for tests that need existing data
    createdTaskTitle = `E2E Task ${Date.now()}`;
    await page.request.post("/api/tasks", {
      data: {
        title: createdTaskTitle,
        description: "Created by Playwright E2E",
        priority: "high",
        status: "todo",
      },
    });
  });

  test("tasks list page loads and shows tasks", async ({ page }) => {
    await page.goto("/tasks");

    // Wait for loading to finish
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    // Should see the task we created
    await expect(page.locator(`text=${createdTaskTitle}`)).toBeVisible();
  });

  test("filter buttons filter tasks by status", async ({ page }) => {
    // Create tasks with different statuses via API
    const todoTitle = `Filter-todo-${Date.now()}`;
    const doneTitle = `Filter-done-${Date.now()}`;

    await page.request.post("/api/tasks", {
      data: { title: todoTitle, status: "todo", priority: "medium" },
    });
    await page.request.post("/api/tasks", {
      data: { title: doneTitle, status: "done", priority: "medium" },
    });

    await page.goto("/tasks");
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    // Click "To Do" filter
    await page.locator("button").filter({ hasText: "To Do" }).click();
    await expect(page.locator(`text=${todoTitle}`)).toBeVisible();
    // Done task should be hidden
    await expect(page.locator(`text=${doneTitle}`)).toBeHidden();

    // Click "Done" filter
    await page.locator("button").filter({ hasText: "Done" }).click();
    await expect(page.locator(`text=${doneTitle}`)).toBeVisible();
    await expect(page.locator(`text=${todoTitle}`)).toBeHidden();

    // Click "All" to reset
    await page.locator("button").filter({ hasText: "All" }).click();
    await expect(page.locator(`text=${todoTitle}`)).toBeVisible();
    await expect(page.locator(`text=${doneTitle}`)).toBeVisible();
  });

  test("click task navigates to detail page", async ({ page }) => {
    await page.goto("/tasks");
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    await page.locator("a").filter({ hasText: createdTaskTitle }).click();

    // Should navigate to /tasks/:id
    await expect(page).toHaveURL(/\/tasks\/[0-9a-f-]+/);

    // Detail page should show the title
    await expect(page.locator("h1")).toContainText(createdTaskTitle);
  });

  test("edit task inline on detail page", async ({ page }) => {
    // Get the task ID
    const listRes = await page.request.get("/api/tasks");
    const tasks = await listRes.json();
    const task = tasks.find(
      (t: { title: string }) => t.title === createdTaskTitle
    );

    await page.goto(`/tasks/${task.id}`);
    await expect(page.locator("text=Loading...")).toBeHidden({
      timeout: 10000,
    });

    // Click Edit
    await page.locator("button").filter({ hasText: "Edit" }).click();

    // Modify title
    const updatedTitle = `Updated ${createdTaskTitle}`;
    const titleInput = page.locator('input[type="text"]');
    await titleInput.clear();
    await titleInput.fill(updatedTitle);

    // Change priority to urgent
    const prioritySelect = page.locator("select").last();
    await prioritySelect.selectOption("urgent");

    // Change status to in_progress
    const statusSelect = page.locator("select").first();
    await statusSelect.selectOption("in_progress");

    // Save
    await page.locator("button").filter({ hasText: "Save" }).click();

    // Wait for save to complete (button text changes back from "Saving...")
    await expect(
      page.locator("button").filter({ hasText: "Edit" })
    ).toBeVisible({ timeout: 5000 });

    // Verify the update persisted
    await expect(page.locator("h1")).toContainText(updatedTitle);

    // Verify via API
    const getRes = await page.request.get(`/api/tasks/${task.id}`);
    const updated = await getRes.json();
    expect(updated.title).toBe(updatedTitle);
    expect(updated.priority).toBe("urgent");
    expect(updated.status).toBe("in_progress");
  });

  test("cancel edit discards changes", async ({ page }) => {
    const listRes = await page.request.get("/api/tasks");
    const tasks = await listRes.json();
    const task = tasks.find(
      (t: { title: string }) => t.title === createdTaskTitle
    );

    await page.goto(`/tasks/${task.id}`);
    await expect(page.locator("text=Loading...")).toBeHidden({
      timeout: 10000,
    });

    // Edit and change title
    await page.locator("button").filter({ hasText: "Edit" }).click();
    const titleInput = page.locator('input[type="text"]');
    await titleInput.clear();
    await titleInput.fill("SHOULD NOT PERSIST");

    // Cancel
    await page.locator("button").filter({ hasText: "Cancel" }).click();

    // Title should revert to original
    await expect(page.locator("h1")).toContainText(createdTaskTitle);
  });

  test("delete task removes it from list", async ({ page }) => {
    await page.goto("/tasks");
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    // Find and delete our task
    const taskCard = page
      .locator("a")
      .filter({ hasText: createdTaskTitle })
      .locator("..");
    await taskCard.locator("button").filter({ hasText: "Delete" }).click();

    // Task should disappear from the list
    await expect(
      page.locator("a").filter({ hasText: createdTaskTitle })
    ).toBeHidden({ timeout: 5000 });

    // Verify via API
    const listRes = await page.request.get("/api/tasks");
    const tasks = await listRes.json();
    const found = tasks.find(
      (t: { title: string }) => t.title === createdTaskTitle
    );
    expect(found).toBeUndefined();
  });

  test("rapid create and delete maintains consistency", async ({ page }) => {
    const titles: string[] = [];

    // Rapid-fire create 5 tasks via API
    for (let i = 0; i < 5; i++) {
      const title = `Rapid-${i}-${Date.now()}`;
      titles.push(title);
      await page.request.post("/api/tasks", {
        data: { title, priority: "low", status: "todo" },
      });
    }

    await page.goto("/tasks");
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    // All 5 should be visible
    for (const title of titles) {
      await expect(page.locator(`text=${title}`)).toBeVisible();
    }

    // Delete all 5 via API
    const listRes = await page.request.get("/api/tasks");
    const allTasks = await listRes.json();
    for (const title of titles) {
      const task = allTasks.find((t: { title: string }) => t.title === title);
      if (task) {
        await page.request.delete(`/api/tasks/${task.id}`);
      }
    }

    // Reload and verify all gone
    await page.reload();
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    for (const title of titles) {
      await expect(page.locator(`text=${title}`)).toBeHidden();
    }
  });

  test("back button on detail page returns to list", async ({ page }) => {
    const listRes = await page.request.get("/api/tasks");
    const tasks = await listRes.json();
    const task = tasks.find(
      (t: { title: string }) => t.title === createdTaskTitle
    );

    await page.goto(`/tasks/${task.id}`);
    await expect(page.locator("text=Loading...")).toBeHidden({
      timeout: 10000,
    });

    await page
      .locator("button")
      .filter({ hasText: /Back to Tasks/ })
      .click();

    await expect(page).toHaveURL(/\/tasks$/);
  });
});
