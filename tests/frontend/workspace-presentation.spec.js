import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const failures = new WeakMap();
test.beforeEach(async ({ page }) => {
  const errors = [];
  failures.set(page, errors);
  page.on("pageerror", error => errors.push(error.message));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    errors.push("Unexpected remote request");
    return route.abort("blockedbyclient");
  });
});
test.afterEach(async ({ page }) => { expect(failures.get(page)).toEqual([]); });

async function boot(page, { native = false, blockStorage = false } = {}) {
  await page.clock.install();
  if (native) {
    await page.addInitScript(installSyntheticBridge, {
      fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 1, delayMs: 0,
    });
  }
  if (blockStorage) {
    await page.addInitScript(() => {
      const get = Storage.prototype.getItem, set = Storage.prototype.setItem;
      const blocked = key => ["ui.density", "ui.overview"].includes(key);
      Storage.prototype.getItem = function (key) {
        if (this === localStorage && blocked(key)) throw new DOMException("Synthetic preference storage unavailable", "SecurityError");
        return get.call(this, key);
      };
      Storage.prototype.setItem = function (key, value) {
        if (this === localStorage && blocked(key)) throw new DOMException("Synthetic preference storage unavailable", "SecurityError");
        return set.call(this, key, value);
      };
    });
  }
  await page.goto("/");
  if (native) {
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
    await expect(page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body')).toContainText("synthetic-stack-00000");
    await expect.poll(() => page.evaluate(() => window.__performanceFixture.active)).toBe(0);
  } else {
    await expect(page.locator(".brand-tag")).toHaveText("browser mode");
  }
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
}

const stored = (page, key) => page.evaluate(key => localStorage.getItem(key), key);
async function openAppearance(page) {
  const opener = page.locator('[data-studio-action="appearance"]:visible, #appearance-btn:visible').first();
  await opener.click();
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
  return opener;
}

test("the default workspace is compact and the expanded overview explains its sample data", async ({ page }) => {
  await boot(page);
  const root = page.locator("html"), overview = page.locator("#studio-overview");
  await expect(root).toHaveAttribute("data-density", "compact");
  await expect(root).toHaveAttribute("data-overview", "compact");
  await expect(page.locator("#studio-density")).toHaveAttribute("aria-pressed", "true");
  await expect(overview).toBeVisible();
  await expect(page.locator("#studio-mode-label")).toHaveText("Demo · sample data");
  await expect(page.locator("#studio-mode-label")).toHaveAttribute("title", /Sample data only\. No AWS connection\./);
  await expect(page.locator("#studio-demo-description")).toBeVisible();
  await expect(page.locator("#studio-demo-description")).toHaveText("Sample data only. No AWS connection.");
  const compactHeight = (await overview.boundingBox()).height;
  await page.locator("#studio-overview-toggle").focus();
  await page.keyboard.press("Enter");
  await expect(root).toHaveAttribute("data-overview", "expanded");
  await expect(page.locator("#studio-overview-toggle")).toBeFocused();
  await expect(page.locator("#studio-demo-description")).toBeVisible();
  await expect(page.locator("#studio-demo-description")).toHaveText("Sample data only. No AWS connection.");
  await expect.poll(async () => (await overview.boundingBox()).height).toBeGreaterThan(compactHeight);
  expect(await stored(page, "ui.overview")).toBe("expanded");
  await page.reload();
  await expect(root).toHaveAttribute("data-overview", "expanded");
  await expect(page.locator("#studio-demo-description")).toBeVisible();
  await page.locator("#studio-overview-toggle").click();
  await expect(root).toHaveAttribute("data-overview", "compact");
});

test("opting out of compact density persists independently of overview and appearance choices", async ({ page }) => {
  await boot(page);
  const density = page.locator("#studio-density"), root = page.locator("html");
  await density.click();
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await expect(density).toHaveAttribute("aria-pressed", "false");
  expect(await stored(page, "ui.density")).toBe("comfortable");
  await page.reload();
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await expect(density).toHaveAttribute("aria-pressed", "false");
  await page.locator("#studio-overview-toggle").click();
  await openAppearance(page);
  await page.locator('[data-appearance-choice="precision"]').click();
  await page.keyboard.press("Escape");
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await expect(root).toHaveAttribute("data-overview", "expanded");
  await density.click();
  await expect(density).toHaveAttribute("aria-pressed", "true");
  expect(await stored(page, "ui.density")).toBe("compact");
  await page.reload();
  await expect(root).toHaveAttribute("data-density", "compact");
  await expect(root).toHaveAttribute("data-overview", "expanded");
  await expect(root).toHaveAttribute("data-appearance", "precision");
});

test("a hidden overview persists and can be restored from the toolbar or Appearance without losing keyboard focus", async ({ page }) => {
  await boot(page);
  const overview = page.locator("#studio-overview"), show = page.locator("#studio-overview-show");
  await page.locator("#studio-overview-hide").focus();
  await page.keyboard.press("Enter");
  await expect(overview).toBeHidden();
  await expect(show).toBeFocused();
  await expect(show).toBeVisible();
  expect(await stored(page, "ui.overview")).toBe("hidden");
  await expect(page.locator("#studio-workspace-mode")).toHaveText("Demo · sample data");
  await expect(page.locator("#studio-workspace-mode")).toBeVisible();
  await page.reload();
  await expect(overview).toBeHidden();
  await expect(show).toBeVisible();
  await page.locator('#studio-nav [data-studio-jump="overview"]').click();
  await expect(page.locator("#studio-workspace-heading")).toBeFocused();
  await expect(overview).toBeHidden();
  expect(await stored(page, "ui.overview")).toBe("hidden");
  await show.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-overview", "compact");
  await expect(page.locator("#studio-overview-toggle")).toBeFocused();
  await expect(overview).toBeVisible();

  const opener = await openAppearance(page);
  const select = page.locator("#appearance-overview");
  await select.focus();
  await select.selectOption("hidden");
  await expect(select).toBeFocused();
  await expect(overview).toBeHidden();
  await select.selectOption("expanded");
  await expect(select).toBeFocused();
  await expect(page.locator("html")).toHaveAttribute("data-overview", "expanded");
  await page.keyboard.press("Escape");
  await expect(opener).toBeFocused();
  await expect(overview).toBeVisible();
});

test("workspace presentation changes make no native requests and retain the verified desktop indicator", async ({ page }) => {
  await boot(page, { native: true });
  await page.clock.fastForward(600);
  const before = await page.evaluate(() => ({ ...window.__performanceFixture.commands }));
  await page.locator("#studio-density").click();
  await page.locator("#studio-overview-toggle").click();
  await expect(page.locator("#studio-demo-description")).toBeHidden();
  await page.locator("#studio-overview-hide").click();
  await expect(page.locator("#studio-workspace-mode")).toBeVisible();
  await expect(page.locator("#studio-workspace-mode")).toHaveText("Desktop · identity verified");
  await expect(page.locator("#studio-workspace-mode")).toHaveAttribute("data-state", "verified");
  await page.locator("#studio-overview-show").click();
  await openAppearance(page);
  await page.locator("#appearance-overview").selectOption("hidden");
  await page.keyboard.press("Escape");
  await page.clock.fastForward(600);
  expect(await page.evaluate(() => ({ ...window.__performanceFixture.commands }))).toEqual(before);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body')).toContainText("synthetic-stack-00000");
});

test("blocked preference storage leaves controls usable and reports session-only choices", async ({ page }) => {
  await boot(page, { blockStorage: true });
  await page.locator("#studio-density").click();
  await page.locator("#studio-overview-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-density", "comfortable");
  await expect(page.locator("html")).toHaveAttribute("data-overview", "expanded");
  await openAppearance(page);
  await expect(page.locator("#workspace-preferences-status")).toHaveAttribute("role", "status");
  await expect(page.locator("#workspace-preferences-status")).toBeVisible();
  await expect(page.locator("#workspace-preferences-status")).toContainText(/session/i);
  await page.locator("#appearance-overview").selectOption("hidden");
  await page.keyboard.press("Escape");
  await expect(page.locator("#studio-overview")).toBeHidden();
  await page.locator("#studio-overview-show").click();
  await expect(page.locator("html")).toHaveAttribute("data-overview", "compact");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expect(page.locator("html")).toHaveAttribute("data-overview", "compact");
});
