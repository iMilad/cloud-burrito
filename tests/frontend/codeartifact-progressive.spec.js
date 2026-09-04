import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const failures = new WeakMap();
const surface = page => page.locator('.widget[data-widget="codeartifact-packages"]');
const rows = page => surface(page).locator('.codeartifact-packages-rows > table > tbody > tr:not(.row-detail)');
const calls = (page, mode) => page.evaluate(mode => window.__progressive.calls.filter(call => call.widget === "codeartifact-packages" && call.inputs.mode === mode), mode);

function installProgressiveControls() {
  const invoke = window.__TAURI__.core.invoke;
  const fixture = window.__progressive = { calls: [], held: [], cancellations: [], failPackage: null, repeatCursor: false };
  fixture.release = index => { fixture.held[index].resolve(); };
  window.__TAURI__.core.invoke = async (command, payload) => {
    const params = payload?.params || {};
    if (command === "request_cancel") fixture.cancellations.push(params.request_id);
    if (command === "widget_fetch") fixture.calls.push(structuredClone(params));
    if (command === "widget_fetch" && params.widget === "codeartifact-packages" && params.inputs.mode === "enrich") {
      await new Promise(resolve => fixture.held.push({ request_id: params.request_id, resolve }));
    }
    const result = await invoke(command, payload);
    if (command === "widget_fetch" && params.widget === "codeartifact-packages") {
      if (params.inputs.mode === "list") {
        result.rows = result.rows.slice(0, params.inputs.max_packages);
        result.coverage.counts.returned = result.rows.length;
        if (fixture.repeatCursor && params.inputs.page_token) result.next_page_token = params.inputs.page_token;
      }
      if (params.inputs.mode === "enrich") {
        result.rows.reverse(); // Deliberately complete metadata in another order.
        if (fixture.failPackage) {
          const failed = result.rows.find(row => row.package === fixture.failPackage);
          if (failed) {
            failed.enrichment_state = "failed";
            failed.enrichment_error = "Synthetic metadata failure";
            failed.last_published = "";
            result.ok = false; result.partial = true; result.error = "Synthetic metadata failure";
            result.coverage.completeness = "unknown";
            result.coverage.has_more = null;
          }
        }
      }
    }
    return result;
  };
}

