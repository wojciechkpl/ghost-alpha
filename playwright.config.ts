import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  timeout: 30_000,
  retries: 0,
  globalSetup: "./e2e/global-setup.ts",
  use: {
    baseURL: "http://127.0.0.1:3333",
    headless: true,
  },
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
  ],
  webServer: {
    command:
      "./target/release/ghost-alpha serve --port 3333 --bind 127.0.0.1",
    port: 3333,
    timeout: 10_000,
    reuseExistingServer: false,
    env: {
      GHOST_ALPHA_FINNHUB_API_KEY: "test-key-not-real",
      GHOST_ALPHA_DB_PATH: "e2e_test.db",
    },
  },
});
