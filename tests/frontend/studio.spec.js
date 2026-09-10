import { expect, test } from "@playwright/test";

// The production presentation layer runs against browser-only synthetic data.
// Every non-local request is blocked; no native bridge or provider is installed.
const failures = new WeakMap();
const items = page => page.locator("#grid-stack > .grid-stack-item");
const widget = (page, name) => page.locator(`.widget[data-widget="${name}"]`);
const commands = page => page.locator("#command-results button");
const storage = page => page.evaluate(() => Object.fromEntries(Object.entries(localStorage)));

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
  await expect(page.locator(".brand-tag")).toHaveText("browser mode");
  await expect(page.locator("#studio-mode-label")).toHaveText("Demo active");
}

test("defaults to Studio with honest counts and a reversible, allowlisted URL preview", async ({ page }) => {
  await boot(page);
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  await expect(page.locator("#studio-tool-count")).toHaveText("6");
  await expect(page.locator("#studio-summary")).toHaveText("6 open tools");
  await expect(page.locator("#command-panel")).toHaveAttribute("inert", "");
  await expect(page.locator("#studio-return")).toBeHidden();
  await expect(page.locator("#auth-status")).toBeHidden();
  await expect(page.locator("#settings-btn")).toBeHidden();

  await page.locator("#studio-classic-switch").click();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await page.goto("/?design=studio");
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  expect(await page.evaluate(() => localStorage.getItem("cb.presentation.v1"))).toBe("classic");
  await page.goto("/?design=unrecognized");
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await expect(page.locator("#studio-return")).toBeVisible();
  await page.keyboard.press("Control+K");
  await expect(page.locator("#command-panel")).toHaveAttribute("aria-hidden", "true");
});

test("command search contains focus, handles empty results, and returns focus before opening existing panels", async ({ page }) => {
  await boot(page);
  const opener = page.locator("#studio-launcher");
  const panel = page.locator("#command-panel");
  const search = page.locator("#command-search");
  await opener.focus();
  await page.keyboard.press("Control+K");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await expect(panel).toHaveAttribute("role", "dialog");
  await expect(panel).toHaveAttribute("aria-modal", "true");
  await expect(search).toBeFocused();
  await expect(page.locator(".dashboard")).toHaveAttribute("inert", "");
  await expect(page.locator(".topbar")).toHaveAttribute("inert", "");
  await commands(page).last().focus();
  await page.keyboard.press("Tab");
  await expect(page.locator("#command-close")).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(commands(page).last()).toBeFocused();
  await search.fill("synthetic-command-that-does-not-exist");
  await expect(commands(page)).toHaveCount(0);
  await expect(page.locator("#command-empty")).toBeVisible();
  await search.press("Enter");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("Escape");
  await expect(panel).toHaveAttribute("inert", "");
  await expect(commands(page)).toHaveCount(0);
  await expect(opener).toBeFocused();
  await expect(page.locator(".dashboard")).not.toHaveAttribute("inert", "");

  await page.keyboard.press("Meta+K");
  await search.fill("settings");
  await expect(commands(page)).toHaveCount(0);
  await search.fill("widget library");
  await expect(commands(page)).toHaveCount(1);
  await search.press("ArrowDown");
  await expect(commands(page).first()).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator("#side-panel")).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("Escape");
  await expect(page.locator("#add-widget-btn")).toBeFocused();
});

test("navigation focuses an existing tool and removal disables its shortcut without adding a replacement", async ({ page }) => {
  await boot(page);
  const navigation = page.locator('#studio-nav [data-studio-jump="cfn-stacks"]');
  const target = widget(page, "cfn-stacks");
  await navigation.click();
  await expect(target).toBeFocused();
  await expect(navigation).toHaveAttribute("aria-current", "location");
  await expect(items(page)).toHaveCount(6);
  await target.locator(".rm-btn").click();
  await expect(items(page)).toHaveCount(5);
  await expect(navigation).toBeDisabled();
  await expect(page.locator("#studio-tool-count")).toHaveText("5");
  await expect(page.locator("#studio-summary")).toHaveText("5 open tools");
  await page.locator("#studio-launcher").click();
  await page.locator("#command-search").fill("CloudFormation Stacks");
  await expect(commands(page)).toHaveCount(0);
  await expect(page.locator("#command-empty")).toBeVisible();
  await expect(items(page)).toHaveCount(5);
});

