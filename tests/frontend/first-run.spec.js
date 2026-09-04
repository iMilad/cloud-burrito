import { expect, test } from "@playwright/test";

const identity = { profile: "demo-first-run", account_id: "acct-first-run-fixture", region: "eu-west-1" };
const stackTile = { id: "cfn-stacks", widget: "cfn-stacks", x: 0, y: 0, w: 6, h: 5 };
const cliTile = { id: "aws-cli", widget: "aws-cli", x: 6, y: 0, w: 6, h: 5,
  config: { context: { mode: "pinned", ...identity }, inputs: { command: "aws cloudformation list-stacks" } } };

// Production UI, synthetic IPC only. Discovery and availability never read a
// personal file, contact AWS, execute CLI, or permit a remote browser request.
async function boot(page, options = {}) {
  const remote = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remote.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  await page.addInitScript(({ identity, stackTile, cliTile, options }) => {
    const defaults = { aws_config_path: "/synthetic/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
    const settings = { ...defaults, default_profile: identity.profile, _storage: { status: "loaded" },
      _settings: { defaults, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} } };
    const supported = { name: identity.profile, account_id: identity.account_id, region: identity.region,
      role_name: "SyntheticReadOnly", sso_session: "synthetic-session", eligibility: "supported_sso", eligibility_reason: null };
    const fixture = window.__firstRun = { calls: [], discovery: options.discovery || "ready", profiles: [supported],
      partial: false, holdDiscovery: !!options.holdDiscovery, holdAvailability: false, holdSelection: false,
      cli: options.cli || "available", selectionError: options.selectionError || null, authFailure: null,
      rejectSelection: !!options.rejectSelection, rejectAuth: false };
    localStorage.setItem("acc.profiles.v1", JSON.stringify({ profiles: [supported], config_path: "/synthetic/obsolete-config" }));
    let active = null, contextSerial = 0;
    function attach(value, params, context = active) {
      return { ...value, _request: { id: params.request_id || null,
        context_id: context ? `synthetic-context-${contextSerial}` : null,
        provider_revision: context ? "synthetic-provider" : null, settings_revision: context ? "1" : null,
        profile: context?.profile || null, account_id: context?.account_id || null, region: context?.region || null,
        audit_id: "synthetic-audit", outcome: value.ok === false ? "failed" : "succeeded" } };
    }
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      fixture.calls.push({ command, params: structuredClone(params) });
      switch (command) {
        case "ping": return { version: "synthetic" };
        case "settings_get": return settings;
        case "settings_set": return { ...settings, ...params, _storage: { status: "saved" } };
        case "dashboard_get": return { tiles: options.cliWidget ? [stackTile, cliTile] : [stackTile], _storage: { status: "loaded" } };
        case "dashboard_set": return { ok: true };
        case "aws_list_profiles":
          if (fixture.holdDiscovery) await new Promise(resolve => { fixture.releaseDiscovery = resolve; });
          return { discovery_state: fixture.discovery, profiles: fixture.discovery === "ready" ? fixture.profiles : [],
            file_exists: fixture.discovery !== "missing_config", partial: fixture.partial,
            coverage: { complete: !fixture.partial, returned: fixture.profiles.length, limit: 500, omitted: fixture.partial ? 3 : 0 } };
        case "aws_set_account":
          active = null;
          if (fixture.rejectSelection) throw new Error("synthetic-private-diagnostic");
          if (fixture.holdSelection) await new Promise(resolve => { fixture.releaseSelection = resolve; });
          if (fixture.selectionError) return attach({ ok: false, error_type: fixture.selectionError,
            error: "Synthetic verification failure", needs_sso_login: fixture.selectionError === "CredentialsExpired" }, params, null);
          active = { profile: params.profile, account_id: params.account_id, region: params.region };
          contextSerial++;
          return attach({ ok: true }, params);
        case "aws_auth_status":
          if (fixture.rejectAuth) throw new Error("synthetic-private-diagnostic");
          if (fixture.authFailure) active = null;
          return attach({ has_context: !!active, logged_in: !!active, ...active,
            connection_state: active ? "verified" : fixture.authFailure ? "expired" : "unselected",
            error_type: fixture.authFailure, needs_sso_login: fixture.authFailure === "CredentialsExpired" }, params);
        case "aws_list_pipelines": return attach({ ok: true, pipelines: [] }, params);
        case "cli_availability":
          if (fixture.holdAvailability) await new Promise(resolve => { fixture.releaseAvailability = resolve; });
          return { ok: true, status: fixture.cli, available: fixture.cli === "available" ? true : fixture.cli === "missing" ? false : null, version_verified: false };
        case "widget_fetch": return attach({ render: "table", columns: ["evidence"], rows: [{ evidence: params.widget === "aws-cli" ? "synthetic CLI result" : "synthetic SDK result" }] }, params,
          params.context?.mode === "pinned" ? params.context : active);
        case "policy_get": return { raw: "Statement: []", valid: true, actions: [], path: "/synthetic/policy" };
        case "audit_tail": return { entries: [] };
        default: throw new Error("Unexpected synthetic boundary");
      }
    } } };
  }, { identity, stackTile, cliTile, options });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toBeVisible();
  return remote;
}
const calls = (page, command) => page.evaluate(command => window.__firstRun.calls.filter(call => call.command === command), command);
const cliFetches = page => page.evaluate(() => window.__firstRun.calls.filter(call => call.command === "widget_fetch" && call.params.widget === "aws-cli"));

