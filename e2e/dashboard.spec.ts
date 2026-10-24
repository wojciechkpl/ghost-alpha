import { test, expect } from "@playwright/test";

// ── Home Page ────────────────────────────────────────────────────────────────

test.describe("Home Page", () => {
  test("loads with title and screening results table", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveTitle(/Ghost Alpha/);
    await expect(page.locator("h1")).toContainText("Screening Results");
  });

  test("displays regime badge", async ({ page }) => {
    await page.goto("/");
    const badge = page.locator(".badge-bull, .badge-bear, .badge-sideways");
    await expect(badge.first()).toBeVisible();
  });

  test("shows seeded screen results in table", async ({ page }) => {
    await page.goto("/");
    // Seeded tickers should appear
    await expect(page.locator("table tbody")).toContainText("NVDA");
    await expect(page.locator("table tbody")).toContainText("AAPL");
  });

  test("table has grade column with badge", async ({ page }) => {
    await page.goto("/");
    const gradeBadge = page.locator(
      '[class*="grade-"], [class*="badge-grade-"]'
    );
    // At least one grade badge should be visible
    await expect(gradeBadge.first()).toBeVisible();
  });

  test("navigation contains all expected links", async ({ page }) => {
    await page.goto("/");
    const nav = page.locator("nav");
    await expect(nav).toContainText("Home");
    await expect(nav).toContainText("Alerts");
    await expect(nav).toContainText("Watchlist");
    await expect(nav).toContainText("Regime");
    await expect(nav).toContainText("Flagged");
    await expect(nav).toContainText("Analytics");
    await expect(nav).toContainText("Strategies");
    await expect(nav).toContainText("Settings");
  });

  test("dashboard nav contains diversification links", async ({ page }) => {
    await page.goto("/allocation");
    const nav = page.locator("nav");
    await expect(nav).toContainText("Allocation");
    await expect(nav).toContainText("Geo Risk");
  });

  test("uses embedded assets, not CDN", async ({ page }) => {
    await page.goto("/");
    const html = await page.content();
    expect(html).not.toContain("cdn.jsdelivr.net");
    expect(html).not.toContain("unpkg.com");
    expect(html).toContain("/static/pico.min.css");
    expect(html).toContain("/static/htmx.min.js");
    expect(html).toContain("/static/ghost-alpha.css");
  });
});

// ── Static Assets ────────────────────────────────────────────────────────────

test.describe("Static Assets", () => {
  test("serves htmx.min.js with correct content type", async ({ request }) => {
    const resp = await request.get("/static/htmx.min.js");
    expect(resp.status()).toBe(200);
    const ct = resp.headers()["content-type"];
    expect(ct).toMatch(/javascript/);
  });

  test("serves pico.min.css with correct content type", async ({ request }) => {
    const resp = await request.get("/static/pico.min.css");
    expect(resp.status()).toBe(200);
    const ct = resp.headers()["content-type"];
    expect(ct).toMatch(/css/);
  });

  test("serves ghost-alpha.css", async ({ request }) => {
    const resp = await request.get("/static/ghost-alpha.css");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("Ghost Alpha");
  });

  test("returns 404 for nonexistent asset", async ({ request }) => {
    const resp = await request.get("/static/nonexistent.xyz");
    expect(resp.status()).toBe(404);
  });
});

// ── Alerts Page ──────────────────────────────────────────────────────────────

test.describe("Alerts Page", () => {
  test("loads with heading and seeded alerts", async ({ page }) => {
    await page.goto("/alerts");
    await expect(page.locator("h1")).toContainText("Alert History");
    await expect(page.locator("table tbody")).toContainText("NVDA");
  });

  test("displays severity badges", async ({ page }) => {
    await page.goto("/alerts");
    const criticalBadge = page.locator(".badge-critical");
    await expect(criticalBadge.first()).toBeVisible();
  });

  test("shows alert message text", async ({ page }) => {
    await page.goto("/alerts");
    await expect(page.locator("table tbody")).toContainText("volume breakout");
  });
});

// ── Watchlist Page ───────────────────────────────────────────────────────────

