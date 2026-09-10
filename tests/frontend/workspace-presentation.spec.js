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

async function boot(page, { nativeDisconnected = false, blockStorage = false, legacyOverview } = {}) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.clock.install();
  if (nativeDisconnected) {
    const options = { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 1, delayMs: 0 };
    // Synthetic native IPC only. A missing synthetic configuration prevents
    // verification; no actual profile, credential file, CLI, or AWS is used.
    await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(options)});
      const syntheticInvoke = window.__TAURI__.core.invoke;
      window.__TAURI__.core.invoke = async (command, payload) => {
        const response = await syntheticInvoke(command, payload);
        return command === "aws_list_profiles"
          ? { discovery_state: "missing_config", file_exists: false, profiles: [] }
          : response;
      };` });
  }
  if (legacyOverview) await page.addInitScript(value => {
    if (!sessionStorage.getItem("synthetic-overview-seeded")) {
      localStorage.setItem("ui.overview", value);
      sessionStorage.setItem("synthetic-overview-seeded", "true");
    }
  }, legacyOverview);
  if (blockStorage) {
    await page.addInitScript(() => {
      const get = Storage.prototype.getItem, set = Storage.prototype.setItem;
      Storage.prototype.getItem = function (key) {
        if (this === localStorage && key === "ui.density") throw new DOMException("Synthetic preference storage unavailable", "SecurityError");
        return get.call(this, key);
      };
      Storage.prototype.setItem = function (key, value) {
        if (this === localStorage && key === "ui.density") throw new DOMException("Synthetic preference storage unavailable", "SecurityError");
        return set.call(this, key, value);
      };
    });
  }
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  if (nativeDisconnected) {
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "missing_config");
    await expect.poll(() => page.evaluate(() => window.__performanceFixture.active)).toBe(0);
  } else {
    await expect(page.locator(".brand-tag")).toHaveText("browser mode");
  }
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
}

const stored = (page, key) => page.evaluate(key => localStorage.getItem(key), key);
const hero = page => page.locator("#studio-overview");
const heroHeight = async page => Math.round((await hero(page).boundingBox()).height);
async function expectFullHero(page) {
  await expect(hero(page)).toBeVisible();
  await expect(page.locator("#studio-heading")).toContainText("Less noise.");
  await expect(page.locator("#studio-heading")).toContainText("More signal.");
  await expect(page.locator("#studio-heading")).toBeVisible();
  await expect(hero(page).locator(".studio-route-art")).toBeVisible();
  const shortcuts = hero(page).locator("[data-studio-jump]");
  await expect(shortcuts).toHaveCount(3);
  for (const type of ["pipeline-runs", "cfn-stacks", "log-tail"]) {
    await expect(hero(page).locator(`[data-studio-jump="${type}"]`)).toBeVisible();
  }
  await expect(page.locator("#studio-overview-toggle, #studio-overview-hide, #studio-overview-show, #appearance-overview")).toHaveCount(0);
  await expect.poll(() => heroHeight(page)).toBeLessThanOrEqual(200);
}
async function openAppearance(page) {
  const opener = page.locator('[data-studio-action="appearance"]:visible, #appearance-btn:visible').first();
  await opener.click();
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
  return opener;
}

test("the full short hero ignores obsolete hidden and expanded overview preferences", async ({ page }) => {
  await boot(page, { legacyOverview: "hidden" });
  await expectFullHero(page);
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expect(page.locator("#studio-density")).toHaveAttribute("aria-pressed", "true");
  await page.locator('#studio-nav [data-studio-jump="overview"]').click();
  await expect(page.locator("#studio-heading")).toBeFocused();
  await page.evaluate(() => localStorage.setItem("ui.overview", "expanded"));
  await page.reload();
  await expectFullHero(page);
});

test("compact density defaults on and its saved opt-out never changes the hero", async ({ page }) => {
  await boot(page);
  const root = page.locator("html"), density = page.locator("#studio-density");
  await expectFullHero(page);
  const height = await heroHeight(page);
  await density.click();
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await expect(density).toHaveAttribute("aria-pressed", "false");
  expect(await stored(page, "ui.density")).toBe("comfortable");
  await expect.poll(() => heroHeight(page)).toBe(height);
  await page.reload();
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await expect(density).toHaveAttribute("aria-pressed", "false");
  await expectFullHero(page);
  await expect.poll(() => heroHeight(page)).toBe(height);
  await density.click();
  await expect(density).toHaveAttribute("aria-pressed", "true");
  expect(await stored(page, "ui.density")).toBe("compact");
  await page.reload();
  await expect(root).toHaveAttribute("data-density", "compact");
  await expectFullHero(page);
});

test("demo mode is explicit and all three retained hero shortcuts navigate to their existing tools", async ({ page }) => {
  await boot(page);
  await expect(page.locator("#studio-mode-label")).toHaveText("Demo active");
  await expect(page.locator("#studio-mode-label")).toHaveAttribute("data-state", "demo");
  await expect(page.locator("#studio-demo-description")).toBeVisible();
  await expect(page.locator("#studio-demo-description")).toHaveText("Sample data · no AWS connection");
  const count = await page.locator("#grid-stack > .grid-stack-item").count();
  for (const type of ["pipeline-runs", "cfn-stacks", "log-tail"]) {
    const shortcut = hero(page).locator(`[data-studio-jump="${type}"]`);
    await expect(shortcut).toBeEnabled();
    await shortcut.focus();
    await shortcut.press("Enter");
    await expect(page.locator(`.widget[data-widget="${type}"]`)).toBeFocused();
  }
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(count);
  await openAppearance(page);
  await expect(page.locator("#appearance-overview")).toHaveCount(0);
});

test("a disconnected native workspace never claims demo mode and density changes do not invoke native work", async ({ page }) => {
  await boot(page, { nativeDisconnected: true });
  await expectFullHero(page);
  const mode = page.locator("#studio-mode-label");
  await expect(mode).toHaveText("Desktop · identity required");
  await expect(mode).toHaveAttribute("data-state", "unverified");
  await expect(page.locator("#studio-demo-description")).toBeHidden();
  await page.clock.fastForward(600);
  const before = await page.evaluate(() => ({ ...window.__performanceFixture.commands }));
  await page.locator("#studio-density").click();
  await page.clock.fastForward(600);
  expect(await page.evaluate(() => ({ ...window.__performanceFixture.commands }))).toEqual(before);
  await expect(mode).toHaveText("Desktop · identity required");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "missing_config");
  expect(before.aws_set_account || 0).toBe(0);
});

test("blocked density storage leaves the full hero and controls usable with a session-only notice", async ({ page }) => {
  await boot(page, { blockStorage: true });
  await expectFullHero(page);
  const density = page.locator("#studio-density");
  await density.click();
  await expect(page.locator("html")).toHaveAttribute("data-density", "comfortable");
  await expect(density).toHaveAttribute("aria-pressed", "false");
  await expectFullHero(page);
  await openAppearance(page);
  await expect(page.locator("#workspace-preferences-status")).toHaveAttribute("role", "status");
  await expect(page.locator("#workspace-preferences-status")).toBeVisible();
  await expect(page.locator("#workspace-preferences-status")).toContainText(/session/i);
  await page.keyboard.press("Escape");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expectFullHero(page);
});
