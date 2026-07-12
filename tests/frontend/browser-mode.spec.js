import { expect, test } from "@playwright/test";

const stackWidget = (page) => page.locator('.widget[data-widget="cfn-stacks"]');
const stackRows = (page) => stackWidget(page).locator(
  ".cfn-stacks-body > table.events-table > tbody > tr:not(.row-detail)"
);
const visibleStackRows = (page) => stackWidget(page).locator(
  ".cfn-stacks-body > table.events-table > tbody > tr:not(.row-detail):not([hidden])"
);

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".brand-tag")).toHaveText("browser mode");
});

test("starts in browser mode and renders mock widgets", async ({ page }) => {
  await expect(page).toHaveTitle("Cloud Burrito");
  await expect(page.locator(".brand-name")).toHaveText("Cloud Burrito");

  const coreStatus = page.locator("#core-status");
  await expect(coreStatus).toHaveAttribute("data-state", "offline");
  await expect(coreStatus.locator(".core-label")).toHaveText("browser mode");

  const account = page.getByRole("combobox", { name: "Default account" });
  await expect(account).toBeDisabled();
  await expect(account).toHaveValue("(browser mode — no profiles)");

  const region = page.getByRole("combobox", { name: "Default region" });
  await expect(region).toBeEnabled();
  await expect(region).toHaveValue("eu-west-1");

  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(6);
  await expect(page.getByRole("heading", { name: "CloudFormation Stacks" })).toBeVisible();
  await expect(stackRows(page)).toHaveCount(4);
  await expect(stackWidget(page).locator(".table-filter-count")).toHaveText("4");
});

test("supports the topbar shortcut and persistent region picker", async ({ page }) => {
  const globalSearch = page.getByRole("textbox", { name: "Global search" });
  await page.keyboard.press("Control+K");
  await expect(globalSearch).toBeFocused();

  const region = page.getByRole("combobox", { name: "Default region" });
  await region.click();

  const list = page.locator("#topbar-picker-list");
  await expect(list).toBeVisible();
  await expect(list.getByRole("option")).toHaveCount(2);
  await expect(region).toHaveAttribute("aria-expanded", "true");

  await region.fill("us east");
  await expect(list.getByRole("option")).toHaveCount(1);
  await expect(list.getByRole("option")).toHaveText("us-east-1");
  await expect(page.locator("#topbar-picker-status")).toHaveText("1 matching option");

  await region.press("Enter");
  await expect(region).toHaveValue("us-east-1");
  await expect(region).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("#region-select")).toHaveValue("us-east-1");
  await expect(list).toBeHidden();
  await expect(stackWidget(page).locator(".widget-context")).toHaveText(
    "Default · (none) · (none) · us-east-1"
  );

  await expect.poll(() => page.evaluate(() => {
    const raw = localStorage.getItem("acc.last.v1");
    return raw ? JSON.parse(raw).region : null;
  })).toBe("us-east-1");

  await page.reload();
  await expect(page.getByRole("combobox", { name: "Default region" })).toHaveValue("us-east-1");
  await expect(stackWidget(page).locator(".widget-context")).toHaveText(
    "Default · (none) · (none) · us-east-1"
  );
});

test("filters and expands CloudFormation mock stacks", async ({ page }) => {
  const widget = stackWidget(page);
  const rows = stackRows(page);
  const count = widget.locator(".table-filter-count");
  const table = widget.locator(".cfn-stacks-body > table.events-table");
  const filter = widget.getByRole("textbox", {
    name: "Search stacks (name, status, or date)…"
  });

  await expect(rows).toHaveCount(4);
  await rows.filter({ hasText: "uc-payment-service-prod" }).click();

  const detail = widget.locator(
    ".cfn-stacks-body > table.events-table > tbody > tr.row-detail"
  );
  await expect(detail).toBeVisible();
  await expect(detail).toContainText("Resources (4)");
  await expect(detail).toContainText("PaymentLambdaV2");

  await filter.fill("rollback");
  await expect(visibleStackRows(page)).toHaveCount(1);
  await expect(count).toHaveText("1 / 4");

  await filter.fill("^uc-(order|inventory)");
  await expect(visibleStackRows(page)).toHaveCount(2);
  await expect(detail).toBeHidden();
  await expect(count).toHaveText("2 / 4");

  await filter.fill("[");
  await expect(filter).toHaveClass(/table-filter-bad/);
  await expect(count).toHaveText("0 / 4");
  await expect(widget.locator(".table-filter-empty")).toHaveText("No matching stacks.");
  await expect(table).toBeHidden();

  await filter.clear();
  await expect(filter).not.toHaveClass(/table-filter-bad/);
  await expect(visibleStackRows(page)).toHaveCount(4);
  await expect(count).toHaveText("4");
  await expect(table).toBeVisible();
});

test("persists widget configuration across browser reloads", async ({ page }) => {
  const tile = page.locator('.grid-stack-item[gs-id="cfn-stacks"]');
  await tile.locator(".cfg-btn").click();

  const panel = page.locator("#widget-config-panel");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await expect(page.locator("#cfg-title")).toHaveText("Configure CloudFormation Stacks");

  await panel.locator('.color-swatch[data-color="blue"]').click();
  await panel.locator("#cfg-save").click();
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(tile.locator(".widget")).toHaveAttribute("data-header-color", "blue");

  await expect.poll(() => page.evaluate(() => {
    const raw = localStorage.getItem("acc.layout.v1");
    if (!raw) return null;
    const layout = JSON.parse(raw);
    return layout.find((entry) => entry.id === "cfn-stacks")?.config?.header_color ?? null;
  })).toBe("blue");

  await page.reload();
  await expect(page.locator('.grid-stack-item[gs-id="cfn-stacks"] .widget')).toHaveAttribute(
    "data-header-color",
    "blue"
  );
});
