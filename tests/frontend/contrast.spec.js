import { expect, test } from "@playwright/test";
import { clickThemeToggle } from "./presentation-controls.mjs";

const KEY = "ui.contrast";
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

async function boot(page) {
  await page.goto("/");
  await expect(page.locator(".brand-tag")).toHaveText("browser mode");
  await expect(page.locator("html")).toHaveAttribute("data-contrast", "0");
}
async function openAppearance(page) {
  if (await page.locator("html").getAttribute("data-design") === "classic") await page.locator("#appearance-btn").click();
  else await page.locator('[data-studio-action="appearance"]').click();
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
}
async function adjust(page, value) {
  const slider = page.getByRole("slider", { name: "Contrast", exact: true });
  await slider.focus();
  await slider.press("Home");
  for (let level = -20; level < value; level += 5) await slider.press("ArrowRight");
  await expect(page.locator("html")).toHaveAttribute("data-contrast", String(value));
}
const palette = page => page.evaluate(() => {
  const root = getComputedStyle(document.documentElement);
  const tokens = names => Object.fromEntries(names.map(name => [name, root.getPropertyValue(name).trim()]));
  return {
    adjusted: tokens(["--text-primary", "--text-secondary", "--text-muted", "--border", "--border-strong"]),
    unchanged: tokens(["--bg", "--bg-elev", "--accent", "--success", "--warning", "--error"]),
    text: getComputedStyle(document.querySelector(".widget")).color,
    border: getComputedStyle(document.querySelector(".widget")).borderTopColor,
    filter: getComputedStyle(document.documentElement).filter,
  };
});

test("original is exact and stronger contrast preserves the palette across every style and theme", async ({ page }) => {
  await boot(page);
  await openAppearance(page);
  for (const design of ["studio", "classic"]) {
    for (const appearance of design === "studio" ? ["original", "precision", "paper", "night"] : ["paper"]) {
      for (const theme of ["light", "dark"]) {
        await page.evaluate(({ design, appearance, theme }) => {
          Object.assign(document.documentElement.dataset, { design, appearance, theme });
        }, { design, appearance, theme });
        const baseline = await palette(page);
        await adjust(page, 50);
        const stronger = await palette(page);
        expect(stronger.adjusted).not.toEqual(baseline.adjusted);
        expect(stronger.text).not.toEqual(baseline.text);
        expect(stronger.border).not.toEqual(baseline.border);
        expect(stronger.unchanged).toEqual(baseline.unchanged);
        expect(stronger.filter).toBe("none");
        await page.getByRole("button", { name: "Reset contrast", exact: true }).click();
        expect(await palette(page)).toEqual(baseline);
        expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBeNull();
      }
    }
  }
});

test("contrast persists and follows appearance, theme and Classic switches without accumulating changes", async ({ page }) => {
  await boot(page);
  await openAppearance(page);
  await adjust(page, 30);
  const paper = await palette(page);
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-contrast", "30");
  expect(await palette(page)).toEqual(paper);
  await openAppearance(page);
  await page.locator('[data-appearance-choice="night"]').click();
  const night = await palette(page);
  expect(night.adjusted).not.toEqual(paper.adjusted);
  await page.getByRole("button", { name: "Reset contrast", exact: true }).click();
  await adjust(page, 30);
  expect(await palette(page)).toEqual(night);
  await page.keyboard.press("Escape");
  await clickThemeToggle(page);
  const otherTheme = await palette(page);
  expect(otherTheme.adjusted).not.toEqual(night.adjusted);
  await openAppearance(page);
  await page.getByRole("button", { name: "Reset contrast", exact: true }).click();
  await adjust(page, 30);
  expect(await palette(page)).toEqual(otherTheme);
  await page.keyboard.press("Escape");
  await page.locator("#studio-classic-switch").click();
  await expect(page.locator("#appearance-btn")).toBeVisible();
  const classic = await palette(page);
  expect(classic.adjusted).not.toEqual(otherTheme.adjusted);
  await openAppearance(page);
  await page.getByRole("button", { name: "Reset contrast", exact: true }).click();
  await adjust(page, 30);
  expect(await palette(page)).toEqual(classic);
  await expect(page.locator("#contrast-status")).toContainText("saved on this device");
});

test("failed local writes keep live contrast usable and never claim persistence", async ({ page }) => {
  await page.addInitScript(key => {
    const set = Storage.prototype.setItem;
    const remove = Storage.prototype.removeItem;
    window.__blockContrastStorage = false;
    Storage.prototype.setItem = function (name, value) {
      if (name === key && window.__blockContrastStorage) throw new DOMException("Synthetic blocked preference", "QuotaExceededError");
      return set.call(this, name, value);
    };
    Storage.prototype.removeItem = function (name) {
      if (name === key && window.__blockContrastStorage) throw new DOMException("Synthetic blocked preference", "SecurityError");
      return remove.call(this, name);
    };
  }, KEY);
  await boot(page);
  const baseline = await palette(page);
  await openAppearance(page);
  await adjust(page, 50);
  await page.evaluate(() => { window.__blockContrastStorage = true; });
  await adjust(page, -20);
  expect((await palette(page)).adjusted).not.toEqual(baseline.adjusted);
  await expect(page.locator("#contrast-status")).toContainText("for this session");
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("50");
  await page.getByRole("button", { name: "Reset contrast", exact: true }).click();
  expect(await palette(page)).toEqual(baseline);
  await expect(page.locator("#contrast-status")).toContainText("may not survive a restart");
  await page.evaluate(() => { window.__blockContrastStorage = false; });
  await adjust(page, 20);
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("20");
  await expect(page.locator("#contrast-status")).toContainText("saved on this device");
});

test("invalid stored contrast falls back to the original design without rewriting storage", async ({ page }) => {
  await page.addInitScript(key => { localStorage.setItem(key, "999"); }, KEY);
  await boot(page);
  await openAppearance(page);
  await expect(page.getByRole("slider", { name: "Contrast", exact: true })).toHaveValue("0");
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("999");
  expect(await page.evaluate(() => document.documentElement.style.getPropertyValue("--text-muted"))).toBe("");
});
