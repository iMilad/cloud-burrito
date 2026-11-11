import { expect, test } from "@playwright/test";
import { A, B, answer, bootWatchlist, calls, row, rowText, saved, thing } from "./watchlist-harness.mjs";

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

test("Enter on the empty-state Add a thing button opens the add dialog", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.locator(".wl-empty button").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#wl-form")).toBeVisible();
});

test("a late pipeline listing is dropped after the profile changes", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Profile").selectOption("demo-b");
  await page.evaluate(() => window.__watchlistHarness.hold("aws_list_pipelines"));
  await form.getByRole("button", { name: "List pipelines" }).click();
  await expect.poll(async () => (await calls(page, "aws_list_pipelines")).length).toBe(1);
  await form.getByLabel("Profile").selectOption("demo-a");
  await page.evaluate(() => window.__watchlistHarness.release("aws_list_pipelines"));
  await page.waitForTimeout(150);
  await expect(form.locator("datalist option")).toHaveCount(0);
});

test("editing a thing with a missing profile refuses and keeps the saved context", async ({ page }) => {
  const gone = { profile: "demo-gone", account_id: "acct-gone-fixture", region: "eu-west-1" };
  await bootWatchlist(page, { things: [thing("p-gone", "pipeline", gone, { pipeline_name: "synthetic-gone" })] });
  await page.keyboard.press("e");
  const form = page.locator("#wl-form");
  await expect(form.getByLabel("Profile")).toHaveValue("demo-gone");
  await expect(form.getByLabel("Profile").locator("option:checked")).toHaveText("demo-gone (not in your AWS config)");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(page.locator("#wl-form-error")).toHaveText("Choose a profile from your AWS config.");
  expect((await saved(page))[0].context).toMatchObject({ profile: "demo-gone", account_id: "acct-gone-fixture" });
});

test("editing shows a note when the profile now points to another account", async ({ page }) => {
  const moved = { profile: "demo-b", account_id: "acct-old-fixture", region: "eu-west-1" };
  await bootWatchlist(page, { things: [thing("p-moved", "pipeline", moved, { pipeline_name: "synthetic-moved" })] });
  await page.keyboard.press("e");
  await expect(page.locator("#wl-form-account-note")).toHaveText("This profile now points to account acct-b-fixture. Saving pins this thing to it.");
});

test("a full watchlist refuses another thing and does not save", async ({ page }) => {
  const things = Array.from({ length: 200 }, (_, i) => thing(`p-${i}`, "pipeline", A, { pipeline_name: `synthetic-${i}` }, i));
  await bootWatchlist(page, { things });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Pipeline name").fill("synthetic-extra");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(page.locator("#wl-form-error")).toHaveText("The watchlist is full (200 things). Remove one, then try again.");
  expect(await calls(page, "watchlist_set")).toEqual([]);
  expect(await saved(page)).toHaveLength(200);
});

const pinnedB = { mode: "pinned", profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" };
const pinnedA = { mode: "pinned", profile: "demo-a", account_id: "acct-a-fixture", region: "eu-west-1" };

test("List functions asks the form's account and sets the log group from the reviewed handoff", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await form.getByLabel("Profile").selectOption("demo-b");
  expect(await calls(page, "widget_fetch", "log-tail")).toEqual([]);
  await form.getByRole("button", { name: "List functions" }).click();
  await expect(form.locator("datalist option")).toHaveCount(2);
  await expect(form.locator("datalist option").first()).toHaveAttribute("value", "synthetic-fn-custom");
  const [listing] = await calls(page, "widget_fetch", "log-tail");
  expect(listing.params.inputs).toEqual({ mode: "list", max_functions: 500 });
  expect(listing.params.context).toEqual(pinnedB);
  const name = form.getByLabel("Function name or log group");
  const note = page.locator("#wl-form-log-note");
  await name.fill("synthetic-fn-default");
  await expect(note).toHaveText("Log group /aws/lambda/synthetic-fn-default comes from the default Lambda naming convention. It is not verified.");
  await name.fill("synthetic-fn-custom");
  await expect(note).toHaveText("Log group /synthetic/custom/exact comes from this function's logging config.");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form).toBeHidden();
  const [stored] = await saved(page);
  expect(stored.inputs).toEqual({ source: "lambda", function_name: "synthetic-fn-custom", log_group: "/synthetic/custom/exact" });
  expect(stored.context).toEqual(pinnedB);
  await expect.poll(async () => (await calls(page, "widget_fetch", "log-tail")).at(-1).params.inputs)
    .toEqual({ mode: "streams", log_group: "/synthetic/custom/exact", max_streams: 1 });
});

test("editing a function keeps its reviewed log group, and the inspector says where it came from", async ({ page }) => {
  const fn = thing("l-fn", "logs", B, { source: "lambda", function_name: "synthetic-fn-custom", log_group: "/synthetic/custom/exact" });
  await bootWatchlist(page, { things: [fn] });
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view")).toContainText("Log group /synthetic/custom/exact comes from this function's logging config.");
  await expect(page.locator(".wl-view")).not.toContainText("naming convention");
  await page.keyboard.press("Escape");
  await page.keyboard.press("e");
  const form = page.locator("#wl-form");
  await expect(page.locator("#wl-form-log-note")).toHaveText("Log group /synthetic/custom/exact comes from this function's logging config.");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form).toBeHidden();
  expect((await saved(page))[0].inputs.log_group).toBe("/synthetic/custom/exact");
});

