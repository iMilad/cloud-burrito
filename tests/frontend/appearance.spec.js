import { expect, test } from "@playwright/test";
import { clickThemeToggle } from "./presentation-controls.mjs";

const KEY = "cb.studio.appearance.v1";
const choices = ["original", "precision", "paper", "night"];
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

test.afterEach(async ({ page }) => {
  expect(failures.get(page)).toEqual([]);
});

async function boot(page, url = "/") {
  await page.goto(url);
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  await expect(page.locator("#studio-mode-label")).toHaveText("Demo active");
}

async function openAppearance(page) {
  const opener = page.locator('[data-studio-action="appearance"]:visible, #appearance-btn:visible').first();
  await opener.click();
  await expect(page.locator("#appearance-panel")).toHaveAttribute("aria-hidden", "false");
  return opener;
}

test("a stored style updates the mark before an asynchronous native boot completes", async ({ page }) => {
  await page.addInitScript(key => {
    localStorage.setItem(key, "precision");
    window.__TAURI__ = { core: { invoke: () => new Promise(() => {}) } };
  }, KEY);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
  await expect(page.locator("[data-studio-mark]").first()).toHaveAttribute("src", /style-precision\.svg$/);
  await expect(page.locator("#app-icon")).toHaveAttribute("href", /style-precision\.svg$/);
});

test("Classic uses the current mark and can return to Studio while native settings are pending", async ({ page }) => {
  await page.addInitScript(key => {
    localStorage.setItem("cb.presentation.v1", "classic");
    localStorage.setItem(key, "night");
    window.__TAURI__ = { core: { invoke: () => new Promise(() => {}) } };
  }, KEY);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.locator(".brand-mark-classic")).toBeVisible();
  await expect(page.locator(".brand-mark-classic")).toHaveAttribute("src", /style-night\.svg$/);
  await page.getByRole("button", { name: "Studio view", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");
  await expect(page.locator("#studio-classic-switch")).toBeFocused();
  expect(await page.evaluate(() => localStorage.getItem("cb.presentation.v1"))).toBe("studio");
  await page.locator("#studio-classic-switch").click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await expect(page.locator("#studio-return")).toBeFocused();
});

test("Paper is the safe default and every allowlisted style persists its matching in-app mark", async ({ page }) => {
  await boot(page);
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "paper");
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBeNull();
  await openAppearance(page);

  for (const choice of choices) {
    const control = page.locator(`[data-appearance-choice="${choice}"]`);
    await control.click();
    await expect(page.locator("html")).toHaveAttribute("data-appearance", choice);
    await expect(control).toHaveAttribute("aria-checked", "true");
    await expect(page.locator('[data-appearance-choice][aria-checked="true"]')).toHaveCount(1);
    await expect(page.locator("[data-studio-mark]").first()).toHaveAttribute("src", new RegExp(`style-${choice === "original" ? "current" : choice}\\.svg$`));
  }
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("night");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");

  await boot(page, "/?appearance=precision");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
  await expect(page.locator("#appearance-status")).toContainText("URL preview");
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("night");
  await boot(page, "/?appearance=synthetic-unknown-style");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");

  await page.evaluate(key => localStorage.setItem(key, "synthetic-unknown-style"), KEY);
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "paper");
  expect(await page.evaluate(key => localStorage.getItem(key), KEY)).toBe("synthetic-unknown-style");
});

test("style, color theme and Classic view remain independent", async ({ page }) => {
  await boot(page);
  const theme = await page.locator("html").getAttribute("data-theme");
  await openAppearance(page);
  await page.locator('[data-appearance-choice="precision"]').click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await page.keyboard.press("Escape");

  await clickThemeToggle(page);
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
  await page.locator("#studio-classic-switch").click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await expect(page.locator(".brand-mark-classic")).toBeVisible();
  await expect(page.locator(".brand-mark-classic")).toHaveAttribute("src", /style-precision\.svg$/);
  await expect(page.locator(".brand-mark-studio")).toBeHidden();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
  await page.locator("#studio-return").click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
});

test("appearance behaves as a keyboard radio group and returns focus on Escape", async ({ page }) => {
  await boot(page);
  const opener = await openAppearance(page);
  const paper = page.locator('[data-appearance-choice="paper"]');
  await paper.focus();
  await paper.press("ArrowRight");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");
  await expect(page.locator('[data-appearance-choice="night"]')).toBeFocused();
  await page.keyboard.press("Home");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "original");
  await expect(page.locator('[data-appearance-choice="original"]')).toBeFocused();
  await page.keyboard.press("End");
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");
  await page.keyboard.press("Escape");
  await expect(page.locator("#appearance-panel")).toHaveAttribute("inert", "");
  await expect(opener).toBeFocused();
});

test("the appearance chooser fits and stays operable in the narrow responsive layout", async ({ page }) => {
  await page.setViewportSize({ width: 500, height: 700 });
  await boot(page);
  await openAppearance(page);
  const panel = page.locator("#appearance-panel");
  await expect.poll(async () => {
    const box = await panel.boundingBox();
    return box ? { left: Math.round(box.x), right: Math.round(box.x + box.width) } : null;
  }).toEqual({ left: 40, right: 500 });
  await expect(page.locator(".appearance-swatches").first()).toBeHidden();
  await page.locator('[data-appearance-choice="night"]').click();
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "night");
  await expect(page.locator("#appearance-panel-close")).toBeVisible();
  expect(await panel.evaluate(node => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(1);
});

test("all eight style and color-theme combinations fit desktop and compact layouts", async ({ page }) => {
  for (const width of [1480, 1024]) {
    await page.setViewportSize({ width, height: 900 });
    await boot(page);
    await openAppearance(page);
    for (const choice of choices) {
      await page.locator(`[data-appearance-choice="${choice}"]`).click();
      for (let iteration = 0; iteration < 2; iteration++) {
        const theme = await page.locator("html").getAttribute("data-theme");
        expect(["light", "dark"]).toContain(theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
        await expect.poll(() => page.locator("[data-studio-mark]").evaluateAll(images => images.every(image => image.complete && image.naturalWidth > 0))).toBe(true);
        await page.keyboard.press("Escape");
        await clickThemeToggle(page);
        await openAppearance(page);
      }
    }
    await page.keyboard.press("Escape");
  }
});
