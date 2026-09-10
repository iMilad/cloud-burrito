import { expect } from "@playwright/test";

// Use the real presentation controls in their current location. Opening the
// Appearance panel here must not close a panel the caller already had open.
export async function clickThemeToggle(page) {
  const toggle = page.locator("#theme-toggle");
  const panel = page.locator("#appearance-panel");
  const inAppearance = await toggle.evaluate(node => !!node.closest("#appearance-panel"));
  const openedHere = inAppearance && await panel.getAttribute("aria-hidden") !== "false";
  let opener;
  if (openedHere) {
    opener = page.locator('[data-studio-action="appearance"]:visible, #appearance-btn:visible').first();
    await opener.click();
    await expect(panel).toHaveAttribute("aria-hidden", "false");
  }
  await toggle.click();
  if (openedHere) {
    await page.keyboard.press("Escape");
    await expect(panel).toHaveAttribute("aria-hidden", "true");
    await expect(opener).toBeFocused();
  }
}

export async function clickResetLayout(page) {
  const reset = page.locator("#reset-layout-btn");
  if (await reset.evaluate(node => !!node.closest("#studio-workspace-menu"))) {
    const menu = page.locator("#studio-workspace-menu");
    if (!await menu.evaluate(node => node.open)) await page.locator("#studio-workspace-menu-toggle").click();
    await expect(menu).toHaveAttribute("open", "");
  }
  await reset.click();
}
