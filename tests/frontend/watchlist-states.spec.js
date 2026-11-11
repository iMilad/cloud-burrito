import { expect, test } from "@playwright/test";
import { A, answer, bootWatchlist, calls, row, rowText, thing } from "./watchlist-harness.mjs";

test("a profile missing from the AWS config needs an account and is not requested", async ({ page }) => {
  const gone = { profile: "demo-gone", account_id: "acct-gone-fixture", region: "eu-west-1" };
  await bootWatchlist(page, { things: [thing("gone", "pipeline", gone, { pipeline_name: "x" })] });
  await expect(rowText(page, "gone")).toHaveText("needs account: profile is not in your AWS config, press e to choose a profile");
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

test("a profile that now points to another account says how to fix it", async ({ page }) => {
  const moved = { ...A, account_id: "acct-old-fixture" };
  await bootWatchlist(page, { things: [thing("moved", "pipeline", moved, { pipeline_name: "x" })] });
  await expect(rowText(page, "moved")).toHaveText("needs account: profile now points to another account, press e to choose a profile");
  expect(await calls(page, "widget_fetch")).toEqual([]);
});

test("an expired sign-in says y copies the login command, and y copies it", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: "http://127.0.0.1:4173" });
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  await expect(rowText(page, "p")).toHaveText("Succeeded");
  await answer(page, "pipeline-runs", { ok: false, error_type: "CredentialsExpired", error: "Synthetic expired session" });
  await page.keyboard.press("r");
  await expect(rowText(page, "p")).toHaveText("sign in needed, press y to copy the login command");
  await expect(row(page, "p").locator(".wl-row-state")).toHaveAttribute("data-tone", "warn");
  await page.keyboard.press("y");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("aws sso login --profile demo-a");
  await expect(page.locator("#wl-notice")).toHaveText("Copied aws sso login --profile demo-a");
});

test("the login command quotes a profile name with a space", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: "http://127.0.0.1:4173" });
  const spaced = { profile: "demo team", account_id: "acct-t-fixture", region: "eu-west-1" };
  await bootWatchlist(page, { profiles: [spaced], things: [thing("t", "pipeline", spaced, { pipeline_name: "synthetic-ok" })] });
  await expect(rowText(page, "t")).toHaveText("Succeeded");
  await answer(page, "pipeline-runs", { ok: false, needs_sso_login: true, error: "Synthetic sign-in needed" });
  await page.keyboard.press("r");
  await expect(rowText(page, "t")).toHaveText("sign in needed, press y to copy the login command");
  await page.keyboard.press("y");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("aws sso login --profile 'demo team'");
});
