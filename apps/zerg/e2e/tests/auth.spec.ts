import { test, expect } from "@playwright/test";
import { TEST_USER, uniqueUser } from "../fixtures/test-user";

test.describe("Authentication", () => {
  test("shows current user on /me after login", async ({ page }) => {
    // Already authenticated via setup — just verify
    await page.goto("/tasks");
    await expect(page).toHaveURL(/\/tasks/);

    // The user menu or layout should reflect the logged-in user
    const meRes = await page.request.get("/api/auth/me");
    expect(meRes.status()).toBe(200);
    const body = await meRes.json();
    expect(body.user.email).toBe(TEST_USER.email);
  });

  test("logout invalidates session", async ({ page, context }) => {
    await page.goto("/tasks");
    await expect(page).toHaveURL(/\/tasks/);

    // Click logout in user menu
    await page.locator("button").filter({ hasText: /logout/i }).click();

    // Should redirect to login
    await page.waitForURL("**/login", { timeout: 5000 });

    // API calls should now return 401
    const meRes = await page.request.get("/api/auth/me");
    expect(meRes.status()).toBe(401);
  });

  test("re-login after logout works", async ({ page }) => {
    await page.goto("/tasks");

    // Logout
    await page.locator("button").filter({ hasText: /logout/i }).click();
    await page.waitForURL("**/login");

    // Login again
    await page.locator('input[type="email"]').fill(TEST_USER.email);
    await page.locator('input[type="password"]').fill(TEST_USER.password);
    await page.locator('button[type="submit"]').click();

    await page.waitForURL("**/tasks", { timeout: 10000 });
    await expect(page).toHaveURL(/\/tasks/);
  });
});
