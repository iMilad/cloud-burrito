import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

// Only the synthetic IPC response supplies expiration. No AWS credentials,
// profile files, native processes or external requests are used by this test.
function installExpiryControls(options) {
  const invoke = window.__TAURI__.core.invoke;
  const fixture = window.__authExpiry = {
    expiresAt: null,
    setAccountAt: null,
    durationMs: options.durationMs,
    selections: 0,
    polls: 0,
  };
  window.__TAURI__.core.invoke = async (command, payload) => {
    const response = await invoke(command, payload);
    if (command === "aws_set_account" && response.ok) {
      fixture.selections++;
      fixture.setAccountAt = Math.floor(Date.now() / 1000);
      fixture.expiresAt = fixture.durationMs === null
        ? null : new Date(Date.now() + fixture.durationMs).toISOString();
    }
    if (command === "aws_auth_status") {
      fixture.polls++;
      return { ...response, expires_at: fixture.expiresAt, set_account_at: fixture.setAccountAt };
    }
    return response;
  };
}

async function boot(page, options = {}) {
  const remoteRequests = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remoteRequests.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install({ time: new Date("2026-09-09T09:00:00Z") });
  const bridge = { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 1, delayMs: 0 };
  const controls = { durationMs: 12 * HOUR, ...options };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(bridge)});(${installExpiryControls.toString()})(${JSON.stringify(controls)});` });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(page.locator("#auth-status")).toContainText("AWS verified");
  return remoteRequests;
}

const state = page => page.evaluate(() => ({ ...window.__authExpiry }));
const identityValue = (page, label) => page.locator("#identity-body dt")
  .filter({ hasText: new RegExp(`^${label}$`) }).locator("xpath=following-sibling::dd[1]");

test("credential countdown uses returned expiry and reconnect can renew for a different duration", async ({ page }) => {
  const remoteRequests = await boot(page);
  const pill = page.locator("#auth-status");
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: 12h left");
  const initial = await state(page);

  await page.clock.fastForward(2 * HOUR);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: 10h left");
  const afterPoll = await state(page);
  expect(afterPoll.expiresAt).toBe(initial.expiresAt);
  expect(afterPoll.setAccountAt).toBe(initial.setAccountAt);
  expect(afterPoll.selections).toBe(initial.selections);
  expect(afterPoll.polls).toBeGreaterThan(initial.polls);

  await pill.click();
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(initial.expiresAt);
  await expect(identityValue(page, "Last account verification")).not.toHaveText("(never)");
  await expect(page.locator("#identity-body")).toContainText("The original SSO login time is not tracked by this app.");
  await page.keyboard.press("Escape");

  const reconnect = page.locator("#connection-retry");
  await expect(reconnect).toHaveText("Reconnect");
  await expect(reconnect).toHaveAttribute("title", /existing SSO session/i);
  await page.evaluate(duration => { window.__authExpiry.durationMs = duration; }, 37 * MINUTE);
  await reconnect.click();
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: 37m left");
  const renewed = await state(page);
  expect(renewed.selections).toBe(initial.selections + 1);
  expect(renewed.setAccountAt).toBeGreaterThan(initial.setAccountAt);
  expect(renewed.expiresAt).not.toBe(initial.expiresAt);

  await page.clock.fastForward(5 * MINUTE);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: 32m left");
  await pill.click();
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(renewed.expiresAt);
  expect(remoteRequests).toEqual([]);
});

test("a verified connection with no returned expiry never invents a remaining lifetime", async ({ page }) => {
  const remoteRequests = await boot(page, { durationMs: null });
  const pill = page.locator("#auth-status");
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: expiry unknown");
  await page.clock.fastForward(2 * HOUR);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · credentials: expiry unknown");
  await pill.click();
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText("(unknown)");
  await expect(page.locator("#identity-body")).toContainText("remaining lifetime of temporary AWS credentials");
  expect(remoteRequests).toEqual([]);
});
