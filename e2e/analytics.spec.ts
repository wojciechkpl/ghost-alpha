import { test, expect } from "@playwright/test";

// ── Analytics Page ──────────────────────────────────────────────────────────

test.describe("Analytics Page", () => {
  test("loads with heading", async ({ page }) => {
    await page.goto("/analytics");
    await expect(page.locator("h1")).toContainText("Analytics");
  });

  test("has all chart section headings", async ({ page }) => {
    await page.goto("/analytics");
    await expect(page.locator("h2")).toContainText(["Score", "Sentiment", "Grade", "RS"]);
  });

  test("score heatmap loads via htmx", async ({ page }) => {
    await page.goto("/analytics");
    // Wait for htmx to load the partial
    const heatmap = page.locator("#score-heatmap");
    await expect(heatmap).toBeVisible();
    // Should contain SVG after htmx loads (seeded data has 3 days × 5 tickers)
    await page.waitForFunction(
      () => document.querySelector("#score-heatmap svg") !== null,
      { timeout: 5000 }
    );
    await expect(heatmap.locator("svg")).toBeVisible();
  });

  test("sentiment heatmap loads via htmx", async ({ page }) => {
    await page.goto("/analytics");
    const heatmap = page.locator("#sentiment-heatmap");
    await page.waitForFunction(
      () => document.querySelector("#sentiment-heatmap svg") !== null,
      { timeout: 5000 }
    );
    await expect(heatmap.locator("svg")).toBeVisible();
  });

  test("grade distribution loads via htmx", async ({ page }) => {
    await page.goto("/analytics");
    const chart = page.locator("#grade-dist");
    await page.waitForFunction(
      () => document.querySelector("#grade-dist svg") !== null,
      { timeout: 5000 }
    );
    await expect(chart.locator("svg")).toBeVisible();
  });

  test("RS scatter plot loads via htmx", async ({ page }) => {
    await page.goto("/analytics");
    const chart = page.locator("#rs-scatter");
    await page.waitForFunction(
      () => document.querySelector("#rs-scatter svg") !== null,
      { timeout: 5000 }
    );
    await expect(chart.locator("svg")).toBeVisible();
  });
});

// ── Analytics Partial Endpoints ─────────────────────────────────────────────

test.describe("Analytics Partials", () => {
  test("/partials/score-heatmap returns SVG", async ({ request }) => {
    const resp = await request.get("/partials/score-heatmap?days=7");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("<svg");
    expect(body).not.toContain("<!DOCTYPE html>");
  });

  test("/partials/sentiment-heatmap returns SVG", async ({ request }) => {
    const resp = await request.get("/partials/sentiment-heatmap?days=7");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("<svg");
  });

  test("/partials/grade-dist returns SVG", async ({ request }) => {
    const resp = await request.get("/partials/grade-dist?days=7");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("<svg");
  });

  test("/partials/rs-scatter returns SVG", async ({ request }) => {
    const resp = await request.get("/partials/rs-scatter");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("<svg");
  });

  test("/partials/alert-calendar returns content", async ({ request }) => {
    const resp = await request.get("/partials/alert-calendar?weeks=4");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    // Should have SVG (seeded alert data) or empty-state message
    expect(body.length).toBeGreaterThan(0);
  });

  test("/partials/regime-history returns content", async ({ request }) => {
    const resp = await request.get("/partials/regime-history?days=30");
    expect(resp.status()).toBe(200);
    const body = await resp.text();
    expect(body).toContain("<svg");
  });

  test("/partials/macro-gauges returns content", async ({ request }) => {
    const resp = await request.get("/partials/macro-gauges");
    expect(resp.status()).toBe(200);
    // May be empty-state (no FRED data seeded) — just verify 200
  });
});

// ── Enhanced Existing Pages ─────────────────────────────────────────────────

test.describe("Enhanced Pages", () => {
  test("alerts page has calendar heatmap section", async ({ page }) => {
    await page.goto("/alerts");
    const calendarDiv = page.locator("#alert-calendar");
    await expect(calendarDiv).toBeVisible();
  });

  test("regime page has history strip section", async ({ page }) => {
    await page.goto("/regime");
    const historyDiv = page.locator("#regime-history");
    await expect(historyDiv).toBeVisible();
  });

  test("regime page has macro gauges section", async ({ page }) => {
    await page.goto("/regime");
    const gaugesDiv = page.locator("#macro-gauges");
    await expect(gaugesDiv).toBeVisible();
  });
});

// ── Score Heatmap Interactivity ─────────────────────────────────────────────

test.describe("Heatmap Interactivity", () => {
  test("score heatmap contains ticker labels from seeded data", async ({
    page,
  }) => {
    await page.goto("/analytics");
    await page.waitForFunction(
      () => document.querySelector("#score-heatmap svg") !== null,
      { timeout: 5000 }
    );
    const svg = page.locator("#score-heatmap svg");
    const content = await svg.innerHTML();
    expect(content).toContain("NVDA");
    expect(content).toContain("AAPL");
  });

  test("score heatmap has tooltip titles", async ({ page }) => {
    await page.goto("/analytics");
    await page.waitForFunction(
      () => document.querySelector("#score-heatmap svg title") !== null,
      { timeout: 5000 }
    );
    const titles = page.locator("#score-heatmap svg title");
    expect(await titles.count()).toBeGreaterThan(0);
  });
});
