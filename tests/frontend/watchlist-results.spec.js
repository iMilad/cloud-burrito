import { expect, test } from "@playwright/test";
import { A, answer, bootWatchlist, calls, row, rowText, thing } from "./watchlist-harness.mjs";

// Result states reach the user in two places: the row and the inspector.
const pipe = thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" });
const runs = (extra = {}) => ({ render: "table", columns: ["execution_id", "status", "last_updated", "trigger"],
  rows: [{ execution_id: "exec-0042", status: "Succeeded", last_updated: "2026-11-02T14:02:00Z", trigger: "Webhook: main@a1f3e9c" }], ...extra });
const coverage = (completeness, extra = {}) => ({ completeness, has_more: completeness === "limited" ? true : null,
  counts: { returned: 1, pages: 1 }, limits: { results: 10 }, reasons: [], ...extra });
const status = page => page.locator(".wl-view-host > .result-status");

test("a partial result shows the partial flag on the row and a partial notice in the inspector", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await answer(page, "pipeline-runs", runs({ ok: false, partial: true, error_type: "PartialFailure",
    error: "A synthetic page failed. Returned evidence is retained.", coverage: coverage("unknown") }));
  await page.keyboard.press("r");
  await expect(row(page, "p").locator(".wl-state-flag")).toHaveText("partial");
  await expect(rowText(page, "p")).toHaveText("Succeeded");
  await page.keyboard.press("Enter");
  await expect(status(page)).toHaveAttribute("data-state", "partial");
  await expect(status(page)).toContainText("Partial result");
  await expect(page.locator(".wl-view-host")).toContainText("exec-0042");
});

test("a limited result shows the partial flag on the row and a limited notice in the inspector", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await answer(page, "pipeline-runs", runs({ truncated: true,
    coverage: coverage("limited", { reasons: [{ code: "next_page", message: "More runs were not loaded." }] }) }));
  await page.keyboard.press("r");
  await expect(row(page, "p").locator(".wl-state-flag")).toHaveText("partial");
  await page.keyboard.press("Enter");
  await expect(status(page)).toHaveAttribute("data-state", "limited");
  await expect(status(page)).toContainText("Limited result");
  await expect(status(page)).toContainText("More were not loaded.");
});

test("a denied result is blocked by policy on the row and shows the permission view in the inspector", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await answer(page, "pipeline-runs", { render: "permission_denied", action: "codepipeline:ListPipelineExecutions",
    reason: "Synthetic policy denied.", _outcome: "denied" });
  await page.keyboard.press("r");
  await expect(rowText(page, "p")).toHaveText("blocked by policy: codepipeline:ListPipelineExecutions");
  await expect(row(page, "p").locator(".wl-row-state")).toHaveAttribute("data-tone", "fail");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view-host .permission-denied-action")).toHaveText("Request blocked: codepipeline:ListPipelineExecutions");
  await expect(page.locator(".wl-view-host .permission-denied-reason")).toHaveText("Synthetic policy denied.");
  await expect(status(page)).toHaveAttribute("data-state", "denied");
});

test("a malformed refresh does not replace the accepted result in the inspector", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view-host .wl-item")).toHaveCount(2);
  await expect(status(page)).toHaveAttribute("data-state", "success");
  await answer(page, "pipeline-runs", runs({ rows: "not-a-list" }));
  await page.keyboard.press("r");
  await expect(status(page)).toHaveAttribute("data-state", "stale");
  await expect(status(page)).toContainText("Response data shape was invalid.");
  await expect(page.locator(".wl-view-host .wl-item")).toHaveCount(2);
  await answer(page, "pipeline-runs", runs({ coverage: { completeness: "everything" } }));
  await page.keyboard.press("r");
  await expect(status(page)).toContainText("Response coverage metadata was invalid.");
  await expect(page.locator(".wl-view-host .wl-item")).toHaveCount(2);
});

test("a malformed result does not replace the accepted row value", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await expect(rowText(page, "p")).toHaveText("Succeeded");
  await answer(page, "pipeline-runs", runs({ rows: "not-a-list", coverage: { completeness: "everything" } }));
  await page.keyboard.press("r");
  await expect.poll(async () => (await calls(page, "widget_fetch", "pipeline-runs")).length).toBe(2);
  await expect(rowText(page, "p")).toHaveText("Succeeded");
  await expect(row(page, "p").locator(".wl-state-flag")).toHaveText("stale");
});

