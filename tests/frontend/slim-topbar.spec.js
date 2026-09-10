import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";
import { clickThemeToggle } from "./presentation-controls.mjs";

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
  await page.setViewportSize({ width: 1280, height: 900 });
});
test.afterEach(async ({ page }) => { expect(failures.get(page)).toEqual([]); });

async function boot(page, native = false) {
  if (native) {
    await page.clock.install({ time: new Date("2026-09-10T08:00:00Z") });
    const fixture = { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 1, delayMs: 0 };
    // The only native boundary is synthetic IPC. No real profiles, processes,
    // credential files, provider requests, or external connections are used.
    await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(fixture)});
      const syntheticInvoke = window.__TAURI__.core.invoke;
      window.__TAURI__.core.invoke = async (command, payload) => {
        const response = await syntheticInvoke(command, payload);
        return command === "aws_auth_status"
          ? { ...response, sso_token_expires_at: "2026-09-10T12:00:00Z", expires_at: "2026-09-10T20:00:00Z" }
          : response;
      };` });
  }
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  if (native) {
    await expect(page.locator("#auth-status")).toHaveText("AWS verified · acct-a-fixture · SSO token: 4h left");
    await expect(page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body')).toContainText("synthetic-stack-00000");
    await expect.poll(() => page.evaluate(() => window.__performanceFixture.active)).toBe(0);
  } else await expect(page.locator(".brand-tag")).toHaveText("browser mode");
}

const heroGeometry = page => page.locator("#studio-overview").evaluate(node => {
  const { width, height } = node.getBoundingClientRect();
  // Transient notices can move the hero without changing its accepted design.
  return [width, height].map(Math.round);
});
async function expectHero(page) {
  await expect(page.locator("#studio-heading")).toContainText("Less noise.");
  await expect(page.locator("#studio-heading")).toContainText("More signal.");
  await expect(page.locator("#studio-overview .studio-route-art")).toBeVisible();
  for (const name of ["pipeline-runs", "cfn-stacks", "log-tail"]) {
    await expect(page.locator(`#studio-overview [data-studio-jump="${name}"]`)).toBeVisible();
  }
}

test("desktop context fields have aligned stacked labels and an explicit compact demo indicator", async ({ page }) => {
  await boot(page);
  await expect(page.locator("#studio-demo-status")).toBeVisible();
  await expect(page.locator("#studio-demo-status")).toHaveText("Demo");
  await expect(page.locator("#core-status")).toBeHidden();
  await expect(page.locator("#core-status .core-label")).toHaveText("browser mode");
  await expect(page.locator("#account-picker-search")).toHaveValue("Demo workspace");
  const fields = await page.locator(".topbar .picker").evaluateAll(nodes => nodes.map(node => {
    const picker = node.getBoundingClientRect();
    const label = node.querySelector("label").getBoundingClientRect();
    const value = node.querySelector("input").getBoundingClientRect();
    const style = getComputedStyle(node);
    const bounds = rect => ({ x: rect.x, right: rect.right, top: rect.top, bottom: rect.bottom });
    return { picker: bounds(picker), label: bounds(label), value: bounds(value),
      background: style.backgroundColor, border: style.borderTopColor };
  }));
  expect(fields).toHaveLength(2);
  for (const field of fields) {
    expect(field.label.bottom).toBeLessThanOrEqual(field.value.top + 1);
    expect(Math.abs(field.label.x - field.value.x)).toBeLessThanOrEqual(2);
    for (const child of [field.label, field.value]) {
      expect(child.x).toBeGreaterThanOrEqual(field.picker.x);
      expect(child.right).toBeLessThanOrEqual(field.picker.right);
      expect(child.top).toBeGreaterThanOrEqual(field.picker.top);
      expect(child.bottom).toBeLessThanOrEqual(field.picker.bottom);
    }
    expect(field.background).toBe("rgba(0, 0, 0, 0)");
    expect(field.border).toBe("rgba(0, 0, 0, 0)");
  }
  expect(Math.abs(fields[0].label.top - fields[1].label.top)).toBeLessThanOrEqual(1);
  expect(Math.abs(fields[0].value.top - fields[1].value.top)).toBeLessThanOrEqual(1);
  expect((await page.locator(".topbar").boundingBox()).height).toBeLessThanOrEqual(62);
  await expectHero(page);
});

