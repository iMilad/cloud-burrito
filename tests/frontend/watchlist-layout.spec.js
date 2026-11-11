import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, row, thing } from "./watchlist-harness.mjs";

// Each row owns its requests through two helper elements. Core marks a
// request host as loading while it runs, but rows show their state in
// .wl-row-state, so the helpers must never take space or show a status.
test("loaded rows stay one line and never show a loading status or Cancel button", async ({ page }) => {
  await bootWatchlist(page, { things: [
    thing("p-1", "pipeline", A, { pipeline_name: "synthetic-ok" }, 0),
    thing("k-1", "package", B, { domain: "d", repository: "r", package_prefix: "acme-core" }, 1),
  ] });
  for (const id of ["p-1", "k-1"]) {
    await expect(row(page, id)).toHaveAttribute("data-loading", "false");
    const helpers = row(page, id).locator(".wl-row-owner, .wl-row-owner-detail");
    await expect(helpers).toHaveCount(2);
    for (const helper of await helpers.all()) await expect(helper).toBeHidden();
    await expect(row(page, id).getByRole("button")).toHaveCount(0);
    const box = await row(page, id).boundingBox();
    expect(box.height, `row ${id} height`).toBeLessThanOrEqual(48);
  }
});
