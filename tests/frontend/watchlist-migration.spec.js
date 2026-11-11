import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, saved, thing } from "./watchlist-harness.mjs";

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
  await expect(page.locator("#wl-notice")).toHaveText("1 widget needs an account. Set a default profile in Settings, then add it with a.");
  // The empty list is written, so the next start does not convert or show this note again.
  expect(await calls(page, "watchlist_set")).toHaveLength(1);
  expect(await saved(page)).toEqual([]);
});

test("a default profile missing from the AWS config gets its own note", async ({ page }) => {
  await bootWatchlist(page, { storage: "missing", tiles: [tiles[1]], settings: { default_profile: "demo-gone" } });
  await expect(page.locator("#wl-notice")).toHaveText("1 widget needs an account. The default profile demo-gone is not in your AWS config."
    + " Choose another default profile in Settings, then add it with a.");
  expect(await saved(page)).toEqual([]);
});

test("a default profile without a supported region gets its own note", async ({ page }) => {
  const far = { profile: "demo-far", account_id: "acct-far-fixture", region: "region-unsupported-fixture" };
  await bootWatchlist(page, { storage: "missing", tiles: [tiles[1], tiles[1]], profiles: [far],
    settings: { default_profile: "demo-far", default_region: "" } });
  await expect(page.locator("#wl-notice")).toHaveText("2 widgets need an account. The default profile demo-far has no supported region."
    + " Set a default region in Settings, then add them with a.");
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

const UNREADABLE = "Your watchlist could not be read, so changes are not saved. Fix or move the file, then restart the app.";

test("an unreadable watchlist is never overwritten by the add form", async ({ page }) => {
  await bootWatchlist(page, { storage: "failed", things: [] });
  await expect(page.locator(".wl-empty h2")).toHaveText("Your watchlist could not be read");
  await page.keyboard.press("a");
  await expect(page.locator("#wl-notice")).toHaveText(UNREADABLE);
  await expect(page.locator("#wl-form")).toBeHidden();
  // Even a form that is already open refuses to save.
  await page.evaluate(() => {
    const W = window.CloudBurritoWatchlist;
    W.state.storageReady = true;
    W.openForm(null);
    W.state.storageReady = false;
  });
  const form = page.locator("#wl-form");
  await form.getByLabel("Pipeline name").fill("synthetic-new");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(page.locator("#wl-notice")).toHaveText(UNREADABLE);
  expect(await calls(page, "watchlist_set")).toEqual([]);
});

test("an unreadable watchlist is never overwritten by s on a command result", async ({ page }) => {
  await bootWatchlist(page, { storage: "failed", things: [] });
  await expect(page.locator(".wl-empty h2")).toHaveText("Your watchlist could not be read");
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-x @demo-a");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  await page.keyboard.press("s");
  await expect(page.locator("#wl-notice")).toHaveText(UNREADABLE);
  expect(await calls(page, "watchlist_set")).toEqual([]);
});

test("nothing can be added while the first conversion runs, so it is never lost", async ({ page }) => {
  const booted = bootWatchlist(page, { storage: "missing", tiles, holds: [{ name: "dashboard_get" }] });
  await page.waitForFunction(() => window.__watchlistHarness?.calls.some(c => c.command === "dashboard_get"));
  await page.keyboard.press("a");
  await expect(page.locator("#wl-notice")).toHaveText(UNREADABLE);
  await expect(page.locator("#wl-form")).toBeHidden();
  await page.evaluate(() => window.__watchlistHarness.release("dashboard_get"));
  await booted;
  await expect(page.locator(".wl-row")).toHaveCount(2);
  expect(await calls(page, "watchlist_set")).toHaveLength(1);
});

test("saved things the app cannot read stay in the file, and the note says so", async ({ page }) => {
  const one = thing("p-1", "pipeline", A, { pipeline_name: "synthetic-one" }, 0);
  const two = thing("p-2", "pipeline", A, { pipeline_name: "synthetic-two" }, 1);
  const unreadable = thing("p-bad", "pipeline", A, { pipeline_name: "" }, 2);
  await bootWatchlist(page, { things: [one, two, unreadable] });
  await expect(page.locator("#wl-notice")).toHaveText("1 saved thing could not be read. It stays in your watchlist file, but is not shown.");
  await expect(page.locator(".wl-row")).toHaveCount(2);
  await page.keyboard.press("Shift+J");
  await expect.poll(async () => (await calls(page, "watchlist_set")).length).toBe(1);
  const stored = await saved(page);
  expect(stored).toHaveLength(3);
  expect(stored.find(t => t.id === "p-bad")).toEqual(unreadable);
});

test("saved things the app cannot read still count toward the 200 limit", async ({ page }) => {
  const things = Array.from({ length: 199 }, (_, i) => thing(`p-${i}`, "pipeline", A, { pipeline_name: `synthetic-${i}` }, i));
  await bootWatchlist(page, { things: [...things, thing("p-bad", "pipeline", A, { pipeline_name: "" }, 199)] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Pipeline name").fill("synthetic-extra");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(page.locator("#wl-form-error")).toHaveText("The watchlist is full (200 things). Remove one, then try again.");
  expect(await calls(page, "watchlist_set")).toEqual([]);
});
