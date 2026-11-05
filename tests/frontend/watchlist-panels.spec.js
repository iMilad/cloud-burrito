import { expect, test } from "@playwright/test";
import { A, bootWatchlist, calls, rowText, thing } from "./watchlist-harness.mjs";

const pipe = thing("p-1", "pipeline", A, { pipeline_name: "synthetic-ok" });

test(", opens settings and Escape closes it without leaking keys to the list", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await page.keyboard.press(",");
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("j");
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "false");
  await page.keyboard.press("Escape");
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "true");
});

test(":audit opens the audit log and :policy shows the policy editor", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await page.keyboard.press(":");
  await page.getByRole("combobox", { name: "Command" }).fill("audit");
  await page.keyboard.press("Enter");
  await expect(page.locator("#audit-table-wrap")).toContainText("widget_fetch");
  await page.keyboard.press("Escape");
  await page.keyboard.press(":");
  await page.getByRole("combobox", { name: "Command" }).fill("policy");
  await page.keyboard.press("Enter");
  await expect(page.locator("#policy-editor")).toBeFocused();
});

test("saving a new AWS config path reloads every thing", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await expect(rowText(page, "p-1")).toHaveText("Succeeded");
  const before = (await calls(page, "widget_fetch")).length;
  await page.keyboard.press(",");
  await page.locator("#settings-aws-config-path").fill("/synthetic/other-config");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect.poll(async () => (await calls(page, "widget_fetch")).length).toBeGreaterThan(before);
  expect((await calls(page, "aws_list_profiles")).length).toBe(2);
});

test("a credential change asks for the selected thing's SSO status again", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await expect(page.locator("#wl-sso")).toContainText("SSO for demo-a");
  const before = (await calls(page, "aws_auth_status")).length;
  await page.keyboard.press(",");
  await page.locator("#settings-aws-config-path").fill("/synthetic/other-config");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect.poll(async () => (await calls(page, "aws_auth_status")).length).toBeGreaterThan(before);
});

test("r on a saved thing asks for its SSO status again", async ({ page }) => {
  await bootWatchlist(page, { things: [pipe] });
  await expect(page.locator("#wl-sso")).toContainText("SSO for demo-a");
  const before = (await calls(page, "aws_auth_status")).length;
  await page.keyboard.press("r");
  await expect.poll(async () => (await calls(page, "aws_auth_status")).length).toBeGreaterThan(before);
});

test("the browser demo opens no panel and says why", async ({ page }) => {
  await page.context().route("**/*", route => new URL(route.request().url()).origin === "http://127.0.0.1:4173"
    ? route.continue() : route.abort("blockedbyclient"));
  await page.goto("/watchlist.html");
  await page.waitForFunction(() => document.body.dataset.ready === "true");
  const notice = "Settings, the policy editor and the audit log open only in the desktop app.";
  await page.keyboard.press(",");
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator("#wl-notice")).toHaveText(notice);
  await page.keyboard.press(":");
  await page.getByRole("combobox", { name: "Command" }).fill("audit");
  await page.keyboard.press("Enter");
  await expect(page.locator("#audit-panel")).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator("#wl-notice")).toHaveText(notice);
  await expect(page.locator("#wl-keys")).not.toContainText("Settings");
  await page.keyboard.press("?");
  await expect(page.locator("#wl-keys")).toContainText("All keys");
  await expect(page.locator("#wl-keys")).not.toContainText("Settings");
});
