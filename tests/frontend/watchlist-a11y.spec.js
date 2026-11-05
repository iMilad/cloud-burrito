import { expect, test } from "@playwright/test";
import { A, B, bootWatchlist, thing } from "./watchlist-harness.mjs";

test("lists, rows, dialog and command line have names and roles", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" }),
    thing("a", "package", B, { domain: "d", repository: "r", package_prefix: "acme-core" })] });
  await expect(page.getByRole("listbox", { name: "Pipelines" })).toBeVisible();
  await expect(page.getByRole("option").first()).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("navigation", { name: "Inspector path" })).toBeAttached();
  await page.keyboard.press(":");
  await expect(page.getByRole("combobox", { name: "Command" })).toBeFocused();
  await page.keyboard.press("Escape");
  await page.keyboard.press("a");
  await expect(page.getByRole("dialog", { name: "Add a thing" })).toBeVisible();
});

test("text colors meet WCAG AA on their surfaces in both themes", async ({ page }) => {
  await bootWatchlist(page, { things: [] });
  for (const theme of ["dark", "light"]) {
    const ratios = await page.evaluate(theme => {
      document.documentElement.dataset.theme = theme;
      const style = getComputedStyle(document.body);
      const rgb = name => style.getPropertyValue(name).trim().match(/[0-9a-f]{2}/gi).map(h => parseInt(h, 16) / 255);
      const lum = c => { const [r, g, b] = c.map(v => v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
      const ratio = (a, b) => { const [x, y] = [lum(rgb(a)), lum(rgb(b))].sort((m, n) => n - m); return (x + 0.05) / (y + 0.05); };
      return ["--wl-ink", "--wl-muted", "--wl-accent", "--wl-ok", "--wl-fail", "--wl-run", "--wl-warn"]
        .flatMap(color => ["--wl-bg", "--wl-panel"].map(surface => ({ color, surface, value: ratio(color, surface) })));
    }, theme);
    for (const { color, surface, value } of ratios) expect(value, `${theme} ${color} on ${surface}`).toBeGreaterThanOrEqual(4.5);
  }
});

const focusables = panel => panel.locator('button:visible:not([disabled]):not([tabindex="-1"]), input:visible:not([disabled]):not([tabindex="-1"]), select:visible:not([disabled]):not([tabindex="-1"]), textarea:visible:not([disabled]):not([tabindex="-1"]), summary:visible:not([tabindex="-1"]), a[href]:visible:not([tabindex="-1"]), [tabindex="0"]:visible');

test("the settings panel keeps Tab inside and Escape returns focus to its opener", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  const panel = page.locator("#settings-panel");
  await expect(panel).toHaveAttribute("inert", "");
  const opener = page.locator("#settings-btn");
  await opener.focus();
  await page.keyboard.press("Enter");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  const controls = focusables(panel);
  await expect(controls.first()).toBeFocused();
  await controls.last().focus();
  await page.keyboard.press("Tab");
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(controls.last()).toBeFocused();
  for (let n = 0; n < 6; n++) {
    await page.keyboard.press("Tab");
    expect(await page.evaluate(() => !!document.activeElement.closest("#settings-panel"))).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(opener).toBeFocused();
});

test("Escape from a settings panel opened with a key returns focus to the selected row", async ({ page }) => {
  await bootWatchlist(page, { things: [thing("p", "pipeline", A, { pipeline_name: "synthetic-ok" })] });
  await page.locator('.wl-row[data-thing-id="p"]').focus();
  await page.keyboard.press(",");
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("Escape");
  await expect(page.locator('.wl-row[data-thing-id="p"]')).toBeFocused();
});
