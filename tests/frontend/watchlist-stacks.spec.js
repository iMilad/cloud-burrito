import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, row, rowText, thing } from "./watchlist-harness.mjs";

const stack = thing("s-1", "stack", B, { stack_name: "synthetic-stack" });
const search = thing("ss-1", "stacks-search", A, { name_prefix: "synthetic-" });
const lookup = thing("lk-1", "lookup", A, { query: "synthetic-fn" });

test("a stack row shows its own stack status and the inspector shows resources and events", async ({ page }) => {
  await bootWatchlist(page, { things: [stack] });
  await expect(rowText(page, "s-1")).toHaveText("Update rollback complete");
  await expect(row(page, "s-1").locator(".wl-row-state")).toHaveAttribute("data-tone", "fail");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tab", { name: /Resources/ })).toBeVisible();
  await page.getByRole("tab", { name: /Events/ }).click();
  // core.formatStatusLabel renders the status in capitals, so ignore case.
  await expect(page.locator(".stack-events-table")).toContainText(/update rollback complete/i);
});

test("a stack search lists stacks and Enter opens one in the same account", async ({ page }) => {
  await bootWatchlist(page, { things: [search] });
  await expect(rowText(page, "ss-1")).toHaveText("2 stacks");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view-host tbody tr[data-source-row]")).toHaveCount(2);
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb strong")).toHaveText("synthetic-two");
  const detail = (await calls(page, "widget_fetch", "cfn-stack-detail"))[0];
  expect(detail.params.inputs).toEqual({ stack_name: "synthetic-two" });
  expect(detail.params.context.profile).toBe("demo-a");
});

test("y copies the ARN of the selected lookup match", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: "http://127.0.0.1:4173" });
  await bootWatchlist(page, { things: [lookup] });
  await expect(rowText(page, "lk-1")).toHaveText("1 match");
  await page.keyboard.press("Enter");
  await expect(page.locator(".lookup-row")).toHaveCount(1);
  await page.keyboard.press("y");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe("arn:aws:lambda:eu-west-1:acct-a-fixture:function:synthetic-fn");
});
