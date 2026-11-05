import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, row, rowText, saved, thing } from "./watchlist-harness.mjs";

test("a adds a pipeline from another account and saves it", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await expect(page.locator(".wl-empty h2")).toHaveText("Add your first thing");
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await expect(form).toBeVisible();
  await form.getByLabel("Kind").selectOption("pipeline");
  await form.getByLabel("Profile").selectOption("demo-b");
  await form.getByLabel("Pipeline name").fill("synthetic-new");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form).toBeHidden();
  const things = await saved(page);
  expect(things).toHaveLength(1);
  expect(things[0]).toMatchObject({ kind: "pipeline", inputs: { pipeline_name: "synthetic-new" },
    context: { mode: "pinned", profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" } });
  await expect(rowText(page, things[0].id)).toHaveText("Succeeded");
});

test("missing required fields keep the form open with a clear message", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  await page.locator("#wl-form").getByRole("button", { name: "Save" }).click();
  await expect(page.locator("#wl-form-error")).toHaveText("Fill in the required fields.");
  expect(await saved(page)).toEqual([]);
});

test("List pipelines asks only the selected account and only when pressed", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Profile").selectOption("demo-b");
  expect(await calls(page, "aws_list_pipelines")).toEqual([]);
  await form.getByRole("button", { name: "List pipelines" }).click();
  await expect(form.locator("datalist option")).toHaveAttribute("value", "synthetic-listed");
  const [listing] = await calls(page, "aws_list_pipelines");
  expect(listing.params.context).toEqual({ mode: "pinned", profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" });
});

test("e changes the account of a thing and reloads it there", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p-ok", "pipeline", B, { pipeline_name: "synthetic-ok" })] });
  await expect(rowText(page, "p-ok")).toHaveText("Succeeded");
  await page.keyboard.press("e");
  const form = page.locator("#wl-form");
  await expect(form.getByLabel("Profile")).toHaveValue("demo-b");
  await form.getByLabel("Profile").selectOption("demo-a");
  await form.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await saved(page))[0].context.profile).toBe("demo-a");
  await expect.poll(async () => (await calls(page, "widget_fetch", "pipeline-runs")).at(-1).params.context.profile).toBe("demo-a");
  await expect(row(page, "p-ok").locator(".wl-acct")).toHaveText("demo-a");
});

test("the old dashboard links to the watchlist", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#open-watchlist")).toHaveAttribute("href", "watchlist.html");
});

test("Enter on the empty-state Add a thing button opens the add dialog", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.locator(".wl-empty button").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#wl-form")).toBeVisible();
});
