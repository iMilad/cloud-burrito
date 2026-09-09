import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const CLOCK_START = "2026-09-09T09:00:00Z";
const SYNTHETIC_EXPIRY_KEY = "cloud-burrito.synthetic-sso-expiry";

// Only the synthetic IPC response supplies expiration. No AWS credentials,
// profile files, native processes or external requests are used by this test.
function installExpiryControls(options) {
  const invoke = window.__TAURI__.core.invoke;
  // This synthetic cache stores an expiry timestamp only. Keeping it across a
  // page reload proves that opening the UI cannot restart an existing clock.
  if (localStorage.getItem(options.expiryKey) === null) {
    localStorage.setItem(options.expiryKey, JSON.stringify(options.ssoTokenExpiresAt));
  }
  const fixture = window.__authExpiry = {
    expiresAt: null,
    ssoTokenExpiresAt: JSON.parse(localStorage.getItem(options.expiryKey)),
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
      return { ...response, expires_at: fixture.expiresAt, set_account_at: fixture.setAccountAt,
        ...(options.omitSsoExpiry ? {} : { sso_token_expires_at: fixture.ssoTokenExpiresAt }) };
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
  await page.clock.install({ time: new Date(CLOCK_START) });
  const bridge = { fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, kind: "stacks", rows: 1, delayMs: 0 };
  const controls = { durationMs: 12 * HOUR, expiryKey: SYNTHETIC_EXPIRY_KEY,
    ssoTokenExpiresAt: new Date(Date.parse(CLOCK_START) + 4 * HOUR).toISOString(), ...options };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(bridge)});(${installExpiryControls.toString()})(${JSON.stringify(controls)});` });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(page.locator("#auth-status")).toContainText("AWS verified");
  await expect(page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body')).toContainText("synthetic-stack-00000");
  return remoteRequests;
}

const state = page => page.evaluate(() => ({ ...window.__authExpiry, now: Date.now(),
  widgetFetches: window.__performanceFixture.commands.widget_fetch || 0 }));
const identityValue = (page, label) => page.locator("#identity-body dt")
  .filter({ hasText: new RegExp(`^${label}$`) }).locator("xpath=following-sibling::dd[1]");

test("the selected SSO token countdown survives role-credential renewal, Retry connection, and app reload", async ({ page }) => {
  const remoteRequests = await boot(page);
  const pill = page.locator("#auth-status");
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 4h left");
  const initial = await state(page);
  expect(Math.round((Date.parse(initial.expiresAt) - initial.now) / MINUTE)).toBe(12 * 60);

  await page.clock.fastForward(2 * HOUR);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 2h left");
  const afterPoll = await state(page);
  expect(afterPoll.expiresAt).toBe(initial.expiresAt);
  expect(afterPoll.ssoTokenExpiresAt).toBe(initial.ssoTokenExpiresAt);
  expect(Math.round((Date.parse(afterPoll.expiresAt) - afterPoll.now) / MINUTE)).toBe(10 * 60);
  expect(afterPoll.setAccountAt).toBe(initial.setAccountAt);
  expect(afterPoll.selections).toBe(initial.selections);
  expect(afterPoll.polls).toBeGreaterThan(initial.polls);

  await pill.click();
  await expect(identityValue(page, "SSO access token expires")).toHaveText(initial.ssoTokenExpiresAt);
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(initial.expiresAt);
  await expect(identityValue(page, "Last account verification")).not.toHaveText("(never)");
  await expect(page.locator("#identity-body")).toContainText("The countdown uses the expiry of your selected SSO access token.");
  await page.keyboard.press("Escape");

  const reconnect = page.locator("#connection-retry");
  await expect(reconnect).toHaveText("Retry connection");
  await expect(reconnect).toHaveAttribute("title", "Rediscover profiles and verify the selected account using your existing SSO session. This does not restart the token countdown; a supported token may renew when needed.");
  await reconnect.click();
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 2h left");
  const renewed = await state(page);
  expect(renewed.selections).toBe(initial.selections + 1);
  expect(renewed.setAccountAt).toBeGreaterThan(initial.setAccountAt);
  expect(renewed.expiresAt).not.toBe(initial.expiresAt);
  expect(Math.round((Date.parse(renewed.expiresAt) - renewed.now) / MINUTE)).toBe(12 * 60);
  expect(renewed.ssoTokenExpiresAt).toBe(initial.ssoTokenExpiresAt);

  await pill.click();
  await expect(identityValue(page, "SSO access token expires")).toHaveText(initial.ssoTokenExpiresAt);
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(renewed.expiresAt);
  await page.keyboard.press("Escape");
  await page.reload();
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 2h left");
  const reloaded = await state(page);
  expect(reloaded.ssoTokenExpiresAt).toBe(initial.ssoTokenExpiresAt);

  // Model actual token renewal: only a changed cache expiry may increase the
  // countdown. Freezing it or taking the minimum with an older expiry is wrong.
  const updatedExpiry = await page.evaluate(({ key, lifetime }) => {
    const expiry = new Date(Date.now() + lifetime).toISOString();
    window.__authExpiry.ssoTokenExpiresAt = expiry;
    localStorage.setItem(key, JSON.stringify(expiry));
    return expiry;
  }, { key: SYNTHETIC_EXPIRY_KEY, lifetime: 3 * HOUR + 35 * MINUTE });
  await pill.click();
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 3h 35m left");
  await expect(identityValue(page, "SSO access token expires")).toHaveText(updatedExpiry);
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(reloaded.expiresAt);
  const updated = await state(page);
  expect(updated.selections).toBe(reloaded.selections);
  expect(updated.expiresAt).toBe(reloaded.expiresAt);
  expect(updated.polls).toBeGreaterThan(reloaded.polls);
  expect(remoteRequests).toEqual([]);
});

test("unknown role-credential expiration does not hide a known SSO token countdown", async ({ page }) => {
  const remoteRequests = await boot(page, { durationMs: null });
  const pill = page.locator("#auth-status");
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 4h left");
  await page.clock.fastForward(2 * HOUR);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: 2h left");
  await pill.click();
  await expect(identityValue(page, "SSO access token expires")).toHaveText((await state(page)).ssoTokenExpiresAt);
  await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText("(unknown)");
  expect(remoteRequests).toEqual([]);
});

for (const [name, options] of [
  ["missing", { omitSsoExpiry: true }],
  ["unknown", { ssoTokenExpiresAt: null }],
  ["invalid", { ssoTokenExpiresAt: "not-a-synthetic-timestamp" }],
]) {
  test(`${name} SSO token expiration never falls back to valid role-credential expiry`, async ({ page }) => {
    const remoteRequests = await boot(page, options);
    const pill = page.locator("#auth-status");
    await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: expiry unknown");
    const initial = await state(page);
    expect(Math.round((Date.parse(initial.expiresAt) - initial.now) / MINUTE)).toBe(12 * 60);
    await page.clock.fastForward(HOUR);
    await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: expiry unknown");
    expect((await state(page)).selections).toBe(initial.selections);
    await pill.click();
    await expect(identityValue(page, "Temporary AWS credentials expire")).toHaveText(initial.expiresAt);
    expect(remoteRequests).toEqual([]);
  });
}

test("an expired SSO token does not discard evidence or reconnect while verified AWS credentials remain valid", async ({ page }) => {
  const remoteRequests = await boot(page, { ssoTokenExpiresAt: new Date(Date.parse(CLOCK_START) - MINUTE).toISOString() });
  const pill = page.locator("#auth-status");
  const evidence = page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body');
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: expired");
  await expect(pill).toHaveAttribute("data-state", "online");
  const initial = await state(page);
  expect(Date.parse(initial.expiresAt)).toBeGreaterThan(initial.now);
  await page.clock.fastForward(2 * MINUTE);
  await expect(pill).toHaveText("AWS verified · acct-a-fixture · SSO token: expired");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(evidence).toContainText("synthetic-stack-00000");
  const after = await state(page);
  expect(after.selections).toBe(initial.selections);
  expect(after.widgetFetches).toBe(initial.widgetFetches);
  expect(after.polls).toBeGreaterThan(initial.polls);
  expect(remoteRequests).toEqual([]);
});
