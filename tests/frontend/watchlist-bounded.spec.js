import { expect, test } from "@playwright/test";
import { A, bootWatchlist, rowText, thing } from "./watchlist-harness.mjs";

test("a large stack search mounts at most 100 rows and its filter searches all of them", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("ss", "stacks-search", A, { name_prefix: "big-" })] });
  await page.evaluate(() => {
    window.__watchlistHarness.widgets["cfn-stacks"] = () => ({ render: "table", columns: ["stack", "status", "resources", "last_updated"],
      rows: Array.from({ length: 150 }, (_, n) => ({ stack: `big-${String(n).padStart(3, "0")}`, status: "CREATE_COMPLETE",
        resources: 1, last_updated: "2026-11-01T00:00:00Z" })) });
  });
  await page.keyboard.press("r");
  await expect(rowText(page, "ss")).toHaveText("150 stacks");
  await page.keyboard.press("Enter");
  const rows = page.locator(".wl-view-host tbody > tr[data-source-row]");
  await expect(rows).toHaveCount(100);
  await page.keyboard.press("G");
  await expect(page.locator(".wl-view-host tr.wl-current")).toContainText("big-099");
  await page.locator(".wl-view-host .table-filter").fill("big-149");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("big-149");
});
