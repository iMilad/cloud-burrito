import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, rowText, saved, thing } from "./watchlist-harness.mjs";

const query = thing("q-1", "query", A, { log_group: "/synthetic/group", query: "fields @message | limit 5", range_seconds: 3600 });
const errors = thing("e-1", "errors", A, { hours: 24, log_group_pattern: "/aws/lambda/" }, 1);

test("cost commands never run on open and show the cost note first", async ({ page }) => {
  await bootWatchlist(page, { things: [query, errors] });
  await expect(rowText(page, "q-1")).toHaveText("not run yet");
  await expect(rowText(page, "e-1")).toHaveText("not run yet");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view")).toContainText("can cost money");
  expect(await calls(page, "widget_fetch", "logs-insights")).toEqual([]);
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view tbody tr")).toHaveCount(1);
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(1);
});

test("an unknown earlier query blocks reruns until the user acknowledges it once", async ({ page }) => {
  await bootWatchlist(page, { things: [query] });
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = true; });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.locator(".query-recovery")).toContainText("may still be running");
  const before = (await calls(page, "widget_fetch", "logs-insights")).length;
  await page.keyboard.press("r");
  await expect(page.locator(".query-recovery")).toBeVisible();
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(before);
  const review = page.getByRole("button", { name: "Review and run again" });
  await expect(review).toBeDisabled();
  await page.locator(".query-recovery select").selectOption("q-1");
  await page.getByLabel("I understand the earlier query may still be running").check();
  await review.click();
  await expect(page.locator(".wl-view tbody tr")).toHaveCount(1);
  const last = (await calls(page, "widget_fetch", "logs-insights")).at(-1);
  expect(last.params.inputs).toMatchObject({ acknowledge_unknown: true, acknowledge_query: "q-1" });
});

test("the form can add a CLI command, which waits for r", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("cli");
  await expect(form).toContainText("Only reviewed read-only AWS CLI commands");
  await form.getByLabel("AWS CLI command", { exact: true }).fill("aws sts get-caller-identity");
  await form.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await saved(page))[0]?.kind).toBe("cli");
  expect(await calls(page, "widget_fetch", "aws-cli")).toEqual([]);
  expect(await calls(page, "cli_availability")).toEqual([]);
});

for (const [label, text, widget, check] of [
  ["aws", "aws sts get-caller-identity", "aws-cli", i => expect(i.command).toBe("aws sts get-caller-identity")],
  ["insights", 'insights /synthetic/group "fields @message | limit 5"', "logs-insights", i => expect(i.log_group).toBe("/synthetic/group")],
  ["errors", "errors", "errors-by-stack", i => expect(i.hours).toBe(24)],
]) {
  test(`:${label} from the command line waits for r and then fetches once`, async ({ page }) => {
    await bootWatchlist(page, { things: [] });
    await page.keyboard.press(":");
    const input = page.getByRole("combobox", { name: "Command" });
    await input.fill(text);
    await input.press("Enter");
    await expect(page.locator(".wl-crumb")).toContainText("not saved");
    await expect(page.locator(".wl-view")).toContainText("Press r or Enter to run it");
    await page.waitForTimeout(300);
    expect(await calls(page, "widget_fetch", widget)).toEqual([]);
    await page.keyboard.press("r");
    await expect.poll(async () => (await calls(page, "widget_fetch", widget)).length).toBe(1);
    await page.waitForTimeout(300);
    const fetches = await calls(page, "widget_fetch", widget);
    expect(fetches).toHaveLength(1);
    check(fetches[0].params.inputs);
    expect(fetches[0].params.context).toMatchObject({ mode: "pinned" });
    expect(await saved(page)).toEqual([]);
  });
}

for (const [kind, item, widget] of [
  ["query", query, "logs-insights"],
  ["cli", thing("c-1", "cli", A, { command: "aws sts get-caller-identity" }), "aws-cli"],
]) {
  test(`one r runs a saved ${kind} thing once and updates its row`, async ({ page }) => {
    await bootWatchlist(page, { things: [item] });
    await page.keyboard.press("Enter");
    await page.keyboard.press("r");
    await expect(page.locator(".wl-view tbody tr")).toHaveCount(1);
    await expect(rowText(page, item.id)).not.toHaveText("not run yet");
    await page.waitForTimeout(300);
    expect(await calls(page, "widget_fetch", widget)).toHaveLength(1);
  });
}

test("Enter does not rerun a query after results are shown", async ({ page }) => {
  await bootWatchlist(page, { things: [query] });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view tbody tr")).toHaveCount(1);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(1);
});

test("changing the earlier query clears the acknowledgement tick", async ({ page }) => {
  await bootWatchlist(page, { things: [query] });
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = true; });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  const review = page.getByRole("button", { name: "Review and run again" });
  await page.locator(".query-recovery select").selectOption("q-1");
  await page.getByLabel("I understand the earlier query may still be running").check();
  await expect(review).toBeEnabled();
  await page.locator(".query-recovery select").selectOption("");
  await page.locator(".query-recovery select").selectOption("q-1");
  await expect(page.getByLabel("I understand the earlier query may still be running")).not.toBeChecked();
  await expect(review).toBeDisabled();
});

test("a query paused by an earlier query says so on the row, and opening it shows the review", async ({ page }) => {
  await bootWatchlist(page, { things: [query] });
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = true; });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.locator(".query-recovery")).toBeVisible();
  await expect(rowText(page, "q-1")).toHaveText("paused, press Enter to review the earlier query");
  await page.keyboard.press("r");
  const view = page.locator(".wl-view-host");
  await expect(view).toContainText("An earlier query may still be running. Review it before running again.");
  await expect(view).not.toContainText("Error:");
  await expect(rowText(page, "q-1")).not.toContainText("press r to retry");
  // Opened again from the list, it shows the review first, not "Press r or Enter to run it".
  await page.keyboard.press("Escape");
  await page.keyboard.press("Enter");
  await expect(page.locator(".query-recovery")).toBeVisible();
  await expect(page.locator(".wl-view")).not.toContainText("Press r or Enter to run it.");
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(1);
  await page.locator(".query-recovery select").selectOption("q-1");
  await page.getByLabel("I understand the earlier query may still be running").check();
  await page.getByRole("button", { name: "Review and run again" }).click();
  await expect(page.locator(".wl-view tbody tr")).toHaveCount(1);
  await expect(rowText(page, "q-1")).toHaveText("1 row");
});
