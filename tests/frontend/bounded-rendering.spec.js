import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const failures = new WeakMap();
const stacks = page => page.locator('.widget[data-widget="cfn-stacks"]');
const rows = page => stacks(page).locator('.cfn-stacks-body > table > tbody > tr:not(.row-detail)');
const filter = page => stacks(page).locator('.cfn-stacks-body > .table-filter-bar input');

function controls(options) {
  const NativeWorker = window.Worker;
  const fixture = window.__rendering = { started: 0, terminated: 0, special: options.special, calls: [] };
  window.Worker = function (...args) {
    fixture.started++;
    const worker = new NativeWorker(...args);
    const terminate = worker.terminate.bind(worker);
    let stopped = false;
    worker.terminate = () => { if (!stopped) fixture.terminated++; stopped = true; terminate(); };
    return worker;
  };
  const original = window.__TAURI__.core.invoke;
  window.__TAURI__.core.invoke = async (command, payload) => {
    const params = payload?.params || {};
    if (command === "widget_fetch") fixture.calls.push(structuredClone(params));
    const result = await original(command, payload);
    if (command === "widget_fetch" && params.widget === "cfn-stacks") {
      if (["pathological", "context"].includes(fixture.special)) result.rows[0].description = "a".repeat(5000) + "!";
      if (fixture.special === "context") result.rows = result.rows.map(row => ({ ...row, stack: `${row.stack}-${params.context?.profile || "inherited"}` }));
      if (fixture.special === "cell-details") {
        result.columns = ["stack", "nested", "withheld", "long"];
        result.rows.forEach((row, index) => Object.assign(row, { nested: "nested preview", withheld: "withheld preview", long: `row-${index}-` + "z".repeat(1500) }));
        result.cell_details = [
          { row: 100, column: "nested", value: { original_row: 100, synthetic: ["preserved", "detail"] } },
          { row: 100, column: "withheld", unavailable: true },
        ];
      }
    }
    return result;
  };
}

