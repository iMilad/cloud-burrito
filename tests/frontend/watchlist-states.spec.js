import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, row, rowText, thing } from "./watchlist-harness.mjs";

test("a profile missing from the AWS config needs an account and is not requested", async ({ page }) => {
  const gone = { profile: "demo-gone", account_id: "acct-gone-fixture", region: "eu-west-1" };
  await bootWatchlist(page, { things: [thing("gone", "pipeline", gone, { pipeline_name: "x" })] });
  await expect(rowText(page, "gone")).toHaveText("needs account: profile is not in your AWS config");
  await expect(row(page, "gone").locator(".wl-row-state")).toHaveAttribute("data-tone", "warn");
  expect(await calls(page, "widget_fetch")).toEqual([]);
});

test("a failed first load says what to do, and r retries", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  // Arm the failure, then load the thing through the page's own exports.
  await page.evaluate(raw => {
    window.__watchlistHarness.fail("pipeline-runs");
    const W = window.CloudBurritoWatchlist;
    W.state.things = [window.CloudBurritoThings.normalize(raw)];
    W.renderList();
    W.loadRow(W.state.things[0]);
  }, thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" }));
  await expect(rowText(page, "p")).toHaveText("failed, press r to retry");
  await expect(row(page, "p").locator(".wl-row-state")).toHaveAttribute("data-tone", "fail");
  await row(page, "p").focus();
  await page.keyboard.press("r");
  await expect(rowText(page, "p")).toHaveText("Succeeded");
});
