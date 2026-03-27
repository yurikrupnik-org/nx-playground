import { test, expect } from "@playwright/test";
import { uniqueUser } from "../fixtures/test-user";

test.describe("Unauthenticated flows", () => {
  test("protected route redirects to login", async ({ page }) => {
    await page.goto("/tasks");
    await page.waitForURL("**/login", { timeout: 5000 });
  });

  test("deep link to task detail redirects to login", async ({ page }) => {
    await page.goto("/tasks/00000000-0000-0000-0000-000000000000");
    await page.waitForURL("**/login", { timeout: 5000 });
  });

  test("register with valid data succeeds", async ({ page }) => {
    const user = uniqueUser("reg");
    await page.goto("/register");

    await page.locator('input[placeholder="Your name"]').fill(user.name);
    await page.locator('input[placeholder="your@email.com"]').fill(user.email);
    await page.locator('input[placeholder="Create a password"]').fill(user.password);
    await page.locator('input[placeholder="Confirm your password"]').fill(user.password);

    // Password requirements should all pass
    const requirementsBox = page.locator(".bg-muted");
    if (await requirementsBox.isVisible()) {
      // All checks should show checkmarks
      const checks = requirementsBox.locator("text=✓");
      await expect(checks).toHaveCount(5);
    }

    await page.locator('button[type="submit"]').click();

    // Should redirect to tasks after successful registration
    await page.waitForURL("**/tasks", { timeout: 10000 });
  });

  test("register with weak password shows validation", async ({ page }) => {
    await page.goto("/register");

    await page.locator('input[placeholder="Your name"]').fill("Test");
    await page.locator('input[placeholder="your@email.com"]').fill("weak@test.local");
    await page.locator('input[placeholder="Create a password"]').fill("abc");
    await page.locator('input[placeholder="Confirm your password"]').fill("abc");

    // Password requirements should show failures
    const requirementsBox = page.locator(".bg-muted");
    if (await requirementsBox.isVisible()) {
      const failures = requirementsBox.locator("text=○");
      expect(await failures.count()).toBeGreaterThan(0);
    }

    // Submit should be disabled or show error
    await page.locator('button[type="submit"]').click();

    // Should stay on register page
    await expect(page).toHaveURL(/\/register/);
  });

  test("register with mismatched passwords shows error", async ({ page }) => {
    await page.goto("/register");

    await page.locator('input[placeholder="Your name"]').fill("Test");
    await page.locator('input[placeholder="your@email.com"]').fill("mismatch@test.local");
    await page.locator('input[placeholder="Create a password"]').fill("Test1234!@#$");
    await page.locator('input[placeholder="Confirm your password"]').fill("Different1234!@#$");

    // Should show password mismatch warning
    await expect(page.locator(".text-red-600")).toBeVisible();
  });

  test("login with wrong credentials shows error", async ({ page }) => {
    await page.goto("/login");

    await page.locator('input[type="email"]').fill("wrong@test.local");
    await page.locator('input[type="password"]').fill("WrongPass123!");
    await page.locator('button[type="submit"]').click();

    // Error message should appear
    await expect(page.locator(".bg-red-50")).toBeVisible({ timeout: 5000 });

    // Should stay on login page
    await expect(page).toHaveURL(/\/login/);
  });

  test("login form prevents empty submission", async ({ page }) => {
    await page.goto("/login");

    // Click submit with empty fields
    await page.locator('button[type="submit"]').click();

    // Browser validation or app validation should prevent navigation
    await expect(page).toHaveURL(/\/login/);
  });

  test("XSS payloads in login fields are handled safely", async ({ page }) => {
    await page.goto("/login");

    const xssPayload = '<script>alert("xss")</script>';
    await page.locator('input[type="email"]').fill(xssPayload);
    await page.locator('input[type="password"]').fill(xssPayload);
    await page.locator('button[type="submit"]').click();

    // No alert should fire, no script execution
    // Page should show error, not execute script
    await expect(page).toHaveURL(/\/login/);

    // Verify no injected script tags in DOM
    const scriptTags = await page.locator('script:text("xss")').count();
    expect(scriptTags).toBe(0);
  });
});
