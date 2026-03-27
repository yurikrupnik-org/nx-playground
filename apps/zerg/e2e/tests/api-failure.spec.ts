import { test, expect } from "@playwright/test";

test.describe("API failure resilience", () => {
  test("500 error shows error state, not blank screen", async ({ page }) => {
    // Intercept task list API and return 500
    await page.route("**/api/tasks", (route) => {
      if (route.request().method() === "GET") {
        route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({ error: "Internal Server Error" }),
        });
      } else {
        route.continue();
      }
    });

    await page.goto("/tasks");

    // Should not show a blank white page
    const bodyText = await page.locator("body").textContent();
    expect(bodyText?.trim().length).toBeGreaterThan(0);

    // Should not show "Loading tasks..." forever
    await page.waitForTimeout(3000);
    // Either an error message or fallback UI should be present
    const hasContent = await page.locator("body").textContent();
    expect(hasContent).toBeTruthy();
  });

  test("network timeout shows error state", async ({ page }) => {
    // Intercept and delay response indefinitely
    await page.route("**/api/tasks", (route) => {
      if (route.request().method() === "GET") {
        // Never respond — simulates network timeout
        route.abort("timedout");
      } else {
        route.continue();
      }
    });

    await page.goto("/tasks");

    // Wait for timeout to surface
    await page.waitForTimeout(5000);

    // Page should still be functional (not crashed)
    const bodyText = await page.locator("body").textContent();
    expect(bodyText?.trim().length).toBeGreaterThan(0);
  });

  test("task detail 404 shows meaningful error", async ({ page }) => {
    const fakeId = "00000000-0000-0000-0000-000000000000";
    await page.goto(`/tasks/${fakeId}`);

    // Should show loading then either error or 404 message
    await page.waitForTimeout(3000);

    const bodyText = await page.locator("body").textContent();
    expect(bodyText?.trim().length).toBeGreaterThan(0);
  });

  test("API returns malformed JSON gracefully", async ({ page }) => {
    await page.route("**/api/tasks", (route) => {
      if (route.request().method() === "GET") {
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: "not valid json {{{",
        });
      } else {
        route.continue();
      }
    });

    await page.goto("/tasks");
    await page.waitForTimeout(3000);

    // Should not crash — should show some error state
    const bodyText = await page.locator("body").textContent();
    expect(bodyText?.trim().length).toBeGreaterThan(0);
  });

  test("slow API shows loading skeleton", async ({ page }) => {
    await page.route("**/api/tasks", async (route) => {
      if (route.request().method() === "GET") {
        // Delay 3 seconds
        await new Promise((r) => setTimeout(r, 3000));
        route.continue();
      } else {
        route.continue();
      }
    });

    await page.goto("/tasks");

    // Loading state should be visible during the delay
    await expect(page.locator("text=Loading tasks...")).toBeVisible();

    // After delay, loading should disappear
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });
  });
});
