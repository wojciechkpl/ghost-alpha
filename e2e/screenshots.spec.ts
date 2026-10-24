/**
 * Playwright script to capture dashboard screenshots for the README.
 *
 * Run:  npx playwright test e2e/screenshots.spec.ts
 *
 * Outputs PNGs to docs/screenshots/.
 */
import { test, expect } from "@playwright/test";
import { mkdirSync } from "fs";
import { join } from "path";

const SCREENSHOT_DIR = join(__dirname, "..", "docs", "screenshots");

test.beforeAll(() => {
  mkdirSync(SCREENSHOT_DIR, { recursive: true });
});

// Use a realistic desktop viewport for all screenshots
test.use({ viewport: { width: 1280, height: 800 } });

test.describe("Dashboard Screenshots", () => {
  test("home — screening results", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "01-home.png"),
      fullPage: true,
    });
  });

  test("alerts — alert history", async ({ page }) => {
    await page.goto("/alerts");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "02-alerts.png"),
      fullPage: true,
    });
  });

  test("watchlist — tracked tickers", async ({ page }) => {
    await page.goto("/watchlist");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "03-watchlist.png"),
      fullPage: true,
    });
  });

  test("regime — market regime", async ({ page }) => {
    await page.goto("/regime");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "04-regime.png"),
      fullPage: true,
    });
  });

  test("flagged — flagged stocks", async ({ page }) => {
    await page.goto("/flagged");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "05-flagged.png"),
      fullPage: true,
    });
  });

  test("strategies — available strategies", async ({ page }) => {
    await page.goto("/strategies");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "06-strategies.png"),
      fullPage: true,
    });
  });

  test("settings — configuration", async ({ page }) => {
    await page.goto("/settings");
    await expect(page.locator("h1")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "07-settings.png"),
      fullPage: true,
    });
  });

  test("analytics — charts and heatmaps", async ({ page }) => {
    await page.goto("/analytics");
    await expect(page.locator("h1")).toBeVisible();
    // Wait for htmx to load the chart partials
    await page.waitForTimeout(2000);
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "08-analytics.png"),
      fullPage: true,
    });
  });

  // ── Diversification pages ──────────────────────────────────────────

  test("allocation — fund mapping", async ({ page }) => {
    await page.goto("/allocation?brokerage=fidelity&profile=aggressive&portfolio_value=1000000");
    await expect(page.locator("h2")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "10-allocation.png"),
      fullPage: true,
    });
  });

  test("geo-risk — geopolitical risk", async ({ page }) => {
    await page.goto("/geo-risk?oil=130&vix=32&gold_3m=0.15&dxy_3m=-0.05");
    await expect(page.locator("h2")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "11-geo-risk.png"),
      fullPage: true,
    });
  });

  test("concentration-decay — volatility chart", async ({ page }) => {
    await page.goto("/concentration-decay?ticker=AMZN&shares=2000&price=215&other_value=80000&years=2");
    await expect(page.locator("h2")).toBeVisible();
    await page.screenshot({
      path: join(SCREENSHOT_DIR, "12-concentration-decay.png"),
      fullPage: true,
    });
  });
});
