import { expect, test } from "@playwright/test";
import { installSyntheticBridge, FIXTURE_REVISION, FIXTURE_SEED } from "../performance/browser-fixture.mjs";

test.beforeEach(async ({ page }) => {
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    return route.abort("blockedbyclient");
  });
});

const pipeline = page => page.locator('[data-widget="pipeline-runs"]').first();
async function configure(page, widget = pipeline(page)) {
  await widget.locator(".cfg-btn").click();
  await expect(page.locator("#widget-config-panel")).toHaveAttribute("aria-hidden", "false");
}

for (const design of ["studio", "classic"]) {
  test(`${design}: preview cancels cleanly and a saved red gradient survives restart`, async ({ page }) => {
    await page.goto(`/?design=${design}`);
    await configure(page);
    await page.getByRole("button", { name: "Header color: red", exact: true }).click();
    await page.getByRole("button", { name: "Soft gradient", exact: true }).click();
    await expect(page.locator("#cfg-header-preview")).toHaveAttribute("data-header-color", "red");
    await expect(page.locator("#cfg-header-preview")).toHaveAttribute("data-header-style", "gradient");
    await expect(pipeline(page)).not.toHaveAttribute("data-header-color", "red");
    await page.locator("#cfg-cancel").click();
    await configure(page);
    await expect(page.getByRole("button", { name: "Header color: neutral", exact: true })).toHaveAttribute("aria-pressed", "true");
    await page.getByRole("button", { name: "Header color: red", exact: true }).click();
    await page.getByRole("button", { name: "Soft gradient", exact: true }).click();
    await page.locator("#cfg-save").click();
    await expect(pipeline(page)).toHaveAttribute("data-header-style", "gradient");
    expect(await pipeline(page).locator(".widget-header").evaluate(node => getComputedStyle(node).backgroundImage)).toContain("linear-gradient");
    await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("acc.layout.v1") || "[]")
      .find(tile => tile.id === "pipeline-runs")?.config?.header_style)).toBe("gradient");
    await page.reload();
    await expect(pipeline(page)).toHaveAttribute("data-header-color", "red");
    await expect(pipeline(page)).toHaveAttribute("data-header-style", "gradient");
    await configure(page);
    await page.locator("#cfg-reset").click();
    await page.locator("#cfg-save").click();
    await expect(pipeline(page)).not.toHaveAttribute("data-header-color", "red");
    await expect(pipeline(page)).toHaveAttribute("data-header-style", "tint");
  });
}

test("header edits preserve live results and unsaved input without issuing data requests", async ({ page }) => {
  await page.addInitScript(installSyntheticBridge, { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED,
    kind: "six", rows: 5, delayMs: 0 });
  await page.goto("/");
  const widget = page.locator('[data-widget="aws-cli"]');
  await expect(widget.locator(".aws-cli-rows")).toContainText("synthetic");
  const counts = await page.evaluate(() => ({ ...window.__performanceFixture.commands }));
  const body = await widget.locator(".widget-body").elementHandle();
  await configure(page, widget);
  await page.getByRole("button", { name: "Header color: purple", exact: true }).click();
  await page.getByRole("button", { name: "Accent line", exact: true }).click();
  await page.locator("#cfg-save").click();
  await expect(widget.locator(".aws-cli-rows")).toContainText("synthetic");
  expect(await body.evaluate(node => node.isConnected)).toBe(true);
  const afterFirst = await page.evaluate(() => window.__performanceFixture.commands);
  const dataCommands = ["widget_fetch", "aws_list_pipelines", "aws_set_account", "request_cancel"];
  for (const command of dataCommands) expect(afterFirst[command] || 0).toBe(counts[command] || 0);
  // Editing a command intentionally clears its old result. A later cosmetic
  // save must preserve this unfinished draft without executing it.
  await widget.getByRole("textbox", { name: "Command", exact: true }).fill("aws ec2 describe-instances");
  await expect(widget.locator(".aws-cli-rows")).toBeHidden();
  const afterEdit = await page.evaluate(() => ({ ...window.__performanceFixture.commands }));
  await configure(page, widget);
  await page.getByRole("button", { name: "Soft gradient", exact: true }).click();
  await page.locator("#cfg-save").click();
  await expect(widget.getByRole("textbox", { name: "Command", exact: true })).toHaveValue("aws ec2 describe-instances");
  await expect(widget.locator(".aws-cli-rows")).toBeHidden();
  await expect.poll(() => page.evaluate(() => window.__performanceFixture.commands.dashboard_set || 0)).toBeGreaterThan(counts.dashboard_set || 0);
  const after = await page.evaluate(() => window.__performanceFixture.commands);
  for (const command of dataCommands) {
    expect(after[command] || 0).toBe(afterEdit[command] || 0);
  }
});
