import { expect, test } from "@playwright/test";
import { A, answer, bootWatchlist, calls, row, thing } from "./watchlist-harness.mjs";

const hostile = `<img src=x onerror="window.__pwned=1">`;

test("names, results and profiles render as text, never as HTML", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("x", "pipeline", A, { pipeline_name: hostile })] });
  await expect(row(page, "x").locator(".wl-row-name")).toHaveText(hostile);
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb")).toContainText(hostile);
  expect(await page.evaluate(() => window.__pwned)).toBeUndefined();
  expect(await page.locator("img[src='x']").count()).toBe(0);
});

test("the page makes no remote requests and never uses the inherited account path", async ({ page }) => {
  const remote = [];
  page.on("request", request => { if (!request.url().startsWith("http://127.0.0.1:4173")) remote.push(request.url()); });
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  expect(remote).toEqual([]);
  expect(await calls(page, "aws_set_account")).toEqual([]);
  expect(await calls(page, "dashboard_set")).toEqual([]);
  const fetches = await calls(page, "widget_fetch");
  expect(fetches.length).toBeGreaterThan(0);
  expect(fetches.every(c => c.params.context?.mode === "pinned" && typeof c.params.request_id === "string")).toBe(true);
});

// Hostile text must stay text in every renderer the inspector reuses.
async function expectInert(page, scope) {
  expect(await page.evaluate(() => window.__pwned)).toBeUndefined();
  expect(await page.locator("img[src='x']").count()).toBe(0);
  await expect(scope.locator("img, svg, script, [onerror], [onload]")).toHaveCount(0);
}

test("a hostile log event message renders as text", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("l", "logs", A, { log_group: "/synthetic/group", source: "group" })] });
  await page.evaluate(hostile => {
    const widgets = window.__watchlistHarness.widgets;
    const streams = widgets["cloudwatch-logs"];
    widgets["cloudwatch-logs"] = i => i.mode === "events"
      ? { render: "log_stream", events: [{ ts: 1700000000000, msg: hostile, level: "info" }] } : streams(i);
  }, hostile);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  const view = page.locator(".wl-view-host");
  await expect(view.locator(".log-line .msg")).toHaveText(hostile);
  await expectInert(page, view);
});

test("a hostile stack resource id renders as text", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("s", "stack", A, { stack_name: "synthetic-stack" })] });
  await answer(page, "cfn-stack-detail", { render: "stack_detail", stack: "synthetic-stack", events: [],
    resources: [{ logical_id: hostile, type: "AWS::DynamoDB::Table", status: "CREATE_COMPLETE", physical_id: hostile }] });
  await page.keyboard.press("r");
  await page.keyboard.press("Enter");
  const view = page.locator(".wl-view-host");
  await expect(view.locator(".stack-resource-physical").first()).toHaveText(hostile);
  await expectInert(page, view);
});

test("a hostile package name renders as text", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("k", "package", A, { domain: "d", repository: "r", package_prefix: "acme-core" })] });
  await answer(page, "codeartifact-packages", { render: "table", mode: "list", columns: ["package", "latest_version", "last_published"],
    next_page_token: null, rows: [{ package: hostile, latest_version: "1.0.0", last_published: "2026-11-02T11:02:00Z",
      versions: [{ version: "1.0.0", published: "2026-11-02T11:02:00Z" }], enrichment_state: "complete" }] });
  await page.keyboard.press("r");
  await page.keyboard.press("Enter");
  const view = page.locator(".wl-view-host");
  await expect(view.locator("tr.expandable")).toContainText(hostile);
  await expectInert(page, view);
});

test("a hostile lookup match renders as text", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("m", "lookup", A, { query: "synthetic-fn" })] });
  await answer(page, "resource-lookup", { render: "reverse_lookup", query: "synthetic-fn", scanned: 1,
    matches: [{ arn: hostile, type: hostile }] });
  await page.keyboard.press("r");
  await page.keyboard.press("Enter");
  const view = page.locator(".wl-view-host");
  await expect(view.locator(".lookup-row")).toContainText(hostile);
  await expectInert(page, view);
});

test("the policy editor highlight renders hostile YAML as text", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  await page.keyboard.press(",");
  await page.locator("#policy-editor").fill(`Statement:\n  - Effect: Allow\n    Resource: '${hostile}'\n# ${hostile}`);
  const highlight = page.locator("#policy-highlight");
  await expect(highlight).toContainText(hostile);
  await expectInert(page, highlight);
});
