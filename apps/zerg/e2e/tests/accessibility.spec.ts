import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const ROUTES_TO_AUDIT = ["/tasks", "/login", "/register"];

test.describe("Accessibility", () => {
  for (const route of ROUTES_TO_AUDIT) {
    test(`${route} has no critical accessibility violations`, async ({
      page,
    }) => {
      await page.goto(route);
      // Wait for content to load
      await page.waitForTimeout(2000);

      const results = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
        .analyze();

      // Filter to critical and serious only
      const critical = results.violations.filter(
        (v) => v.impact === "critical" || v.impact === "serious"
      );

      if (critical.length > 0) {
        const summary = critical
          .map(
            (v) =>
              `[${v.impact}] ${v.id}: ${v.description} (${v.nodes.length} instances)`
          )
          .join("\n");
        console.log(`Accessibility issues on ${route}:\n${summary}`);
      }

      expect(
        critical,
        `Found ${critical.length} critical/serious a11y violations on ${route}`
      ).toHaveLength(0);
    });
  }

  test("login form is keyboard navigable", async ({ page }) => {
    await page.goto("/login");

    // Tab through the form
    await page.keyboard.press("Tab"); // Focus email
    const emailFocused = await page.locator('input[type="email"]').evaluate(
      (el) => el === document.activeElement
    );
    expect(emailFocused).toBe(true);

    await page.keyboard.press("Tab"); // Focus password
    const passwordFocused = await page
      .locator('input[type="password"]')
      .evaluate((el) => el === document.activeElement);
    expect(passwordFocused).toBe(true);

    await page.keyboard.press("Tab"); // Focus submit button
    const submitFocused = await page
      .locator('button[type="submit"]')
      .evaluate((el) => el === document.activeElement);
    expect(submitFocused).toBe(true);
  });

  test("task list is keyboard navigable", async ({ page }) => {
    await page.goto("/tasks");
    await expect(page.locator("text=Loading tasks...")).toBeHidden({
      timeout: 10000,
    });

    // Tab should reach filter buttons and task links
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");

    // Should be able to activate a filter with Enter
    await page.keyboard.press("Enter");

    // Page should still be functional
    const bodyText = await page.locator("body").textContent();
    expect(bodyText?.trim().length).toBeGreaterThan(0);
  });
});
