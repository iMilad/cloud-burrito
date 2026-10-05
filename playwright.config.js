import { defineConfig, devices } from "@playwright/test";

if (process.env.CLOUD_BURRITO_CHROME_PATH) {
  throw new Error("The frontend gate requires Playwright's pinned Chromium. Unset CLOUD_BURRITO_CHROME_PATH and run: npx playwright install chromium");
}

export default defineConfig({
  testDir: "tests/frontend",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 2 : 3,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  timeout: 20_000,
  // Include runner/browser/server teardown in a finite gate. Never turn a
  // successful test count with a hanging worker into a successful exit.
  globalTimeout: 300_000,
  use: {
    baseURL: "http://127.0.0.1:4173",
    headless: true,
    screenshot: "only-on-failure",
    trace: "on-first-retry"
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1480, height: 920 }
      }
    }
  ],
  webServer: {
    command: "python3 scripts/serve-frontend-tests.py",
    reuseExistingServer: false,
    url: "http://127.0.0.1:4173"
  }
});
