import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, thing } from "./watchlist-harness.mjs";

const pipe = thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" });
const view = page => page.locator(".wl-view-host");
const crumb = page => page.locator(".wl-crumb");

test("a held build log that arrives after Escape does not change the run view", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(view(page).locator(".exec-action-block")).toHaveCount(2);
  await page.evaluate(() => window.__watchlistHarness.hold("codebuild-log"));
  await page.keyboard.press("Enter");
  await expect(crumb(page)).toContainText("Build log");
  await expect.poll(async () => (await calls(page, "widget_fetch", "codebuild-log")).length).toBe(1);
  await page.keyboard.press("Escape");
  await expect(crumb(page)).not.toContainText("Build log");
  await page.evaluate(() => window.__watchlistHarness.release("codebuild-log"));
  await page.waitForTimeout(200);
  await expect(view(page).locator(".exec-action-block")).toHaveCount(2);
  await expect(page.locator(".log-line")).toHaveCount(0);
  await expect(crumb(page)).toContainText("run exec-0042");
});

test("a held execution detail that arrives after Escape does not change the run list", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await page.keyboard.press("Enter");
  await expect(view(page).locator(".wl-item")).toHaveCount(2);
  await page.evaluate(() => window.__watchlistHarness.hold("pipeline-execution-detail"));
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await calls(page, "widget_fetch", "pipeline-execution-detail")).length).toBe(1);
  await page.keyboard.press("Escape");
  await expect(crumb(page)).not.toContainText("run exec");
  await page.evaluate(() => window.__watchlistHarness.release("pipeline-execution-detail"));
  await page.waitForTimeout(200);
  await expect(view(page).locator(".wl-item")).toHaveCount(2);
  await expect(page.locator(".exec-action-block")).toHaveCount(0);
});

test("leaving a filtered stack search terminates its filter worker", async ({ page }) => {
  await page.addInitScript(() => {
    const NativeWorker = window.Worker;
    const counts = window.__workers = { started: 0, terminated: 0 };
    window.Worker = function (...args) {
      counts.started++;
      const worker = new NativeWorker(...args);
      const terminate = worker.terminate.bind(worker);
      let stopped = false;
      worker.terminate = () => { if (!stopped) counts.terminated++; stopped = true; terminate(); };
      return worker;
    };
  });
  await bootWatchlist(page, { things: [thing("ss", "stacks-search", A, { name_prefix: "big-" }), thing("other", "stacks-search", A, { name_prefix: "other-" }, 1)] });
  await page.evaluate(() => {
    window.__watchlistHarness.widgets["cfn-stacks"] = () => ({ render: "table", columns: ["stack", "status", "resources", "last_updated"],
      rows: Array.from({ length: 150 }, (_, n) => ({ stack: `big-${String(n).padStart(3, "0")}`, status: "CREATE_COMPLETE",
        resources: 1, last_updated: "2026-11-01T00:00:00Z" })) });
  });
  const live = () => page.evaluate(() => window.__workers.started - window.__workers.terminated);
  await page.keyboard.press("r");
  await page.keyboard.press("Enter");
  const filter = page.locator(".wl-view-host .table-filter");
  await filter.fill("big-14");
  await expect(filter).toHaveAttribute("data-filter-pending", "false");
  await expect(page.locator(".wl-view-host tbody > tr[data-source-row]")).toHaveCount(10);
  expect(await live()).toBe(1);
  // Escape leaves the text field, then the inspector; j and Enter open another thing.
  await page.keyboard.press("Escape");
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  await expect(crumb(page)).toContainText("other-");
  await expect.poll(live).toBe(0);
  expect(await page.evaluate(() => window.__workers.started)).toBe(1);
});
