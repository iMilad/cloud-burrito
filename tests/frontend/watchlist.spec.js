import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, calls, row, rowText, saved, thing } from "./watchlist-harness.mjs";

const failing = thing("p-fail", "pipeline", A, { pipeline_name: "synthetic-failing" }, 0);
const passing = thing("p-ok", "pipeline", B, { pipeline_name: "synthetic-ok" }, 1);
const pkg = thing("a-core", "package", B, { domain: "synthetic-domain", repository: "synthetic-repo", package_prefix: "acme-core" });
const cli = thing("c-1", "cli", A, { command: "aws sts get-caller-identity" });

test("each row loads with its own pinned account and shows its state", async ({ page }) => {
  await bootWatchlist(page, { things: [failing, passing, pkg] });
  await expect(rowText(page, "p-fail")).toHaveText("Failed");
  await expect(row(page, "p-fail").locator(".wl-row-state")).toHaveAttribute("data-tone", "fail");
  await expect(rowText(page, "p-ok")).toHaveText("Succeeded");
  await expect(rowText(page, "a-core")).toHaveText("4.12.0");
  const fetches = await calls(page, "widget_fetch");
  expect(fetches.every(c => c.params.context.mode === "pinned")).toBe(true);
  expect(fetches.find(c => c.params.widget === "pipeline-runs" && c.params.inputs.pipeline_name === "synthetic-ok").params.context.profile).toBe("demo-b");
  expect(await calls(page, "aws_set_account")).toEqual([]);
  expect(await calls(page, "dashboard_set")).toEqual([]);
  await expect(page.locator("#wl-summary")).toContainText("3 things in 2 accounts");
});

test("cli things wait until r runs them", async ({ page }) => {
  await bootWatchlist(page, { things: [cli] });
  await expect(rowText(page, "c-1")).toHaveText("not run yet");
  expect(await calls(page, "widget_fetch", "aws-cli")).toEqual([]);
  await row(page, "c-1").focus();
  await page.keyboard.press("r");
  await expect(rowText(page, "c-1")).toHaveText("1 row");
  expect(await calls(page, "widget_fetch", "aws-cli")).toHaveLength(1);
});

test("a failed refresh keeps the last value with a stale label", async ({ page }) => {
  await bootWatchlist(page, { things: [passing] });
  await expect(rowText(page, "p-ok")).toHaveText("Succeeded");
  await page.evaluate(() => window.__watchlistHarness.fail("pipeline-runs"));
  await row(page, "p-ok").focus();
  await page.keyboard.press("r");
  await expect(row(page, "p-ok").locator(".wl-state-flag")).toHaveText("stale");
  await expect(rowText(page, "p-ok")).toHaveText("Succeeded");
});

test("j, k, dd and K edit the list and save it", async ({ page }) => {
  await bootWatchlist(page, { things: [failing, passing, pkg] });
  await expect(row(page, "p-fail")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("j");
  await expect(row(page, "p-ok")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("K");
  await expect.poll(async () => (await saved(page)).find(t => t.id === "p-ok").order).toBe(0);
  await page.keyboard.press("d");
  await page.keyboard.press("d");
  await expect(row(page, "p-ok")).toHaveCount(0);
  await expect.poll(async () => (await saved(page)).map(t => t.id).sort()).toEqual(["a-core", "p-fail"]);
});

test("the demo page works without the desktop bridge", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => document.body.dataset.ready === "true");
  await expect(page.locator("#wl-mode")).toHaveText("Demo, sample data");
  await expect(page.locator(".wl-row")).not.toHaveCount(0);
  await expect(page.locator('.wl-row[data-thing-id="demo-pipe-1"] .wl-state-text')).toHaveText("Failed");
});

test("the demo answers the browse lists and the next package page with sample data", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => document.body.dataset.ready === "true");
  await page.keyboard.press("a");
  const form = page.locator("#wl-form");
  await form.getByLabel("Kind").selectOption("logs");
  await form.getByRole("button", { name: "List functions" }).click();
  await expect(form.locator("datalist option")).toHaveCount(2);
  await form.getByLabel("Function name or log group").fill("ledger-consumer");
  await expect(page.locator("#wl-form-log-note")).toHaveText("Log group /acme/ledger/consumer comes from this function's logging config.");
  await form.locator("#wl-field-source").selectOption("group");
  await form.getByLabel("Function name or log group").fill("lambda");
  await form.getByRole("button", { name: "List log groups" }).click();
  await expect(form.locator("datalist option")).toHaveCount(2);
  await form.getByRole("button", { name: "Cancel" }).click();
  await page.locator('.wl-row[data-thing-id="demo-pkg-1"]').click();
  await expect(page.locator("tr.expandable")).toHaveCount(3);
  await page.getByRole("button", { name: "Next package page" }).click();
  await expect(page.locator("tr.expandable")).toHaveCount(4);
  await expect(page.getByRole("button", { name: "Next package page" })).toHaveCount(0);
});

