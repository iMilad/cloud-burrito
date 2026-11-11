import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, rowText, thing } from "./watchlist-harness.mjs";

const pipe = thing("p-1", "pipeline", A, { pipeline_name: "synthetic-ok" });

test("the saved theme applies at start and a preference save does not reload things", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe], settings: { theme: "light" } });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(rowText(page, "p-1")).toHaveText("Succeeded");
  const before = (await calls(page, "widget_fetch")).length;
  await page.locator("#settings-btn").click();
  await page.locator("#settings-default-region").selectOption("us-east-1");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  expect((await calls(page, "settings_set"))[0].params.default_region).toBe("us-east-1");
  expect((await calls(page, "widget_fetch")).length).toBe(before);
});

test("missing settings metadata fails closed and offers a retry", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe], settingsMetadata: false });
  await expect(page.locator("#settings-storage-warning")).toContainText("Settings could not be loaded");
  await expect(page.locator("#wl-notice")).toHaveText("Settings could not be loaded. Open Settings to retry.");
  expect(await calls(page, "settings_set")).toEqual([]);
});

test("load-time field errors show, and a rejected save marks its field", async ({ page }) => {
  await bootWatchlist(page, { things: [], settingsMetadata: { field_errors: { theme: "Choose light or dark theme" } } });
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-theme-error")).toHaveText("Choose light or dark theme");
  await page.evaluate(() => { window.__watchlistHarness.rejectSettings = true; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes were rejected");
  await expect(page.locator("#settings-default-region")).toHaveAttribute("aria-invalid", "true");
});