// Error counts: absent rows never prove zero errors.
const errs = thing("e", "errors", A, { hours: 24, log_group_pattern: "/aws/lambda/" });
const NO_ZERO = /no error|zero errors? found|\b0 errors?\b/i;
const counts = { queried: 2, succeeded: 0, failed: 1, timed_out: 1 };

async function runErrors(page, result) {
  await bootWatchlist(page, { things: [errs] });
  await answer(page, "errors-by-stack", result);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
}

test("a failed error-count query never claims zero errors on the row or in the inspector", async ({ page }) => {
  await runErrors(page, { render: "errors_chart", hours: 24, rows: [], ok: false, error_type: "QueryFailed",
    error: "Synthetic query failed", counts });
  await expect(rowText(page, "e")).toHaveText("failed, press r to retry");
  await expect(page.locator(".wl-view-host")).toContainText("Synthetic query failed");
  await expect(rowText(page, "e")).not.toContainText(NO_ZERO);
  await expect(page.locator(".wl-view-host")).not.toContainText(NO_ZERO);
});

test("a partial error-count query with no rows says the counts are incomplete", async ({ page }) => {
  await runErrors(page, { render: "errors_chart", hours: 24, status: "failed", partial: true, rows: [],
    error: "Synthetic query failed", counts });
  await expect(rowText(page, "e")).toHaveText("counts incomplete");
  await expect(row(page, "e").locator(".wl-state-flag")).toHaveText("partial");
  const view = page.locator(".wl-view-host");
  await expect(view).toContainText("absence of rows does not establish zero errors");
  await expect(view).toContainText("0 succeeded, 1 failed, 1 timed out, 2 queried");
  await expect(rowText(page, "e")).not.toContainText(NO_ZERO);
  await expect(view).not.toContainText(NO_ZERO);
  await expect(view).not.toContainText("No error events");
});

test("a partial error-count query with rows shows the surviving counts and its coverage", async ({ page }) => {
  await runErrors(page, { render: "errors_chart", hours: 24, status: "partial", partial: true,
    rows: [{ stack: "synthetic-surviving-group", errors: 2 }], error: "Some groups failed",
    counts: { queried: 2, succeeded: 1, failed: 1, timed_out: 0 } });
  await expect(rowText(page, "e")).toHaveText("synthetic-surviving-group: 2");
  await expect(row(page, "e").locator(".wl-state-flag")).toHaveText("partial");
  await expect(page.locator(".wl-view-host")).toContainText("1 succeeded, 1 failed");
});

// Query cleanup: a cancel is local until the owned result confirms StopQuery.
const B = { profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" };
const q = (id, identity, order) => thing(id, "query", identity, { log_group: "/synthetic/group", query: "fields @message | limit 5", range_seconds: 3600 }, order);
const cancelled = cleanup => ({ render: "table", columns: ["@message"], rows: [], ok: false, error_type: "QueryCancelled",
  error: "Synthetic cancelled query", query_state: "cancelled", recovery_required: false, cleanup, _outcome: "cancelled" });

async function startAndCancel(page) {
  await bootWatchlist(page, { things: [q("q-a", A, 0)] });
  await page.keyboard.press("Enter");
  await page.evaluate(() => window.__watchlistHarness.hold("logs-insights"));
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await calls(page, "widget_fetch", "logs-insights")).length).toBe(1);
  await page.getByRole("button", { name: "Cancel current request" }).click();
  await expect(page.locator(".request-cancel-status")).toHaveText("Cancelled locally. Remote cleanup is not yet confirmed.");
  await expect(page.locator(".wl-view-host")).not.toContainText("Remote query stop confirmed.");
  expect(await calls(page, "request_cancel")).toHaveLength(1);
}

test("Cancel stays unconfirmed until the owned result confirms StopQuery", async ({ page }) => {
  await startAndCancel(page);
  await answer(page, "logs-insights", cancelled({ status: "stopped", remote_stop_confirmed: true, remote_queries_may_still_run: false }));
  await page.evaluate(() => window.__watchlistHarness.release("logs-insights"));
  await expect(page.locator(".wl-view-host .result-cleanup")).toHaveText("Remote query stop confirmed.");
  await expect(page.locator(".query-recovery")).toHaveCount(0);
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(1);
});

