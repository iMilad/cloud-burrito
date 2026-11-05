import { expect, test } from "@playwright/test";
import { bootWatchlist, calls, saved } from "./watchlist-harness.mjs";

const pinB = { profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" };
const tiles = [
  { id: "pipeline-runs", widget: "pipeline-runs", x: 0, y: 0, w: 4, h: 3, config: { inputs: { pinned_pipelines: [{ ...pinB, pipeline_name: "synthetic-ok" }] } } },
  { id: "codeartifact-packages", widget: "codeartifact-packages", x: 0, y: 3, w: 6, h: 4, config: { inputs: { domain: "d", repository: "r", package_prefix: "acme-core" } } },
  { id: "cfn-stacks", widget: "cfn-stacks", x: 6, y: 0, w: 6, h: 3, config: {} },
];

test("a first start converts old widgets once and never writes the old dashboard", async ({ page }) => {
  await bootWatchlist(page, { storage: "missing", tiles });
  await expect(page.locator("#wl-notice")).toHaveText("Converted 2 widgets into 2 things. Your old dashboard file was not changed.");
  await expect(page.locator(".wl-row")).toHaveCount(2);
  expect((await saved(page)).map(t => t.kind).sort()).toEqual(["package", "pipeline"]);
  expect(await calls(page, "dashboard_set")).toEqual([]);
  expect(await calls(page, "dashboard_get")).toHaveLength(1);
});

test("an existing watchlist is never replaced by a conversion", async ({ page }) => {
  await bootWatchlist(page, { storage: "loaded", things: [], tiles });
  expect(await calls(page, "dashboard_get")).toEqual([]);
  await expect(page.locator(".wl-empty")).toBeVisible();
});

test("inherited widgets without a default profile are skipped with a clear note", async ({ page }) => {
  await bootWatchlist(page, { storage: "missing", tiles: [tiles[1]], settings: { default_profile: "" } });
  await expect(page.locator("#wl-notice")).toContainText("1 widget needs an account");
  expect(await saved(page)).toEqual([]);
});

test("a conversion that is too big keeps the first 200 things and says what was left out", async ({ page }) => {
  const many = Array.from({ length: 205 }, (_, i) => ({ ...pinB, pipeline_name: `synthetic-pipe-${i}` }));
  const big = [{ id: "pipeline-runs", widget: "pipeline-runs", x: 0, y: 0, w: 4, h: 3, config: { inputs: { pinned_pipelines: many } } }];
  await bootWatchlist(page, { storage: "missing", tiles: big });
  await expect(page.locator("#wl-notice")).toContainText("5 more did not fit, because the watchlist holds at most 200 things.");
  const items = await saved(page);
  expect(items).toHaveLength(200);
  expect(items[0].inputs.pipeline_name).toBe("synthetic-pipe-0");
  expect(items[199].inputs.pipeline_name).toBe("synthetic-pipe-199");
});

test("a failed save after conversion shows the save error, not a success notice", async ({ page }) => {
  await bootWatchlist(page, { storage: "missing", tiles, failSave: true });
  await expect(page.locator("#wl-notice")).toHaveText("Changes could not be saved. The file on disk was not changed. Try again.");
  await expect(page.locator("#wl-notice")).not.toContainText("Converted");
  expect(await saved(page)).toEqual([]);
});