for (const state of ["missing_config", "unreadable_config", "malformed_config", "no_profiles"]) {
  test(`${state} offers visible recovery, ignores old profile cache, and retries without restart`, async ({ page }) => {
    const remote = await boot(page, { discovery: state });
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", state);
    await expect(page.locator("#connection-settings")).toBeVisible();
    await expect(page.locator("#connection-retry")).toBeEnabled();
    expect(await calls(page, "aws_set_account")).toEqual([]);
    expect(await calls(page, "widget_fetch")).toEqual([]);
    await expect(page.locator("#account-picker-search")).toBeDisabled();
    await page.evaluate(() => { window.__firstRun.discovery = "ready"; });
    await page.locator("#connection-retry").click();
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
    await expect(page.locator("#connection-identity")).toHaveText(`Profile: ${identity.profile} · Verified account: ${identity.account_id} · Region: ${identity.region}`);
    await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("synthetic SDK result");
    expect((await calls(page, "aws_set_account")).length).toBe(1);
    expect(remote).toEqual([]);
  });
}

test("fresh discovery is required before cached verification, and a later malformed file clears old identity and evidence", async ({ page }) => {
  await boot(page, { holdDiscovery: true });
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "discovering");
  expect(await calls(page, "aws_set_account")).toEqual([]);
  await page.evaluate(() => { window.__firstRun.holdDiscovery = false; window.__firstRun.releaseDiscovery(); });
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("synthetic SDK result");
  await page.evaluate(() => { window.__firstRun.discovery = "malformed_config"; });
  await page.locator("#connection-retry").click();
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "malformed_config");
  await expect(page.locator('[data-widget="cfn-stacks"]')).not.toContainText("synthetic SDK result");
  await expect(page.locator("#auth-status")).not.toHaveAttribute("data-state", "online");
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "malformed_config");
  expect((await calls(page, "aws_set_account")).length).toBe(1);
});

test("unsupported credential providers are explained without verification, and limited discovery is visible", async ({ page }) => {
  await boot(page, { holdDiscovery: true });
  await page.evaluate(() => {
    const fixture = window.__firstRun;
    fixture.profiles[0].eligibility = "unsupported_credentials";
    fixture.profiles[0].eligibility_reason = "Synthetic fixed unsupported provider reason";
    fixture.partial = true;
    fixture.holdDiscovery = false;
    fixture.releaseDiscovery();
  });
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "unsupported");
  await expect(page.locator("#connection-profiles")).toContainText("Unsupported credential provider");
  await expect(page.locator("#connection-coverage")).toContainText("Other profiles were omitted");
  await expect(page.locator("#account-picker-search")).toBeDisabled();
  expect(await calls(page, "aws_set_account")).toEqual([]);
  await page.evaluate(() => { window.__firstRun.profiles[0].eligibility = "supported_sso"; window.__firstRun.partial = false; });
  await page.locator("#connection-retry").click();
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(page.locator("#connection-coverage")).toBeHidden();
});