async function boot(page, count, special = null) {
  const errors = [];
  failures.set(page, errors);
  page.on("pageerror", () => errors.push("Production page error"));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    errors.push("Unexpected remote request"); return route.abort("blockedbyclient");
  });
  const fixture = { kind: "table", rows: count, delayMs: 0, fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(fixture)});(${controls.toString()})(${JSON.stringify({ special })});` });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(rows(page)).toHaveCount(Math.min(count, 100));
}

test.afterEach(async ({ page }) => {
  expect(failures.get(page)).toEqual([]);
  expect(await page.evaluate(() => window.__performanceFixture.errors)).toEqual([]);
});

for (const count of [100, 1000, 10000]) {
  test(`${count} source rows mount at most100 regular rows and filter across the full dataset`, async ({ page }) => {
    await boot(page, count);
    const target = `synthetic-stack-${String(count - 1).padStart(5, "0")}`;
    await filter(page).fill(target);
    await expect(filter(page)).toHaveAttribute("data-filter-pending", "false");
    await expect(stacks(page).locator(".table-filter-count")).toHaveText(`1 / ${count}`);
    await expect(rows(page).filter({ hasText: target })).toBeVisible();
    expect(await rows(page).count()).toBeLessThanOrEqual(100);
    await filter(page).fill("[");
    await expect(filter(page)).toHaveAttribute("aria-invalid", "true");
    await expect(stacks(page).locator(".table-filter-feedback")).toContainText("literal matches");
    await filter(page).fill("");
    await expect(stacks(page).locator(".table-filter-count")).toHaveText(String(count));
    await expect(rows(page)).toHaveCount(Math.min(count, 100));
  });
}

test("keyboard paging remembers expanded row identity without retaining off-page DOM", async ({ page }) => {
  await boot(page, 1000);
  const first = rows(page).first();
  await first.focus(); await first.press("Enter");
  await expect(stacks(page).locator(".row-detail")).toContainText("SyntheticLogGroup");
  const next = stacks(page).getByRole("button", { name: "Next rows", exact: true });
  await next.focus(); await next.press("Enter");
  await expect(rows(page).first()).toHaveAttribute("data-source-row", "100");
  await expect(stacks(page).locator(".row-detail")).toHaveCount(0);
  const previous = stacks(page).getByRole("button", { name: "Previous rows", exact: true });
  await previous.focus(); await previous.press("Enter");
  await expect(rows(page).first()).toHaveAttribute("aria-expanded", "true");
  await expect(stacks(page).locator(".row-detail")).toContainText("SyntheticLogGroup");
  await expect(rows(page)).toHaveCount(100);
});

test("full values and withheld CLI details stay associated with the original row after filtering", async ({ page }) => {
  await boot(page, 1000, "cell-details");
  await filter(page).fill("synthetic-stack-00100");
  await expect(stacks(page).locator(".table-filter-count")).toHaveText("1 / 1000");
  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-source-row", "100");
  expect((await row.getAttribute("aria-label")).length).toBeLessThanOrEqual(256);
  const longCell = row.locator("td").nth(3);
  expect((await longCell.innerText()).length).toBeLessThan(550);
  await row.getByLabel("Inspect full nested value").click();
  await expect(row.locator(".table-full-value")).toContainText('"original_row": 100');
  await row.getByLabel("Inspect full withheld value").click();
  await expect(row.locator(".table-full-value:visible")).toContainText("Use --query to return a smaller value");
  await row.getByLabel("Inspect full long value").click();
  await expect(row.locator(".table-full-value:visible")).toContainText("row-100-");
  expect(await row.locator("details[open]").count()).toBe(1);
  await expect(stacks(page).locator(".row-detail")).toHaveCount(0);
});

test("pathological regex cannot block the main thread and preserves the last valid matches", async ({ page }) => {
  await boot(page, 1000, "pathological");
  await filter(page).fill("synthetic-stack-00001");
  await expect(stacks(page).locator(".table-filter-count")).toHaveText("1 / 1000");
  await filter(page).fill("(a+)+$");
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("Escape");
  await expect(stacks(page).locator(".table-filter-feedback")).toContainText("150 ms work limit");
  await expect(stacks(page).locator(".table-filter-count")).toHaveText("1 / 1000");
  await expect(rows(page).first()).toContainText("synthetic-stack-00001");
  expect(await page.evaluate(() => window.__rendering.terminated)).toBeGreaterThan(0);
});

test("removing a filtering table terminates workers and repeated add/remove cycles leave no active workers", async ({ page }) => {
  await boot(page, 1000);
  for (let index = 0; index < 10; index++) {
    await filter(page).fill("synthetic-stack");
    await expect(filter(page)).toHaveAttribute("data-filter-pending", "false");
    await stacks(page).locator(".rm-btn").click();
    await expect(stacks(page)).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__rendering.started - window.__rendering.terminated)).toBe(0);
    if (index < 9) {
      await page.locator("#add-widget-btn").click();
      await page.locator(".prebuilt-item").filter({ has: page.locator(".prebuilt-name", { hasText: /^CloudFormation Stacks$/ }) }).locator(".prebuilt-add").click();
      await expect(rows(page)).toHaveCount(100);
    }
  }
});


test("changing verification context while filtering discards old matches and terminates its worker", async ({ page }) => {
  await boot(page, 1000, "context");
  await filter(page).fill("synthetic-stack-00001");
  await expect(stacks(page).locator(".table-filter-count")).toHaveText("1 / 1000");
  await filter(page).fill("(a+)+$");
  const account = page.getByRole("combobox", { name: "Default account" });
  await account.fill("synthetic-b");
  await account.press("Enter");
  await expect(page.locator("#account-select")).toHaveValue("synthetic-b");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(rows(page)).toHaveCount(100);
  await expect(filter(page)).toHaveValue("");
  await expect(stacks(page).locator(".table-filter-feedback")).toHaveText("");
  await expect.poll(() => page.evaluate(() => window.__rendering.started - window.__rendering.terminated)).toBe(0);
});


test("same-context refresh restores the focused filter and caret instead of a previously focused row", async ({ page }) => {
  await boot(page, 1000);
  await rows(page).first().focus();
  await filter(page).fill("synthetic-stack-000");
  await expect(filter(page)).toHaveAttribute("data-filter-pending", "false");
  await filter(page).evaluate(input => input.setSelectionRange(3, 8));
  await expect(filter(page)).toBeFocused();
  const originalFilter = await filter(page).elementHandle();
  await stacks(page).locator('.widget-header [title="Refresh"]').evaluate(button => button.click());
  // Wait for the response to replace the original control, so this cannot
  // pass by checking the retained input while refresh is still in flight.
  await expect.poll(() => originalFilter.evaluate(input => input.isConnected)).toBe(false);
  await expect(filter(page)).toHaveValue("synthetic-stack-000");
  await expect(filter(page)).toHaveAttribute("data-filter-pending", "false");
  await expect(filter(page)).toBeFocused();
  expect(await filter(page).evaluate(input => [input.selectionStart, input.selectionEnd])).toEqual([3, 8]);
});