async function boot(page, count = 1000) {
  const errors = [];
  failures.set(page, errors);
  page.on("pageerror", () => errors.push("Production page error"));
  await page.route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    errors.push("Unexpected remote request");
    return route.abort("blockedbyclient");
  });
  const options = { kind: "packages", rows: count, delayMs: 0, fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(options)});(${installProgressiveControls.toString()})();` });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await surface(page).getByRole("button", { name: "Load or refresh packages" }).click();
  await expect(rows(page)).toHaveCount(Math.min(count, 50));
  await expect.poll(() => page.evaluate(() => window.__progressive.held.length)).toBe(1);
}

test.afterEach(async ({ page }) => {
  expect(failures.get(page)).toEqual([]);
  expect(await page.evaluate(() => window.__performanceFixture.errors)).toEqual([]);
});

test("one thousand packages return one identity page before one visible metadata batch", async ({ page }) => {
  await boot(page);
  expect(await calls(page, "list")).toHaveLength(1);
  const batches = await calls(page, "enrich");
  expect(batches).toHaveLength(1);
  expect(batches[0].inputs.packages).toHaveLength(25);
  expect(batches[0].inputs).not.toHaveProperty("max_packages");
  expect(batches[0].inputs).not.toHaveProperty("page_token");
  await expect(rows(page).first()).toHaveAttribute("data-enrichment-state", "pending");
  await expect(surface(page).locator(".codeartifact-page-progress")).toContainText("50 of up to 1000");
  await rows(page).first().focus();
  await page.evaluate(() => {
    window.__rowIdentity = document.querySelector('tr[data-package="synthetic-package-00000"]');
    window.__progressive.release(0);
  });
  await expect(rows(page).first()).toHaveAttribute("data-enrichment-state", "complete");
  await expect(rows(page).nth(24)).toHaveAttribute("data-enrichment-state", "complete");
  await expect(rows(page).nth(25)).toHaveAttribute("data-enrichment-state", "pending");
  expect(await page.evaluate(() => window.__rowIdentity === document.querySelector('tr[data-package="synthetic-package-00000"]'))).toBe(true);
  await expect(rows(page).first()).toBeFocused();
  expect(await rows(page).evaluateAll(nodes => nodes.map(node => node.dataset.package))).toEqual(
    Array.from({ length: 50 }, (_, index) => `synthetic-package-${String(index).padStart(5, "0")}`));
  expect(await calls(page, "enrich")).toHaveLength(1);
});

test("failed metadata keeps successful rows and retries only failed package identities", async ({ page }) => {
  await boot(page, 50);
  await page.evaluate(() => { window.__progressive.failPackage = "synthetic-package-00001"; window.__progressive.release(0); });
  await expect(rows(page).nth(1)).toHaveAttribute("data-enrichment-state", "failed");
  await expect(rows(page).first()).toHaveAttribute("data-enrichment-state", "complete");
  await expect(surface(page).locator(".codeartifact-packages-rows > .result-status")).toContainText("Partial result");
  await surface(page).locator(".codeartifact-load-details").click();
  await expect.poll(() => page.evaluate(() => window.__progressive.held.length)).toBe(2);
  const retry = (await calls(page, "enrich"))[1].inputs.packages;
  expect(retry).toContain("synthetic-package-00001");
  expect(retry).not.toContain("synthetic-package-00000");
  expect(retry).toHaveLength(25);
  await page.evaluate(() => { window.__progressive.failPackage = null; window.__progressive.release(1); });
  await expect(rows(page).nth(1)).toHaveAttribute("data-enrichment-state", "complete");
});

test("reopening pending history shares enrichment and caches the completed history", async ({ page }) => {
  await boot(page, 50);
  const first = rows(page).first();
  await first.focus(); await first.press("Enter");
  await expect(surface(page).locator(".row-detail")).toContainText("Waiting for package details");
  await first.press("Enter"); await first.press("Enter");
  expect(await calls(page, "enrich")).toHaveLength(1);
  await page.evaluate(() => window.__progressive.release(0));
  await expect(surface(page).locator(".row-detail")).toContainText("Version history");
  const historyCalls = () => page.evaluate(() => window.__progressive.calls.filter(call => call.widget === "codeartifact-package-version-history").length);
  expect(await historyCalls()).toBe(1);
  await first.press("Enter"); await first.press("Enter");
  await expect(surface(page).locator(".row-detail")).toContainText("Version history");
  expect(await historyCalls()).toBe(1);
});

test("next page detaches old enrichment and a repeated cursor cannot loop", async ({ page }) => {
  await boot(page);
  const oldId = (await calls(page, "enrich"))[0].request_id;
  await page.evaluate(() => { window.__progressive.repeatCursor = true; });
  await surface(page).locator(".codeartifact-next-page").click();
  await expect(rows(page).first()).toHaveAttribute("data-package", "synthetic-package-00050");
  await expect.poll(() => page.evaluate(() => window.__progressive.cancellations)).toContain(oldId);
  await expect(surface(page).locator(".codeartifact-next-page")).toBeDisabled();
  await expect(surface(page).locator(".codeartifact-page-progress")).toContainText("Repeated continuation token");
  await page.evaluate(() => window.__progressive.release(0));
  await expect(rows(page).first()).toHaveAttribute("data-enrichment-state", "pending");
  await expect(rows(page).first()).not.toContainText("1.0.2");
  expect(await calls(page, "list")).toHaveLength(2);
});

test("editing inputs cancels enrichment and late metadata cannot restore old package rows", async ({ page }) => {
  await boot(page);
  const oldId = (await calls(page, "enrich"))[0].request_id;
  await surface(page).locator(".codeartifact-prefix").fill("different-synthetic-prefix");
  await expect.poll(() => page.evaluate(() => window.__progressive.cancellations)).toContain(oldId);
  await page.evaluate(() => window.__progressive.release(0));
  await expect(surface(page).locator(".codeartifact-packages-rows")).toBeHidden();
  await expect(rows(page)).toHaveCount(0);
  expect(await calls(page, "list")).toHaveLength(1);
});
