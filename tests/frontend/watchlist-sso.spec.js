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

test("removing the last thing clears the SSO line", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" })] });
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  await page.keyboard.press("d");
  await page.keyboard.press("d");
  await expect(page.locator(".wl-row")).toHaveCount(0);
  await expect(page.locator("#wl-sso")).toBeHidden();
  await expect(page.locator("#wl-sso")).toHaveText("");
});

test("showing an unsaved command result clears the SSO line", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" })] });
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-x @demo-b");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  await expect(page.locator("#wl-sso")).toBeHidden();
});

test("a late reply for the previous thing never paints over the new selection", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" }, 0),
    thing("p-b", "pipeline", B, { pipeline_name: "synthetic-b" }, 1)] });
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  await page.evaluate(() => window.__watchlistHarness.hold("aws_auth_status"));
  await page.keyboard.press("j");
  await expect.poll(async () => (await calls(page, "aws_auth_status")).length).toBe(2);
  await page.keyboard.press("k");
  // The reply for B lands while the debounce for A is still waiting.
  await page.evaluate(() => window.__watchlistHarness.release("aws_auth_status"));
  await page.waitForTimeout(100);
  await expect(page.locator("#wl-sso")).not.toContainText("demo-b");
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
});

test("reordering keeps the status and asks AWS nothing new", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" }, 0),
    thing("p-b", "pipeline", B, { pipeline_name: "synthetic-b" }, 1)] });
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  const before = (await calls(page, "aws_auth_status")).length;
  await page.keyboard.press("J");
  await page.waitForTimeout(500);
  await expect(page.locator("#wl-sso")).toHaveText("SSO for demo-a: 7h 1m left");
  expect((await calls(page, "aws_auth_status")).length).toBe(before);
});
