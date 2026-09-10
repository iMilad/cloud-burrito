import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const STORE = "cloud-burrito.synthetic-collapse-dashboard";
const initialTiles = [
  { id: "collapse-stack-a", widget: "cfn-stacks", x: 0, y: 0, w: 6, h: 6,
    config: { header_color: "blue", header_style: "line", inputs: {} } },
  { id: "collapse-stack-b", widget: "cfn-stacks", x: 6, y: 0, w: 6, h: 5 },
  { id: "collapse-pipeline", widget: "pipeline-runs", x: 0, y: 6, w: 12, h: 6 },
];

function installCollapseControls({ store, tiles }) {
  const invoke = window.__TAURI__.core.invoke;
  if (localStorage.getItem(store) === null) localStorage.setItem(store, JSON.stringify(tiles));
  const fixture = window.__collapseFixture = { calls: [], saved: JSON.parse(localStorage.getItem(store)),
    holdFetch: false, pending: [] };
  window.__TAURI__.core.invoke = async (command, payload) => {
    fixture.calls.push({ command, params: structuredClone(payload?.params || {}) });
    if (command === "dashboard_get") return { tiles: structuredClone(fixture.saved), _storage: { status: "loaded", store: "dashboard" } };
    if (command === "dashboard_set") {
      fixture.saved = structuredClone(payload.params.tiles);
      localStorage.setItem(store, JSON.stringify(fixture.saved));
      return { tiles: fixture.saved, _storage: { status: "saved", store: "dashboard" } };
    }
    const result = await invoke(command, payload);
    if (command === "widget_fetch" && fixture.holdFetch) {
      await new Promise(resolve => fixture.pending.push(resolve));
    }
    return result;
  };
}

