import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, saved, thing } from "./watchlist-harness.mjs";

test(": runs a pipeline lookup in another account without saving it", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-ok", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await expect(input).toBeFocused();
  await input.fill("pipe synthetic-failing @demo-b");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("synthetic-failing");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  await expect(page.locator(".wl-item")).toHaveCount(2);
  const run = (await calls(page, "widget_fetch", "pipeline-runs")).find(c => c.params.inputs.pipeline_name === "synthetic-failing");
  expect(run.params.context).toEqual({ mode: "pinned", ...B });
  expect(await saved(page)).toHaveLength(1);
  await page.keyboard.press("s");
  await expect.poll(async () => (await saved(page)).map(t => t.inputs.pipeline_name)).toEqual(["synthetic-ok", "synthetic-failing"]);
  await expect(page.locator(".wl-crumb")).not.toContainText("not saved");
});

test("Tab completes profiles and errors keep the command line open", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-x @demo-");
  await expect(page.locator("#wl-command-list [role=option]")).toHaveCount(2);
  await input.press("ArrowDown");
  await input.press("Tab");
  await expect(input).toHaveValue("pipe synthetic-x @demo-b");
  await input.fill("pipe synthetic-x @nobody");
  await input.press("Enter");
  await expect(page.locator("#wl-command-hint")).toContainText("Unknown profile @nobody");
  await expect(input).toBeVisible();
  await input.press("Escape");
  await expect(input).toBeHidden();
});

test("Meta+K opens the command line and :add opens the form", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("Meta+k");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill(":add");
  await input.press("Enter");
  await expect(page.locator("#wl-form")).toBeVisible();
});

test("s refuses a 201st thing and does not write", async ({ page }) => {
  const things = Array.from({ length: 200 }, (_, n) => thing(`synthetic-${n}`, "pipeline", A, { pipeline_name: `synthetic-p${n}` }, n));
  await bootWatchlist(page, { things });
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-extra @demo-b");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  await page.keyboard.press("s");
  await expect(page.locator("#wl-notice")).toContainText("The watchlist is full (200 things). Remove one, then try again.");
  expect(await calls(page, "watchlist_set")).toHaveLength(0);
  expect(await saved(page)).toHaveLength(200);
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
});

test("dd, e, J and K do nothing to the list while an unsaved result is shown", async ({ page }) => {
  const things = [thing("p-1", "pipeline", A, { pipeline_name: "synthetic-one" }, 0),
    thing("p-2", "pipeline", A, { pipeline_name: "synthetic-two" }, 1)];
  await bootWatchlist(page, { things });
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-x @demo-a");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  const notice = page.locator("#wl-notice");
  for (const keys of [["d", "d"], ["e"], ["Shift+J"], ["Shift+K"]]) {
    await page.evaluate(() => { document.getElementById("wl-notice").textContent = ""; });
    for (const key of keys) await page.keyboard.press(key);
    await expect(notice).toHaveText("This result is not saved. Press s to save it.");
    await expect(page.locator("#wl-form")).toBeHidden();
  }
  expect(await calls(page, "watchlist_set")).toEqual([]);
  expect((await saved(page)).map(t => t.id)).toEqual(["p-1", "p-2"]);
  await expect(page.locator(".wl-row")).toHaveCount(2);
  await expect(page.locator(".wl-row").first()).toHaveAttribute("data-thing-id", "p-1");
});