test("workspace options supports keyboard opening, Escape and outside dismissal without exiting fullscreen", async ({ page }) => {
  await boot(page);
  const menu = page.locator("#studio-workspace-menu"), summary = page.locator("#studio-workspace-menu-toggle");
  const reset = page.locator("#reset-layout-btn");
  await expect(summary).toHaveAccessibleName("Workspace options");
  await expect(summary).toHaveAttribute("aria-expanded", "false");
  await expect(reset).toBeHidden();
  await summary.focus();
  await summary.press("Enter");
  await expect(menu).toHaveAttribute("open", "");
  await expect(summary).toHaveAttribute("aria-expanded", "true");
  await reset.focus();
  await expect(reset).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).not.toHaveAttribute("open", "");
  await expect(summary).toBeFocused();
  await summary.click();
  await page.locator("#studio-heading").click();
  await expect(menu).not.toHaveAttribute("open", "");
  await expect(summary).toHaveAttribute("aria-expanded", "false");

  await summary.click();
  await page.locator("#studio-menu-appearance").click();
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
  await expect(menu).not.toHaveAttribute("open", "");
  await page.keyboard.press("Escape");
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "true");
  await expect(summary).toBeFocused();

  const widget = page.locator('.widget[data-widget="cfn-stacks"]');
  await widget.locator(".fs-btn").click();
  await expect(widget).toHaveClass(/fullscreen/);
  await summary.click();
  await reset.focus();
  await page.keyboard.press("Escape");
  await expect(menu).not.toHaveAttribute("open", "");
  await expect(summary).toBeFocused();
  await expect(widget).toHaveClass(/fullscreen/);
  await page.keyboard.press("Escape");
  await expect(widget).not.toHaveClass(/fullscreen/);
});

test("the original theme and reset controls remain functional through a Classic roundtrip", async ({ page }) => {
  await boot(page);
  await page.locator("#theme-toggle, #reset-layout-btn").evaluateAll(nodes => {
    window.__syntheticOriginalControls = nodes;
  });
  await expect(page.locator("#appearance-theme-slot > #theme-toggle")).toHaveCount(1);
  await expect(page.locator("#studio-workspace-menu-panel #reset-layout-btn")).toHaveCount(1);
  await expect(page.locator("#theme-toggle")).toBeHidden();
  await clickThemeToggle(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  // Toggling while Appearance is already open must preserve dialog focus and
  // containment, rather than closing the caller's active panel.
  await page.locator('[data-studio-action="appearance"]:visible, #appearance-btn:visible').first().click();
  await clickThemeToggle(page);
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
  await expect(page.locator("#theme-toggle")).toBeFocused();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.keyboard.press("Escape");
  await page.locator("#studio-classic-switch").click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  for (const id of ["theme-toggle", "reset-layout-btn"]) await expect(page.locator(`.topbar-right > #${id}`)).toBeVisible();
  await expect(page.locator("#studio-workspace-menu")).toBeHidden();
  await clickThemeToggle(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.locator("#studio-return").click();
  await expect(page.locator("#appearance-theme-slot > #theme-toggle")).toHaveCount(1);
  await expect(page.locator("#studio-workspace-menu-panel #reset-layout-btn")).toHaveCount(1);
  await expect(page.locator("#theme-toggle")).toHaveCount(1);
  await expect(page.locator("#reset-layout-btn")).toHaveCount(1);
  expect(await page.evaluate(() => window.__syntheticOriginalControls.every(node => document.getElementById(node.id) === node))).toBe(true);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("native account and token context remains visible while the accepted hero stays unchanged", async ({ page }) => {
  await boot(page, true);
  const auth = page.locator("#auth-status"), connection = page.locator("#connection-toggle");
  await expect(auth).toBeVisible();
  await expect(connection).toBeVisible();
  await expect(connection).toHaveAccessibleName("Connection details");
  await expect(page.locator("#studio-demo-status")).toBeHidden();
  await page.locator("#connection-close").click();
  await expect(connection).toBeFocused();
  await expect(auth).toBeVisible();
  await expectHero(page);
  const before = await heroGeometry(page);
  const readCount = () => page.evaluate(() => window.__performanceFixture.commands.aws_set_account);
  const selections = await readCount();
  await clickThemeToggle(page);
  expect(await readCount()).toBe(selections);
  const account = page.getByRole("combobox", { name: "Default account" });
  await account.fill("synthetic-b");
  await account.press("Enter");
  await expect(auth).toHaveText("AWS verified · acct-b-fixture · SSO token: 4h left");
  const region = page.getByRole("combobox", { name: "Default region" });
  await region.fill("us-east-1");
  await region.press("Enter");
  await expect(page.locator("#region-select")).toHaveValue("us-east-1");
  await expect(auth).toHaveText("AWS verified · acct-b-fixture · SSO token: 4h left");
  await expect.poll(() => heroGeometry(page)).toEqual(before);
  await page.clock.fastForward(15 * 60_000);
  await expect(auth).toHaveText("AWS verified · acct-b-fixture · SSO token: 3h 45m left");
  await expect(auth).toBeVisible();
  await expect(connection).toBeVisible();
  expect((await page.locator(".topbar").boundingBox()).height).toBeLessThanOrEqual(62);
  await expectHero(page);
  expect(await page.evaluate(() => window.__performanceFixture.errors)).toEqual([]);
});