// Only synthetic IPC and synthetic local persistence; every non-local browser
// request is denied. No AWS configuration, credential, native app, or CLI runs.
async function boot(page, options = {}) {
  const remote = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remote.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  const bridge = { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 2, delayMs: 0 };
  const controls = { store: STORE, tiles: options.tiles || initialTiles };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(bridge)});(${installCollapseControls.toString()})(${JSON.stringify(controls)});` });
  await page.goto(options.design === "classic" ? "/?design=classic" : "/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(tile(page, "collapse-stack-a").locator(".cfn-stacks-body")).toContainText("synthetic-stack-00000");
  await expect(tile(page, "collapse-stack-a").locator(".collapse-btn")).toHaveCount(1);
  return remote;
}

const tile = (page, id) => page.locator(`.grid-stack-item[gs-id="${id}"]`);
const geometry = target => target.evaluate(item => ({
  h: item.gridstackNode.h, minH: item.gridstackNode.minH, maxH: item.gridstackNode.maxH,
  noResize: !!item.gridstackNode.noResize, config: JSON.parse(item.dataset.config || "{}"),
}));
const operations = page => page.evaluate(() => window.__collapseFixture.calls.filter(call =>
  ["aws_list_profiles", "aws_set_account", "widget_fetch"].includes(call.command)));
async function savedTile(page, id) {
  await page.clock.fastForward(500);
  return page.evaluate(id => window.__collapseFixture.saved.find(item => item.id === id), id);
}

for (const design of ["classic", "studio"]) {
  test(`${design}: collapse keeps only the header, shrinks the grid, and restores the previous height without reloading data`, async ({ page }) => {
    const remote = await boot(page, { design });
    const target = tile(page, "collapse-stack-a"), sibling = tile(page, "collapse-stack-b");
    const toggle = target.locator(".collapse-btn"), body = target.locator(".widget-body");
    const before = await geometry(target), calls = await operations(page);
    const receipt = await body.locator(".result-status time").first().getAttribute("datetime");
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(toggle).toHaveAttribute("aria-controls", await body.getAttribute("id"));
    await toggle.click();
    await expect(toggle).toHaveAccessibleName("Expand widget");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(target.locator(".widget-header")).toBeVisible();
    await expect(target.locator(".widget-context")).toBeVisible();
    await expect(body).toBeHidden();
    expect(await body.evaluate(node => node.inert)).toBe(true);
    expect((await geometry(target)).h).toBeLessThan(before.h);
    expect((await geometry(target)).noResize).toBe(true);
    await expect(sibling.locator(".widget-body")).toBeVisible();
    await expect(sibling.locator(".collapse-btn")).toHaveAttribute("aria-expanded", "true");
    const persisted = await savedTile(page, "collapse-stack-a");
    expect(persisted.config).toMatchObject({ collapsed: true, expanded_height: before.h,
      header_color: "blue", header_style: "line" });
    expect(persisted.h).toBe((await geometry(target)).h);
    await toggle.press("Enter");
    await expect(toggle).toHaveAccessibleName("Collapse widget");
    await expect(body).toBeVisible();
    expect(await body.evaluate(node => node.inert)).toBe(false);
    expect((await geometry(target)).h).toBe(before.h);
    expect((await geometry(target)).noResize).toBe(before.noResize);
    expect((await geometry(target)).minH).toBe(before.minH);
    expect(await body.locator(".result-status time").first().getAttribute("datetime")).toBe(receipt);
    expect(await operations(page)).toEqual(calls);
    expect(remote).toEqual([]);
  });
}

test("collapsed body controls leave keyboard navigation and late responses cannot reopen the body", async ({ page }) => {
  await boot(page);
  const target = tile(page, "collapse-pipeline");
  const input = target.locator(".pipeline-name-search"), toggle = target.locator(".collapse-btn");
  await expect(input).toBeEnabled();
  await input.focus();
  await toggle.click();
  await expect(toggle).toBeFocused();
  await expect(input).toBeHidden();
  await input.evaluate(node => node.focus());
  await expect(toggle).toBeFocused();
  await toggle.press("Tab");
  await expect(target.locator(".fs-btn")).toBeFocused();

  const stack = tile(page, "collapse-stack-a"), body = stack.locator(".widget-body");
  await page.evaluate(() => { window.__collapseFixture.holdFetch = true; });
  await stack.locator('button[title="Refresh"]').click();
  await expect.poll(() => page.evaluate(() => window.__collapseFixture.pending.length)).toBeGreaterThan(0);
  await stack.locator(".collapse-btn").click();
  await page.evaluate(() => {
    window.__collapseFixture.holdFetch = false;
    window.__collapseFixture.pending.splice(0).forEach(resolve => resolve());
  });
  await expect(body.locator(".result-status").first()).toHaveAttribute("data-state", "success");
  await expect(body).toBeHidden();
  expect(await body.evaluate(node => node.inert)).toBe(true);
  await stack.locator(".collapse-btn").click();
  await expect(body).toBeVisible();
  await expect(body).toContainText("synthetic-stack-00000");
});

test("per-tile collapse survives native dashboard reload and another tile of the same type stays independent", async ({ page }) => {
  await boot(page);
  const original = tile(page, "collapse-stack-a");
  const originalHeight = (await geometry(original)).h;
  await original.locator(".collapse-btn").click();
  await savedTile(page, "collapse-stack-a");
  await page.locator("#add-widget-btn").click();
  await page.locator(".prebuilt-item").filter({ has: page.locator(".prebuilt-name", { hasText: /^CloudFormation Stacks$/ }) })
    .locator(".prebuilt-add").click();
  const stackTiles = page.locator('.grid-stack-item:has(.widget[data-widget="cfn-stacks"])');
  await expect(stackTiles).toHaveCount(3);
  const added = stackTiles.filter({ has: page.locator('.widget:not(.is-collapsed)') }).last();
  const addedId = await added.getAttribute("gs-id");
  expect(addedId).not.toBe("collapse-stack-a");
  expect(addedId).not.toBe("collapse-stack-b");
  await expect(added.locator(".collapse-btn")).toHaveCount(1);
  await expect(added.locator(".collapse-btn")).toHaveAttribute("aria-expanded", "true");
  const addedHeight = (await geometry(added)).h;
  await added.locator(".collapse-btn").click();
  const addedSaved = await savedTile(page, addedId);
  expect(addedSaved.config).toMatchObject({ collapsed: true, expanded_height: addedHeight });
  const controls = await stackTiles.locator(".collapse-btn").evaluateAll(nodes => nodes.map(node => node.getAttribute("aria-controls")));
  expect(new Set(controls).size).toBe(3);
  await page.reload();
  await expect(original.locator(".collapse-btn")).toHaveAttribute("aria-expanded", "false");
  await expect(original.locator(".widget-body")).toBeHidden();
  await expect(tile(page, addedId).locator(".widget-body")).toBeHidden();
  await expect(tile(page, "collapse-stack-b").locator(".widget-body")).toBeVisible();
  await original.locator(".collapse-btn").click();
  expect((await geometry(original)).h).toBe(originalHeight);
  await tile(page, addedId).locator(".collapse-btn").click();
  expect((await geometry(tile(page, addedId))).h).toBe(addedHeight);

  // Resizing after expansion must restore normal GridStack constraints and
  // become the next remembered expanded height rather than an old snapshot.
  await original.evaluate(item => item.gridstackNode.grid.update(item, { h: 8 }));
  expect((await geometry(original)).h).toBe(8);
  await original.locator(".collapse-btn").click();
  expect((await savedTile(page, "collapse-stack-a")).config.expanded_height).toBe(8);
  await original.locator(".collapse-btn").click();
  expect((await geometry(original)).h).toBe(8);
});

test("fullscreen and collapse restore grid sizing and focus in both directions", async ({ page }) => {
  await boot(page);
  const target = tile(page, "collapse-stack-a"), widget = target.locator(".widget");
  const toggle = target.locator(".collapse-btn"), fullscreen = target.locator(".fs-btn");
  const height = (await geometry(target)).h;
  await fullscreen.click();
  await expect(widget).toHaveClass(/fullscreen/);
  await toggle.click();
  await expect(widget).not.toHaveClass(/fullscreen/);
  await expect(widget).toHaveClass(/is-collapsed/);
  await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
  await expect(toggle).toBeFocused();
  expect((await savedTile(page, "collapse-stack-a")).config.expanded_height).toBe(height);
  await fullscreen.click();
  await expect(widget).toHaveClass(/fullscreen/);
  await expect(widget).not.toHaveClass(/is-collapsed/);
  await expect(target.locator(".widget-body")).toBeVisible();
  await fullscreen.click();
  expect((await geometry(target)).h).toBe(height);
  expect((await geometry(target)).noResize).toBe(false);
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
});

test("collapsed grid rows grow to fit wrapped headers on a narrow screen", async ({ page }) => {
  await boot(page);
  const target = tile(page, "collapse-stack-a");
  await target.locator(".collapse-btn").click();
  const savedExpandedHeight = (await geometry(target)).config.expanded_height;
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => target.evaluate(item => {
    const header = item.querySelector(".widget-header").getBoundingClientRect();
    const tileBox = item.getBoundingClientRect();
    return header.bottom <= tileBox.bottom && header.right <= tileBox.right;
  })).toBe(true);
  await expect(target.locator(".collapse-btn")).toBeVisible();
  await expect(target.locator(".widget-body")).toBeHidden();
  expect((await geometry(target)).config.expanded_height).toBe(savedExpandedHeight);
  await target.locator(".collapse-btn").click();
  expect((await geometry(target)).h).toBe(savedExpandedHeight);
  await expect(target.locator(".widget-body")).toBeVisible();
});

test("saving and resetting widget appearance preserve then explicitly reset collapsed state", async ({ page }) => {
  await boot(page);
  const target = tile(page, "collapse-stack-a"), toggle = target.locator(".collapse-btn");
  const height = (await geometry(target)).h;
  await toggle.click();
  await target.locator(".cfg-btn").click();
  await page.getByRole("button", { name: "Header color: green", exact: true }).click();
  await page.locator("#cfg-save").click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(target.locator(".widget-body")).toBeHidden();
  expect((await savedTile(page, "collapse-stack-a")).config).toMatchObject({ collapsed: true, expanded_height: height, header_color: "green" });
  await target.locator(".cfg-btn").click();
  await page.locator("#cfg-reset").click();
  await page.locator("#cfg-save").click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(target.locator(".widget-body")).toBeVisible();
  expect((await geometry(target)).h).toBe(height);
});