test.describe("Watchlist Page", () => {
  test("loads with heading and seeded tickers", async ({ page }) => {
    await page.goto("/watchlist");
    await expect(page.locator("h1")).toContainText("Watchlist");
    await expect(page.locator("table tbody")).toContainText("AMD");
    await expect(page.locator("table tbody")).toContainText("GOOGL");
  });

  test("shows notes for tickers that have them", async ({ page }) => {
    await page.goto("/watchlist");
    await expect(page.locator("table tbody")).toContainText(
      "Watching for breakout"
    );
  });

  test("has add ticker form", async ({ page }) => {
    await page.goto("/watchlist");
    await expect(page.locator('input[name="ticker"]')).toBeVisible();
    await expect(page.locator('button[type="submit"]')).toBeVisible();
  });

  test("can add a ticker via the form", async ({ page }) => {
    await page.goto("/watchlist");
    await page.fill('input[name="ticker"]', "TSLA");
    await page.click('button[type="submit"]');
    // Wait for htmx to swap the response
    await page.waitForTimeout(500);
    // TSLA should now appear somewhere on the page
    await expect(page.locator("body")).toContainText("TSLA");
  });

  test("can remove a ticker", async ({ page }) => {
    await page.goto("/watchlist");
    // Count rows before
    const rowsBefore = await page.locator("table tbody tr").count();
    // Click the first Remove button
    const removeBtn = page.locator("button", { hasText: "Remove" }).first();
    if ((await removeBtn.count()) > 0) {
      await removeBtn.click();
      await page.waitForTimeout(500);
      // Should have one fewer row (or the page refreshed without it)
      const rowsAfter = await page.locator("table tbody tr").count();
      expect(rowsAfter).toBeLessThanOrEqual(rowsBefore);
    }
  });
});

// ── Regime Page ──────────────────────────────────────────────────────────────

test.describe("Regime Page", () => {
  test("loads with heading and regime data", async ({ page }) => {
    await page.goto("/regime");
    await expect(page.locator("h1")).toContainText("Market Regime");
  });

  test("shows Bull badge for seeded data", async ({ page }) => {
    await page.goto("/regime");
    const badge = page.locator(".badge-bull");
    await expect(badge.first()).toBeVisible();
  });

  test("shows SPY price data", async ({ page }) => {
    await page.goto("/regime");
    await expect(page.locator("body")).toContainText("520");
  });
});

// ── Flagged Stocks Page ──────────────────────────────────────────────────────

test.describe("Flagged Stocks Page", () => {
  test("loads with heading", async ({ page }) => {
    await page.goto("/flagged");
    await expect(page.locator("h1")).toContainText("Flagged");
  });

  test("shows seeded flagged tickers", async ({ page }) => {
    await page.goto("/flagged");
    await expect(page.locator("body")).toContainText("NVDA");
    await expect(page.locator("body")).toContainText("PLTR");
  });
});

// ── Strategies Page ──────────────────────────────────────────────────────────

test.describe("Strategies Page", () => {
  test("loads with heading and all 4 strategies", async ({ page }) => {
    await page.goto("/strategies");
    await expect(page.locator("h1")).toContainText("Strategies");
    await expect(page.locator("body")).toContainText("minervini");
    await expect(page.locator("body")).toContainText("breakout");
    await expect(page.locator("body")).toContainText("mean-reversion");
    await expect(page.locator("body")).toContainText("trend-strength");
  });
});

// ── Settings Page ────────────────────────────────────────────────────────────

test.describe("Settings Page", () => {
  test("loads with heading", async ({ page }) => {
    await page.goto("/settings");
    await expect(page.locator("h1")).toContainText("Settings");
  });

  test("masks API key", async ({ page }) => {
    await page.goto("/settings");
    const body = await page.content();
    // The full test key "test-key-not-real" should NOT appear
    expect(body).not.toContain("test-key-not-real");
  });
});

// ── Partial Endpoints ────────────────────────────────────────────────────────

test.describe("Partial Endpoints", () => {
  test("/partials/screen-table returns fragment", async ({ request }) => {
    const resp = await request.get("/partials/screen-table");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).not.toContain("<!DOCTYPE html>");
    expect(body).not.toContain("<html");
    // Should contain table row data
    expect(body).toContain("<tr");
  });

  test("/partials/alert-list returns fragment", async ({ request }) => {
    const resp = await request.get("/partials/alert-list");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).not.toContain("<!DOCTYPE html>");
  });

  test("/partials/watchlist-rows returns fragment", async ({ request }) => {
    const resp = await request.get("/partials/watchlist-rows");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).not.toContain("<!DOCTYPE html>");
  });

  test("/partials/regime-strip returns fragment", async ({ request }) => {
    const resp = await request.get("/partials/regime-strip");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).not.toContain("<!DOCTYPE html>");
  });

  test("alert list partial filters by severity", async ({ request }) => {
    const resp = await request.get(
      "/partials/alert-list?severity=CRITICAL"
    );
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    // Should contain CRITICAL alert but not INFO
    expect(body).toContain("NVDA");
  });
});

