import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, row, saved, thing } from "./watchlist-harness.mjs";

const one = thing("p-1", "pipeline", A, { pipeline_name: "synthetic-one" }, 0);
const two = thing("p-2", "pipeline", B, { pipeline_name: "synthetic-two" }, 1);
const stack = thing("s-1", "stack", A, { stack_name: "synthetic-stack" });
// Rows the user can see, not only rows marked hidden.
const visible = page => page.locator("#wl-list .wl-row:visible");
const focusArea = page => page.locator("body");

test("/ filters rows by name, profile or kind, and Esc clears it", async ({ page }) => {
  await bootWatchlist(page, { things: [one, two, stack] });
  await page.keyboard.press("/");
  await expect(page.locator("#wl-filter")).toBeFocused();
  await page.keyboard.type("two");
  await expect(visible(page)).toHaveCount(1);
  await expect(row(page, "p-2")).toBeVisible();
  await expect(row(page, "p-1")).toBeHidden();
  await expect(row(page, "p-2")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator('.wl-group[data-group="stacks"]')).toBeHidden();
  await page.locator("#wl-filter").fill("demo-b");
  await expect(visible(page)).toHaveCount(1);
  await page.locator("#wl-filter").fill("pipeline");
  await expect(visible(page)).toHaveCount(2);
  await page.keyboard.press("Escape");
  await expect(page.locator("#wl-filter")).toHaveValue("");
  await expect(visible(page)).toHaveCount(3);
  await expect(focusArea(page)).toHaveAttribute("data-focus", "list");
  await expect(row(page, "p-2")).toBeFocused();
});

test("typing in the filter never runs list keys", async ({ page }) => {
  await bootWatchlist(page, { things: [one, two] });
  await page.keyboard.press("/");
  await page.keyboard.type("dd");
  await expect(visible(page)).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(visible(page)).toHaveCount(2);
  expect(await calls(page, "watchlist_set")).toEqual([]);
});

test("Tab moves between the list and the inspector", async ({ page }) => {
  await bootWatchlist(page, { things: [one, two] });
  await page.keyboard.press("Enter");
  const items = page.locator(".wl-view-host .wl-item");
  await expect(items).toHaveCount(2);
  await expect(focusArea(page)).toHaveAttribute("data-focus", "inspector");
  await expect(page.locator("#wl-keys")).toContainText("Back");
  await page.keyboard.press("Tab");
  await expect(focusArea(page)).toHaveAttribute("data-focus", "list");
  await expect(row(page, "p-1")).toBeFocused();
  await expect(page.locator("#wl-keys")).toContainText("Add");
  await page.keyboard.press("j");
  await expect(row(page, "p-2")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(focusArea(page)).toHaveAttribute("data-focus", "inspector");
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("j");
  await expect(items.nth(1)).toHaveClass(/wl-current/);
  // The inspector still shows the thing opened with Enter.
  await expect(page.locator(".wl-crumb strong")).toHaveText("synthetic-one");
});

test("after Esc from an unsaved result, dd, e, J and K act on the list again", async ({ page }) => {
  await bootWatchlist(page, { things: [one, two] });
  await page.keyboard.press(":");
  const input = page.getByRole("combobox", { name: "Command" });
  await input.fill("pipe synthetic-x @demo-a");
  await input.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
  await page.keyboard.press("Escape");
  await expect(focusArea(page)).toHaveAttribute("data-focus", "list");
  await page.keyboard.press("Shift+J");
  await expect(page.locator(".wl-row").first()).toHaveAttribute("data-thing-id", "p-2");
  await expect.poll(async () => (await saved(page)).find(t => t.id === "p-1")?.order).toBe(1);
  await page.keyboard.press("Shift+K");
  await expect(page.locator(".wl-row").first()).toHaveAttribute("data-thing-id", "p-1");
  await page.keyboard.press("e");
  await expect(page.locator("#wl-form")).toBeVisible();
  await expect(page.getByLabel("Pipeline name")).toHaveValue("synthetic-one");
  await page.keyboard.press("Escape");
  await expect(page.locator("#wl-form")).toBeHidden();
  await page.keyboard.press("d");
  await page.keyboard.press("d");
  await expect(page.locator(".wl-row")).toHaveCount(1);
  await expect.poll(async () => (await saved(page)).map(t => t.id)).toEqual(["p-2"]);
  // The unsaved result stays in the inspector, so s can still save it.
  await expect(page.locator(".wl-crumb")).toContainText("not saved");
});
