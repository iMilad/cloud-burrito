import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, thing } from "./watchlist-harness.mjs";

test("the top bar shows the SSO time of the selected thing's own profile", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" }, 0),
    thing("p-b", "pipeline", B, { pipeline_name: "synthetic-b" }, 1)] });
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  await page.keyboard.press("j");
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-b: 3h 1m left");
  const statuses = await calls(page, "aws_auth_status");
  expect(statuses.every(c => c.params.context?.mode === "pinned")).toBe(true);
});
