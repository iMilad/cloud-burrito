import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, rowText, thing } from "./watchlist-harness.mjs";

const lambda = thing("l-fn", "logs", A, { log_group: "/aws/lambda/synthetic-fn", source: "lambda", function_name: "synthetic-fn" });
const group = thing("l-group", "logs", A, { log_group: "/synthetic/group", source: "group" }, 1);

test("a logs row shows the newest event time from one stream read", async ({ page }) => {
  await bootWatchlist(page, { things: [lambda, group] });
  await expect(rowText(page, "l-fn")).toHaveText("last event");
  const reads = await calls(page, "widget_fetch");
  expect(reads.find(c => c.params.inputs.log_group === "/aws/lambda/synthetic-fn").params.widget).toBe("log-tail");
  expect(reads.find(c => c.params.inputs.log_group === "/synthetic/group").params.widget).toBe("cloudwatch-logs");
  expect(reads.every(c => c.params.inputs.mode === "streams" && c.params.inputs.max_streams === 1)).toBe(true);
});

test("Enter opens streams, then events, and n finds the error line", async ({ page }) => {
  await bootWatchlist(page, { things: [lambda] });
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-view")).toContainText("naming convention");
  await expect(page.locator(".wl-item")).toHaveCount(2);
  await page.keyboard.press("Enter");
  await expect(page.locator(".wl-crumb strong")).toHaveText("synthetic-stream-new");
  await expect(page.locator(".log-line")).toHaveCount(2);
  await page.keyboard.press("n");
  await expect(page.locator(".log-line.wl-current")).toContainText("ERROR synthetic timeout");
  const events = (await calls(page, "widget_fetch")).find(c => c.params.inputs.mode === "events");
  expect(events.params.inputs).toEqual({ mode: "events", log_group: "/aws/lambda/synthetic-fn", log_stream: "synthetic-stream-new", limit: 500 });
});