test("keyboard drills from a failed run to the failing build log line and back", async ({ page }) => {
  await bootWatchlist(page, { things: [failing] });
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb")).toHaveText("synthetic-failing");
  await expect(page.locator(".wl-item")).toHaveCount(2);
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText("run exec-0042");
  await expect(page.locator(".exec-action-block.wl-current")).toContainText("Integration tests");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb strong")).toHaveText("Build log");
  await expect(page.locator(".log-line")).toHaveCount(4);
  await page.keyboard.press("n");
  await expect(page.locator(".log-line.wl-current")).toContainText("FAILED synthetic_test");
  await page.keyboard.press("n");
  await expect(page.locator(".log-line.wl-current")).toContainText("Phase complete: FAILED");
  const build = (await calls(page, "widget_fetch", "codebuild-log"))[0];
  expect(build.params.inputs).toEqual({ build_id: "synthetic-build:2" });
  expect(build.params.context.profile).toBe("demo-a");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(page.locator(".wl-crumb")).toHaveText("synthetic-failing");
  await page.keyboard.press("Escape");
  await expect(row(page, "p-fail")).toBeFocused();
});

test("o opens only reviewed console links", async ({ page }) => {
  await bootWatchlist(page, { things: [failing] });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.locator(".exec-action-block.wl-current")).toContainText("Integration tests");
  await page.keyboard.press("o");
  await expect(page.locator("#wl-notice")).toHaveText("No AWS console link is available here.");
  await expect(page.locator(".exec-action-block").first().locator("a.exec-link"))
    .toHaveAttribute("href", /^https:\/\/eu-west-1\.console\.aws\.amazon\.com\//);
});

test("package history opens with Enter and y copies the selected version", async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: "http://127.0.0.1:4173" });
  await bootWatchlist(page, { things: [pkg] });
  await page.keyboard.press("Enter");
  await expect(page.locator("tr.expandable")).toHaveCount(1);
  await page.keyboard.press("y");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("4.12.0");
  await page.keyboard.press("Enter");
  await expect(page.locator(".codeartifact-version-value")).toHaveText(["4.12.0", "4.11.3"]);
  const history = (await calls(page, "widget_fetch", "codeartifact-package-version-history"))[0];
  expect(history.params.context.profile).toBe("demo-b");
});

test("Next package page loads page 2 in the package's own account and appends its rows", async ({ page }) => {
  await bootWatchlist(page, { things: [pkg] });
  await page.evaluate(() => { window.__watchlistHarness.packagePages = 2; });
  await page.keyboard.press("Enter");
  await expect(page.locator("tr.expandable")).toHaveCount(1);
  const next = page.getByRole("button", { name: "Next package page" });
  await expect(next).toBeVisible();
  await next.click();
  await expect(page.locator("tr.expandable")).toHaveCount(2);
  await expect(page.locator("tr.expandable").nth(1)).toContainText("acme-core-page-2");
  await expect(page.locator("tr.expandable").nth(1)).toContainText("4.12.0");
  await expect(next).toHaveCount(0);
  const lists = (await calls(page, "widget_fetch", "codeartifact-packages")).filter(c => c.params.inputs.mode === "list");
  expect(lists.at(-1).params.inputs).toEqual({ mode: "list", domain: "synthetic-domain", repository: "synthetic-repo",
    package_prefix: "acme-core", max_packages: 50, page_token: "synthetic-page-2" });
  const enrich = (await calls(page, "widget_fetch", "codeartifact-packages")).filter(c => c.params.inputs.mode === "enrich").at(-1);
  expect(enrich.params.inputs.packages).toEqual(["acme-core-page-2"]);
  for (const call of await calls(page, "widget_fetch", "codeartifact-packages")) {
    expect(call.params.context).toEqual({ mode: "pinned", ...B });
  }
});

test("a package list with one page shows no next page button", async ({ page }) => {
  await bootWatchlist(page, { things: [pkg] });
  await page.keyboard.press("Enter");
  await expect(page.locator("tr.expandable")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Next package page" })).toHaveCount(0);
});

test("the summary counts distinct accounts, not profiles", async ({ page }) => {
  const sameAccount = { profile: "demo-c", account_id: A.account_id, region: "us-east-1" };
  await bootWatchlist(page, { profiles: [A, B, sameAccount], things: [
    thing("p-a", "pipeline", A, { pipeline_name: "synthetic-a" }, 0),
    thing("p-c", "pipeline", sameAccount, { pipeline_name: "synthetic-c" }, 1)] });
  await expect(page.locator("#wl-summary")).toHaveText(/^2 things in 1 account/);
});