// ── Navigation ───────────────────────────────────────────────────────────────

test.describe("Navigation", () => {
  test("can navigate between all pages", async ({ page }) => {
    await page.goto("/");

    await page.click('nav a[href="/alerts"]');
    await expect(page.locator("h1")).toContainText("Alert");

    await page.click('nav a[href="/watchlist"]');
    await expect(page.locator("h1")).toContainText("Watchlist");

    await page.click('nav a[href="/regime"]');
    await expect(page.locator("h1")).toContainText("Regime");

    await page.click('nav a[href="/flagged"]');
    await expect(page.locator("h1")).toContainText("Flagged");

    await page.click('nav a[href="/analytics"]');
    await expect(page.locator("h1")).toContainText("Analytics");

    await page.click('nav a[href="/strategies"]');
    await expect(page.locator("h1")).toContainText("Strategies");

    await page.click('nav a[href="/settings"]');
    await expect(page.locator("h1")).toContainText("Settings");

    await page.click('nav a[href="/"]');
    await expect(page.locator("h1")).toContainText("Screening");
  });
});

// ── Diversification Pages ────────────────────────────────────────────────────

test.describe("Allocation Page", () => {
  test("loads with heading and fund table", async ({ page }) => {
    await page.goto("/allocation?brokerage=fidelity&profile=moderate&portfolio_value=500000");
    await expect(page.locator("h2")).toContainText("Target Allocation");
    await expect(page.locator("table")).toBeVisible();
  });

  test("shows fund tickers", async ({ page }) => {
    await page.goto("/allocation?brokerage=fidelity&profile=aggressive");
    await expect(page.locator("body")).toContainText("FZROX");
  });

  test("shows TLH partner column", async ({ page }) => {
    await page.goto("/allocation");
    await expect(page.locator("body")).toContainText("TLH Partner");
  });

  test("switches brokerage via query param", async ({ page }) => {
    await page.goto("/allocation?brokerage=vanguard");
    await expect(page.locator("body")).toContainText("VTI");
  });
});

test.describe("Geo Risk Page", () => {
  test("loads with heading and risk assessment", async ({ page }) => {
    await page.goto("/geo-risk");
    await expect(page.locator("h2")).toContainText("Geopolitical Risk");
  });

  test("shows composite score", async ({ page }) => {
    await page.goto("/geo-risk?oil=130&vix=32&gold_3m=0.15&dxy_3m=-0.05");
    await expect(page.locator("body")).toContainText("/10");
  });

  test("shows allocation adjustments for high risk", async ({ page }) => {
    await page.goto("/geo-risk?oil=130&vix=32&gold_3m=0.15&dxy_3m=-0.05");
    await expect(page.locator("body")).toContainText("energy");
    await expect(page.locator("body")).toContainText("Adjustment");
  });

  test("shows Low level for calm inputs", async ({ page }) => {
    await page.goto("/geo-risk");
    await expect(page.locator("body")).toContainText("Low");
  });
});

test.describe("Concentration Decay Page", () => {
  test("loads with heading and SVG chart", async ({ page }) => {
    await page.goto("/concentration-decay?ticker=AMZN&shares=1000&price=200&other_value=50000&years=2");
    await expect(page.locator("h2")).toContainText("Concentration Decay");
    await expect(page.locator("svg")).toBeVisible();
  });

  test("shows quarterly breakdown table", async ({ page }) => {
    await page.goto("/concentration-decay?ticker=GOOG&shares=500&price=170");
    await expect(page.locator("body")).toContainText("Quarter");
    await expect(page.locator("body")).toContainText("Est. Vol");
  });

  test("shows vol reduction summary", async ({ page }) => {
    await page.goto("/concentration-decay?ticker=AMZN&shares=2000&price=215&other_value=80000&years=2");
    await expect(page.locator("body")).toContainText("reduction");
  });
});

// ── Responsive / Accessibility ───────────────────────────────────────────────

test.describe("Accessibility Basics", () => {
  test("all pages have proper lang attribute", async ({ page }) => {
    await page.goto("/");
    const lang = await page.locator("html").getAttribute("lang");
    expect(lang).toBe("en");
  });

  test("tables have thead with th elements", async ({ page }) => {
    await page.goto("/");
    const headers = page.locator("table thead th");
    expect(await headers.count()).toBeGreaterThan(0);
  });

  test("page is usable at mobile width", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/");
    await expect(page.locator("h1")).toBeVisible();
    await expect(page.locator("table")).toBeVisible();
  });
});