test("Cancel with an unconfirmed stop says remote queries may still run and pauses reruns", async ({ page }) => {
  await startAndCancel(page);
  await answer(page, "logs-insights", cancelled({ status: "not_confirmed", remote_stop_confirmed: false, remote_queries_may_still_run: true }));
  await page.evaluate(() => window.__watchlistHarness.release("logs-insights"));
  await expect(page.locator(".wl-view-host .result-cleanup")).toHaveText("Remote stop was not confirmed.");
  await expect(page.locator(".wl-view-host")).toContainText("Remote queries may still be running.");
  await expect(page.locator(".wl-view-host")).not.toContainText("Remote query stop confirmed.");
  await expect(page.locator(".query-recovery")).toContainText("Runs are paused for this query");
  await page.keyboard.press("r");
  await expect(page.locator(".query-recovery")).toBeVisible();
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(1);
});

// Back to the list, move to the thing with j or k, then open and run it.
async function runQuery(page, key) {
  await page.keyboard.press("Escape");
  if (key) await page.keyboard.press(key);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
}

test("a recovery in account A does not block the same query in account B", async ({ page }) => {
  await bootWatchlist(page, { things: [q("q-a", A, 0), q("q-b", B, 1)] });
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = true; });
  await runQuery(page);
  await expect(page.locator(".query-recovery")).toBeVisible();
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = false; });
  await runQuery(page, "j");
  await expect(page.locator(".wl-view-host tbody tr")).toHaveCount(1);
  await expect(page.locator(".query-recovery")).toHaveCount(0);
  const fetches = await calls(page, "widget_fetch", "logs-insights");
  expect(fetches.map(c => c.params.context.profile)).toEqual(["demo-a", "demo-b"]);
});

test("a recovery cleared in account B does not unblock the same query in account A", async ({ page }) => {
  await bootWatchlist(page, { things: [q("q-a", A, 0), q("q-b", B, 1)] });
  await page.evaluate(() => { window.__watchlistHarness.unknownQuery = true; });
  await runQuery(page);
  await expect(page.locator(".query-recovery")).toBeVisible();
  await runQuery(page, "j");
  await expect(page.locator(".query-recovery")).toBeVisible();
  await page.locator(".query-recovery select").selectOption("q-1");
  await page.getByLabel("I understand the earlier query may still be running").check();
  await page.getByRole("button", { name: "Review and run again" }).click();
  await expect(page.locator(".wl-view-host tbody tr")).toHaveCount(1);
  await expect(page.locator(".query-recovery")).toHaveCount(0);
  const before = (await calls(page, "widget_fetch", "logs-insights")).length;
  await runQuery(page, "k");
  await expect(page.locator(".query-recovery")).toContainText("Runs are paused for this query");
  expect(await calls(page, "widget_fetch", "logs-insights")).toHaveLength(before);
});

// Diagnostics.
test("a bridge error with a private payload shows only the fixed text", async ({ page }) => {
  const logs = [];
  page.on("console", message => logs.push(message.text()));
  page.on("pageerror", error => logs.push(error.message));
  await bootWatchlist(page, { things: [pipe] });
  await expect(rowText(page, "p")).toHaveText("Succeeded");
  await page.evaluate(() => { window.__watchlistHarness.widgets["pipeline-runs"] = () => { throw new Error("CB_SYNTHETIC_DIAGNOSTIC_SECRET"); }; });
  await page.keyboard.press("r");
  await expect(row(page, "p").locator(".wl-state-flag")).toHaveAttribute("title", "Desktop request failed. Try again.");
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view-host")).toContainText("Desktop request failed. Try again.");
  await expect(page.locator("body")).not.toContainText("CB_SYNTHETIC_DIAGNOSTIC_SECRET");
  expect(await page.locator("[title*='CB_SYNTHETIC']").count()).toBe(0);
  expect(logs.join("\n")).not.toContain("CB_SYNTHETIC_DIAGNOSTIC_SECRET");
});

test("a failed CLI result never gets a success badge", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("c", "cli", A, { command: "aws sts get-caller-identity" })] });
  await answer(page, "aws-cli", { ok: false, render: "raw_json", data: { error: "Synthetic cleanup was not confirmed" } });
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  const view = page.locator(".wl-view-host");
  await expect(view).toContainText("Synthetic cleanup was not confirmed");
  await expect(view.locator(".result-status")).toHaveAttribute("data-state", "failed");
  await expect(view.locator(".result-status")).not.toContainText("Updated");
  await expect(view.locator(".badge-success, .badge-ok, .badge.success")).toHaveCount(0);
  await expect(rowText(page, "c")).toHaveText("failed, press r to retry");
  await expect(row(page, "c").locator(".wl-row-state")).toHaveAttribute("data-tone", "fail");
});