for (const [error, state] of [["CredentialsExpired", "expired"], ["IdentityMismatch", "mismatch"], ["CredentialsError", "failed"]]) {
  test(`${error} stays unverified with a truthful retry path`, async ({ page }) => {
    await boot(page, { selectionError: error });
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", state);
    await expect(page.locator("#connection-identity")).toContainText("Account is not verified");
    expect(await calls(page, "widget_fetch")).toEqual([]);
    if (state === "expired") await expect(page.locator("#connection-message")).toContainText("outside this app");
    await page.evaluate(() => { window.__firstRun.selectionError = null; window.__firstRun.holdSelection = true; });
    await page.locator("#connection-retry").click();
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verifying");
    await page.evaluate(() => { window.__firstRun.releaseSelection(); });
    await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  });
}

test("an accepted expiry poll updates the recovery strip and removes inherited evidence", async ({ page }) => {
  await boot(page);
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("synthetic SDK result");
  await page.evaluate(() => { window.__firstRun.authFailure = "CredentialsExpired"; });
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "expired");
  await expect(page.locator('[data-widget="cfn-stacks"]')).not.toContainText("synthetic SDK result");
  await page.evaluate(() => { window.__firstRun.authFailure = null; });
  await page.locator("#connection-retry").click();
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
});

test("missing CLI disables only its widget and retries availability without executing a command", async ({ page }) => {
  const remote = await boot(page, { cliWidget: true, cli: "missing" });
  const cli = page.locator('[data-widget="aws-cli"]');
  await expect(cli.locator(".cli-availability")).toHaveAttribute("data-state", "missing");
  await expect(cli.locator(".cli-run-btn")).toBeDisabled();
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("synthetic SDK result");
  expect(await cliFetches(page)).toEqual([]);
  await page.evaluate(() => { window.__firstRun.cli = "available"; });
  await cli.getByRole("button", { name: "Retry CLI check" }).click();
  await expect(cli.locator(".cli-run-btn")).toBeEnabled();
  await expect(cli.locator(".cli-availability")).toContainText("version has not been checked");
  expect(await cliFetches(page)).toEqual([]);
  await cli.locator(".cli-run-btn").click();
  await expect(cli).toContainText("synthetic CLI result");
  expect(await cliFetches(page)).toHaveLength(1);
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("synthetic SDK result");
  expect(remote).toEqual([]);
});

test("an unavailable identity status has visible safe recovery and retains same-context evidence", async ({ page }) => {
  await boot(page);
  const stack = page.locator('[data-widget="cfn-stacks"]');
  await expect(stack).toContainText("synthetic SDK result");
  await page.evaluate(() => { window.__firstRun.rejectAuth = true; });
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "failed");
  await expect(page.locator("#auth-status")).toHaveAttribute("title", "Identity status could not be refreshed. Retry connection.");
  await expect(stack).toContainText("synthetic SDK result");
  await expect(page.locator("body")).not.toContainText("synthetic-private-diagnostic");
  await page.evaluate(() => { window.__firstRun.rejectAuth = false; });
  await page.locator("#connection-retry").click();
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
});

test("a rejected connection request offers retry without exposing the bridge diagnostic", async ({ page }) => {
  await boot(page, { rejectSelection: true });
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "failed");
  expect(await calls(page, "widget_fetch")).toEqual([]);
  await page.locator("#connection-details").click();
  await expect(page.locator("#identity-body")).toContainText("Account verification could not be completed. Retry connection.");
  await expect(page.locator("body")).not.toContainText("synthetic-private-diagnostic");
});

test("removing a CLI tile while its local availability check waits cannot start a stale command", async ({ page }) => {
  await boot(page, { cliWidget: true });
  const cli = page.locator('[data-widget="aws-cli"]');
  await expect(cli).toContainText("synthetic CLI result");
  const before = (await cliFetches(page)).length;
  await page.evaluate(() => { window.__firstRun.holdAvailability = true; });
  await cli.locator(".cli-run-btn").click();
  await expect(cli.locator(".cli-availability")).toHaveAttribute("data-state", "checking");
  await cli.locator(".rm-btn").click();
  await page.evaluate(() => { window.__firstRun.releaseAvailability(); });
  await expect(cli).toHaveCount(0);
  expect(await cliFetches(page)).toHaveLength(before);
});
