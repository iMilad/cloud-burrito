import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, row, thing } from "./watchlist-harness.mjs";

const hostile = `<img src=x onerror="window.__pwned=1">`;

test("names, results and profiles render as text, never as HTML", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("x", "pipeline", A, { pipeline_name: hostile })] });
  await expect(row(page, "x").locator(".wl-row-name")).toHaveText(hostile);
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText(hostile);
  expect(await page.evaluate(() => window.__pwned)).toBeUndefined();
  expect(await page.locator("img[src='x']").count()).toBe(0);
});

test("the page makes no remote requests and never uses the inherited account path", async ({ page }) => {
  const remote = [];
  page.on("request", request => { if (!request.url().startsWith("http://127.0.0.1:4173")) remote.push(request.url()); });
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  expect(remote).toEqual([]);
  expect(await calls(page, "aws_set_account")).toEqual([]);
  expect(await calls(page, "dashboard_set")).toEqual([]);
  const fetches = await calls(page, "widget_fetch");
  expect(fetches.length).toBeGreaterThan(0);
  expect(fetches.every(c => c.params.context?.mode === "pinned" && typeof c.params.request_id === "string")).toBe(true);
});
