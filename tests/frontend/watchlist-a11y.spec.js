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