test("List log groups uses the typed text, for logs and for a query, in the form's account", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await form.locator("#wl-field-source").selectOption("group");
  await expect(form.getByRole("button", { name: "List functions" })).toHaveCount(0);
  await form.getByLabel("Function name or log group").fill("group-");
  await form.getByRole("button", { name: "List log groups" }).click();
  await expect(form.locator("datalist option")).toHaveCount(2);
  await expect(form.locator("datalist option").first()).toHaveAttribute("value", "/synthetic/group-a");
  const [logs] = await calls(page, "widget_fetch", "cloudwatch-logs");
  expect(logs.params.inputs).toEqual({ mode: "groups", max_groups: 500, name_pattern: "group-" });
  expect(logs.params.context).toEqual(pinnedA);

  await form.getByLabel("Kind").selectOption("query");
  await expect(form.locator("datalist option")).toHaveCount(0);
  await form.getByLabel("Region").selectOption("us-east-1");
  await form.getByRole("button", { name: "List log groups" }).click();
  await expect(form.locator("datalist option")).toHaveCount(3);
  const query = (await calls(page, "widget_fetch", "cloudwatch-logs")).at(-1);
  expect(query.params.inputs).toEqual({ mode: "groups", max_groups: 500 });
  expect(query.params.context).toEqual({ ...pinnedA, region: "us-east-1" });
  await form.getByLabel("Log group").fill("/synthetic/group-b");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form).toBeHidden();
  expect((await saved(page))[0]).toMatchObject({ kind: "query", inputs: { log_group: "/synthetic/group-b" },
    context: { ...pinnedA, region: "us-east-1" } });
  // A query never runs on its own.
  expect(await calls(page, "widget_fetch", "logs-insights")).toEqual([]);
});

test("a late function listing is dropped after the source or profile changes", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await page.evaluate(() => window.__watchlistHarness.hold("log-tail", "list"));
  await form.getByRole("button", { name: "List functions" }).click();
  await expect.poll(async () => (await calls(page, "widget_fetch", "log-tail")).length).toBe(1);
  await form.getByLabel("Profile").selectOption("demo-b");
  await page.evaluate(() => window.__watchlistHarness.release("log-tail", "list"));
  await page.waitForTimeout(150);
  await expect(form.locator("datalist option")).toHaveCount(0);
  await form.getByLabel("Function name or log group").fill("synthetic-fn-custom");
  await expect(page.locator("#wl-form-log-note")).toContainText("It is not verified.");
});

test("a denied function listing says it is blocked by policy", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await answer(page, "log-tail", { render: "permission_denied", action: "lambda:ListFunctions", reason: "Synthetic policy denied.", _outcome: "denied" });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await form.getByRole("button", { name: "List functions" }).click();
  await expect(page.locator("#wl-form-error")).toHaveText("Blocked by policy: lambda:ListFunctions. Allow it in the policy, then try again.");
  await expect(form.locator("datalist option")).toHaveCount(0);
});

// Listings show that they are loading, and say when they stopped early.
async function capListing(page, widget, mode) {
  await page.evaluate(([widget, mode]) => {
    const harness = window.__watchlistHarness;
    const normal = harness.widgets[widget];
    harness.widgets[widget] = i => i.mode === mode ? { ...normal(i), capped: true } : normal(i);
  }, [widget, mode]);
}

test("List functions shows that it is loading, and says when the list stopped early", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await capListing(page, "log-tail", "list");
  await page.evaluate(() => window.__watchlistHarness.hold("log-tail", "list"));
  const button = form.getByRole("button", { name: "List functions" });
  const status = page.locator("#wl-form-list-status");
  await button.click();
  await expect(status).toHaveText("Loading the list…");
  await expect(button).toHaveAttribute("aria-busy", "true");
  await page.evaluate(() => window.__watchlistHarness.release("log-tail", "list"));
  await expect(form.locator("datalist option")).toHaveCount(2);
  await expect(status).toHaveText("The list stopped at 2 functions. Type the name if yours is not listed.");
  await expect(button).toHaveAttribute("aria-busy", "false");
});

test("List log groups says when the list stopped early, and a new choice clears the note", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("query");
  const status = page.locator("#wl-form-list-status");
  await form.getByRole("button", { name: "List log groups" }).click();
  await expect(form.locator("datalist option")).toHaveCount(3);
  await expect(status).toBeHidden();
  await capListing(page, "cloudwatch-logs", "groups");
  await form.getByRole("button", { name: "List log groups" }).click();
  await expect(status).toHaveText("The list stopped at 3 log groups. Type more of the name, then list again.");
  await form.getByLabel("Region").selectOption("us-east-1");
  await expect(status).toBeHidden();
  await expect(form.locator("datalist option")).toHaveCount(0);
});
