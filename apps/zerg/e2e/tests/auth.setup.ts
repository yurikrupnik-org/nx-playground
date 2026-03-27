import { test as setup, expect } from "@playwright/test";
import { TEST_USER } from "../fixtures/test-user";

const authFile = "apps/zerg/e2e/.auth/user.json";

setup("authenticate", async ({ page }) => {
  // Register the test user (ignore if already exists)
  const registerRes = await page.request.post("/api/auth/register", {
    data: {
      email: TEST_USER.email,
      password: TEST_USER.password,
      name: TEST_USER.name,
    },
  });
  // 200 = new user, 409 = already exists — both are fine
  expect([200, 409]).toContain(registerRes.status());

  // Login via UI to get cookies set in browser context
  await page.goto("/login");
  await page.locator('input[type="email"]').fill(TEST_USER.email);
  await page.locator('input[type="password"]').fill(TEST_USER.password);
  await page.locator('button[type="submit"]').click();

  // Wait for redirect to /tasks (proves auth succeeded)
  await page.waitForURL("**/tasks", { timeout: 10000 });

  // Save signed-in state
  await page.context().storageState({ path: authFile });
});