test("duplicate tools stay distinct in the launcher and keyboard navigation selects the intended instance", async ({ page }) => {
  await boot(page);
  await page.locator("#add-widget-btn").click();
  const catalog = page.locator("#side-panel .prebuilt-item").filter({
    has: page.locator(".prebuilt-name", { hasText: /^CloudFormation Stacks$/ }),
  });
  await catalog.getByRole("button", { name: "Add", exact: true }).click();
  await expect(items(page)).toHaveCount(7);
  await expect(widget(page, "cfn-stacks")).toHaveCount(2);
  await expect(page.locator("#studio-tool-count")).toHaveText("7");
  const instanceIds = await page.locator('#grid-stack > .grid-stack-item:has(.widget[data-widget="cfn-stacks"])')
    .evaluateAll(nodes => nodes.map(node => node.getAttribute("gs-id")));
  expect(new Set(instanceIds).size).toBe(2);
  await page.locator("#studio-launcher").click();
  await page.locator("#command-search").fill("CloudFormation Stacks");
  await expect(commands(page)).toHaveCount(2);
  await expect(commands(page).nth(0)).toHaveAttribute("data-command-key", `tile:${instanceIds[0]}`);
  await expect(commands(page).nth(1)).toHaveAttribute("data-command-key", `tile:${instanceIds[1]}`);
  await page.locator("#command-search").press("ArrowUp");
  await expect(commands(page).nth(1)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(widget(page, "cfn-stacks").nth(1)).toBeFocused();
  await expect(page.locator("#command-panel")).toHaveAttribute("inert", "");
  await expect(items(page)).toHaveCount(7);
});

test("switching and reloading both designs preserves the saved layout, widget configuration and account selection", async ({ page }) => {
  const savedLayout = JSON.stringify([
    { id: "cfn-stacks", widget: "cfn-stacks", x: 0, y: 0, w: 7, h: 5, config: { header_color: "blue" } },
    { id: "resource-lookup", widget: "resource-lookup", x: 7, y: 0, w: 5, h: 5 },
  ]);
  const savedSelection = JSON.stringify({ profile: "", region: "us-east-1" });
  await page.addInitScript(({ savedLayout, savedSelection }) => {
    if (sessionStorage.getItem("studio-fixture-seeded")) return;
    localStorage.setItem("acc.layout.v1", savedLayout);
    localStorage.setItem("acc.last.v1", savedSelection);
    sessionStorage.setItem("studio-fixture-seeded", "true");
  }, { savedLayout, savedSelection });
  await boot(page);
  await expect(items(page)).toHaveCount(2);
  await expect(widget(page, "cfn-stacks")).toHaveAttribute("data-header-color", "blue");
  await page.locator("#theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  const before = await storage(page);
  const geometry = () => items(page).evaluateAll(nodes => nodes.map(node => ({
    id: node.getAttribute("gs-id"),
    position: ["x", "y", "w", "h"].map(field => node.gridstackNode[field]),
  })));
  const positions = await geometry();

  for (const [button, design] of [["#studio-classic-switch", "classic"], ["#studio-return", "studio"]]) {
    await page.locator(button).click();
    await expect(page.locator("html")).toHaveAttribute("data-design", design);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    // A width-triggered GridStack change would otherwise write after its debounce.
    await page.waitForTimeout(700);
    expect(await geometry()).toEqual(positions);
    expect(await storage(page)).toEqual({ ...before, "cb.presentation.v1": design });
  }

  await page.locator("#studio-classic-switch").click();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-design", "classic");
  await expect(items(page)).toHaveCount(2);
  expect(await geometry()).toEqual(positions);
  await page.locator("#studio-return").click();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
  await expect(items(page)).toHaveCount(2);
  await expect(widget(page, "cfn-stacks")).toHaveAttribute("data-header-color", "blue");
  await expect(page.locator("#region-picker-search")).toHaveValue("us-east-1");
  expect(await geometry()).toEqual(positions);
  expect(await storage(page)).toEqual({ ...before, "cb.presentation.v1": "studio" });
});

test("Classic has a reachable topbar return from fullscreen at desktop and narrow widths", async ({ page }) => {
  for (const width of [1480, 1024, 500]) {
    await page.setViewportSize({ width, height: 900 });
    await boot(page, "/?design=classic&appearance=precision");
    const target = widget(page, "pipeline-runs");
    await target.locator(".fs-btn").click();
    await expect(target).toHaveClass(/fullscreen/);
    await expect(page.locator("body")).toHaveClass(/has-fullscreen-widget/);

    const returnButton = page.getByRole("button", { name: "Studio view", exact: true });
    await expect(page.locator(".topbar-right #studio-return")).toBeVisible();
    await expect(returnButton).toBeInViewport();
    // A normal click verifies that the fullscreen widget cannot cover the control.
    await returnButton.click();
    await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
    await expect(target).not.toHaveClass(/fullscreen/);
    await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
    await expect(page.locator("#studio-classic-switch")).toBeFocused();
    expect(new URL(page.url()).searchParams.has("design")).toBe(false);
    expect(new URL(page.url()).searchParams.get("appearance")).toBe("precision");
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-design", "studio");
    await expect(page.locator("html")).toHaveAttribute("data-appearance", "precision");
  }
});

test("compact mode and reduced-motion navigation remain usable at a narrow desktop width", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.addInitScript(() => {
    const original = Element.prototype.scrollIntoView;
    window.__studioScrollCalls = [];
    Element.prototype.scrollIntoView = function (options) {
      window.__studioScrollCalls.push({ widget: this.dataset.widget, behavior: options?.behavior });
      return original.call(this, options);
    };
  });
  await boot(page);
  await expect(page.locator("#studio-density")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await page.locator("#studio-launcher").click();
  await page.locator("#command-search").fill("Lambda Logs");
  await expect(commands(page)).toHaveCount(1);
  await page.locator("#command-search").press("Enter");
  await expect(widget(page, "log-tail")).toBeFocused();
  expect(await page.evaluate(() => window.__studioScrollCalls.filter(call => call.widget === "log-tail"))).toEqual([
    { widget: "log-tail", behavior: "auto" },
  ]);
  await expect(items(page)).toHaveCount(6);
  await page.locator("#studio-density").click();
  await expect(page.locator("html")).toHaveAttribute("data-density", "comfortable");
  await expect(page.locator("#studio-density")).toHaveAttribute("aria-pressed", "false");
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
});
