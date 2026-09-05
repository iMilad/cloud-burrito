import { expect, test } from "@playwright/test";

const defaults = { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
const profiles = ["a", "b"].map((suffix, index) => ({ name: `demo-${suffix}`, account_id: String(index + 1).repeat(12),
  region: "eu-west-1", role_name: "SyntheticReadOnly", sso_session: "synthetic-session",
  eligibility: "supported_sso", eligibility_reason: "Synthetic supported SSO profile" }));
const tile = { id: "cfn-stacks", widget: "cfn-stacks", x: 0, y: 0, w: 6, h: 4 };

// Runs production HTML/JS with a synthetic persistence + context bridge. Every
// browser request outside the local test server is denied. No native calls.
async function bootSettings(page, options = {}) {
  const remote = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remote.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  await page.addInitScript(({ defaults, profiles, tile, options }) => {
    const key = "cloud-burrito-settings-synthetic";
    const persisted = JSON.parse(sessionStorage.getItem(key) || "null") || {
      settings: { ...defaults, ...options.settings }, tiles: options.tiles || [tile],
    };
    const persist = () => sessionStorage.setItem(key, JSON.stringify(persisted));
    const backendDefaults = { ...defaults, ...options.defaults };
    const regions = options.regions || ["eu-west-1", "us-east-1"];
    const fixture = window.__settingsFixture = {
      calls: [], persisted, persist, holdReads: !!options.holdReads, holdSaves: false, holdLayout: false,
      failSave: false, failLayout: false,
      holdVerification: false,
    };
    const fieldErrors = value => {
      const errors = {};
      if (!regions.includes(value.default_region)) errors.default_region = "Choose a supported default region";
      if (!["dark", "light"].includes(value.theme)) errors.theme = "Choose light or dark theme";
      return errors;
    };
    const metadata = errors => ({ defaults: backendDefaults, allowed_regions: regions, field_errors: errors });
    const response = (value, status) => ({ ...value, _storage: { status, store: "settings" },
      ...(options.omitMetadata ? {} : { _settings: metadata(fieldErrors(value)) }) });
    if (options.cache) localStorage.setItem("acc.profiles.v1", JSON.stringify({ profiles, config_path: "/synthetic/config" }));
    let active = null, revision = 1;
    const attach = (value, params, context = active) => ({ ...value, _request: {
      id: params.request_id || null, context_id: context ? `synthetic-context-${revision}` : null,
      provider_revision: context ? `synthetic-provider-${revision}` : null, settings_revision: context ? String(revision) : null,
      profile: context?.profile || null, account_id: context?.account_id || null, region: context?.region || null,
      outcome: value.ok === false ? "failed" : "succeeded", audit_id: "synthetic-request",
    } });
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      fixture.calls.push({ command, params: structuredClone(params) });
      switch (command) {
        case "request_cancel": if (typeof params.request_id !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(params.request_id)) throw new Error("Invalid synthetic cancellation ID"); return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
        case "ping": return { version: "synthetic" };
        case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
        case "settings_get":
          if (fixture.holdReads) await new Promise(resolve => { fixture.releaseRead = resolve; });
          return response(persisted.settings, "loaded");
        case "settings_set": {
          if (fixture.holdSaves) await new Promise(resolve => { fixture.releaseSave = resolve; });
          const next = Object.fromEntries(Object.entries(backendDefaults).map(([key, fallback]) => [key, params[key]?.trim() || fallback]));
          const errors = fieldErrors(next);
          if (fixture.failSave || Object.keys(errors).length) return { ok: false,
            error_type: fixture.failSave ? "StorageWriteFailed" : "InvalidRequest", error: "Synthetic rejection",
            _settings: metadata(errors), _storage: { status: "failed", store: "settings" } };
          if (["aws_config_path", "sso_session_name"].some(key => next[key] !== persisted.settings[key])) { revision++; active = null; }
          persisted.settings = next;
          persist();
          return response(next, "saved");
        }
        case "dashboard_get": return { version: 1, tiles: persisted.tiles, _storage: { status: "loaded", store: "dashboard" } };
        case "dashboard_set":
          if (fixture.holdLayout) await new Promise(resolve => { fixture.releaseLayout = resolve; });
          if (fixture.failLayout) return { ok: false, error_type: "StorageWriteFailed", error: "Synthetic rejection" };
          persisted.tiles = params.tiles; persist();
          return { version: 1, tiles: params.tiles, _storage: { status: "saved", store: "dashboard" } };
        case "aws_list_profiles": return { profiles, file_exists: true, config_path: "/synthetic/config", error: null, discovery_state: "ready" };
        case "aws_set_account":
          if (fixture.holdVerification) await new Promise(resolve => { fixture.releaseVerification = resolve; });
          active = { profile: params.profile, account_id: params.account_id, region: params.region };
          return attach({ ok: true }, params);
        case "aws_auth_status": return attach({ has_context: !!active, logged_in: !!active, connection_state: active ? "verified" : "unselected", ...active }, params);
        case "aws_list_pipelines": return attach({ ok: true, pipelines: [] }, params);
        case "widget_fetch": return attach({ render: "table", columns: ["stack"], rows: [{ stack: `evidence-${(params.context?.mode === "pinned" ? params.context : active)?.profile}` }] }, params,
          params.context?.mode === "pinned" ? params.context : active);
        case "policy_get": return { raw: "Statement: []", valid: true, actions: [], path: "/synthetic/policy.yaml" };
        case "audit_history": if (params.action !== "status") throw new Error("Unexpected audit history action"); return { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." };
        case "audit_tail": return { entries: [] };
        default: throw new Error("Unexpected synthetic boundary");
      }
    } } };
  }, { defaults, profiles, tile, options });
  await page.goto("/");
  await expect.poll(() => page.evaluate(() => window.__settingsFixture.calls.some(call => call.command === "settings_get"))).toBe(true);
  return remote;
}

const callsFor = (page, command) => page.evaluate(command => window.__settingsFixture.calls.filter(call => call.command === command), command);
const persistedSettings = page => page.evaluate(() => structuredClone(window.__settingsFixture.persisted.settings));
async function closeSettings(page) {
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
}

test("settings and catalogue load before cached profile verification, and explicit defaults win initially", async ({ page }) => {
  const remote = await bootSettings(page, { cache: true, holdReads: true,
    settings: { default_profile: "demo-b", default_region: "us-east-1", theme: "light" } });
  expect(await callsFor(page, "aws_set_account")).toEqual([]);
  expect(await callsFor(page, "aws_list_profiles")).toEqual([]);
  expect(await callsFor(page, "widget_fetch")).toEqual([]);
  await expect(page.locator("#theme-toggle")).toBeDisabled();
  await page.evaluate(() => { window.__settingsFixture.holdReads = false; window.__settingsFixture.releaseRead(); });
  await expect.poll(async () => (await callsFor(page, "aws_set_account")).length).toBe(1);
  expect((await callsFor(page, "aws_set_account"))[0].params).toMatchObject({ profile: "demo-b", region: "us-east-1" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("#appearance-unsaved")).toBeHidden();
  expect(await callsFor(page, "settings_set")).toEqual([]);
  expect(remote).toEqual([]);
});

test("missing settings metadata fails closed instead of using frontend defaults", async ({ page }) => {
  await bootSettings(page, { omitMetadata: true, cache: true });
  await expect(page.locator("#settings-storage-warning")).toBeVisible();
  expect(await callsFor(page, "aws_list_profiles")).toEqual([]);
  expect(await callsFor(page, "aws_set_account")).toEqual([]);
  expect(await callsFor(page, "settings_set")).toEqual([]);
});

test("theme preview and default preferences save without replacing verified evidence, and restore on reopen", async ({ page }) => {
  await bootSettings(page, { settings: { default_profile: "demo-a" } });
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("evidence-demo-a");
  const before = await callsFor(page, "aws_set_account");
  await page.locator("#theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("#appearance-unsaved")).toBeVisible();
  expect(await callsFor(page, "settings_set")).toEqual([]);
  await page.locator("#appearance-settings").click();
  await expect(page.locator("#settings-theme")).toHaveValue("light");
  await expect(page.locator("#settings-default-profile")).toHaveAttribute("placeholder", "(none)");
  await page.locator("#settings-default-profile").fill("demo-b");
  await page.locator("#settings-default-region").selectOption("us-east-1");
  await page.evaluate(() => { window.__settingsFixture.failSave = true; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes could not be saved");
  await expect(page.locator("#appearance-unsaved")).toBeVisible();
  expect((await persistedSettings(page)).theme).toBe("dark");
  await closeSettings(page);
  await page.locator("#appearance-settings").click();
  await expect(page.locator("#settings-theme")).toHaveValue("light");
  await expect(page.locator("#settings-default-profile")).toHaveValue("demo-b");
  await page.evaluate(() => { window.__settingsFixture.failSave = false; window.__settingsFixture.holdSaves = true; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saving…");
  await expect(page.locator("#settings-save")).toBeDisabled();
  await page.evaluate(() => { window.__settingsFixture.releaseSave(); });
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect(page.locator("#appearance-unsaved")).toBeHidden();
  expect(await callsFor(page, "aws_set_account")).toHaveLength(before.length);
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("evidence-demo-a");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("demo-b");
  await expect(page.locator("#settings-default-region")).toHaveValue("us-east-1");
});

test("unsupported saved region stays visible and rejected save identifies its field", async ({ page }) => {
  await bootSettings(page, { settings: { default_profile: "demo-a", default_region: "ap-south-1" } });
  await expect(page.locator("#region-picker-search")).toHaveValue("ap-south-1 — unsupported");
  expect(await callsFor(page, "aws_set_account")).toEqual([]);
  await expect(page.locator("#settings-storage-warning")).toHaveCount(0);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-region")).toHaveValue("ap-south-1");
  await expect(page.locator("#settings-default-region-error")).toHaveText("Choose a supported default region");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes were rejected");
  await expect(page.locator("#settings-default-region")).toHaveAttribute("aria-invalid", "true");
  expect((await persistedSettings(page)).default_region).toBe("ap-south-1");
  await page.locator("#settings-default-region").selectOption("us-east-1");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect(page.locator("#settings-default-region-error")).toBeHidden();
});

test("unsupported saved theme is an editable field error, with no automatic replacement", async ({ page }) => {
  await bootSettings(page, { settings: { theme: "synthetic-unsupported-theme" } });
  await expect(page.locator("#theme-toggle")).toBeEnabled();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("#settings-storage-warning")).toHaveCount(0);
  expect(await callsFor(page, "settings_set")).toEqual([]);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-theme")).toHaveValue("synthetic-unsupported-theme");
  await expect(page.locator("#settings-theme-error")).toHaveText("Choose light or dark theme");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes were rejected");
  expect((await persistedSettings(page)).theme).toBe("synthetic-unsupported-theme");
  await page.locator("#settings-theme").selectOption("light");
  await expect(page.locator("#appearance-unsaved")).toBeVisible();
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect(page.locator("#settings-theme-error")).toBeHidden();
});

test("late credential save invalidates a newer account and pinned evidence before re-verifying the current account", async ({ page }) => {
  const pin = { id: "cfn-stacks:pin", widget: "cfn-stacks", x: 6, y: 0, w: 6, h: 4,
    config: { context: { mode: "pinned", profile: "demo-a", account_id: profiles[0].account_id, region: "eu-west-1" } } };
  await bootSettings(page, { settings: { default_profile: "demo-a" }, tiles: [tile, pin] });
  const inherited = page.locator('[gs-id="cfn-stacks"]');
  const pinned = page.locator('[gs-id="cfn-stacks:pin"]');
  await expect(inherited).toContainText("evidence-demo-a");
  await expect(pinned).toContainText("evidence-demo-a");
  await page.locator("#settings-btn").click();
  await page.locator("#settings-aws-config-path").fill("/synthetic/new-source");
  await page.evaluate(() => { window.__settingsFixture.holdSaves = true; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saving…");
  await closeSettings(page);
  await page.locator("#account-picker-search").fill("demo-b");
  await page.locator("#topbar-picker-list .combo-item").filter({ hasText: "demo-b" }).click();
  await expect(inherited).toContainText("evidence-demo-b");
  await expect(pinned).toContainText("evidence-demo-a");
  await page.evaluate(() => { window.__settingsFixture.holdVerification = true; window.__settingsFixture.releaseSave(); });
  await expect.poll(async () => (await callsFor(page, "aws_set_account")).length).toBe(3);
  await expect(inherited).not.toContainText("evidence-demo-b");
  await expect(pinned).not.toContainText("evidence-demo-a");
  await expect(page.locator("#auth-status")).not.toContainText("auth: ok");
  expect((await callsFor(page, "aws_set_account"))[2].params.profile).toBe("demo-b");
  await page.evaluate(() => { window.__settingsFixture.releaseVerification(); });
  await expect(inherited).toContainText("evidence-demo-b");
  await expect(page.locator("#account-picker-search")).toHaveValue(/demo-b/);
  expect((await callsFor(page, "aws_set_account")).every((call, index) => index === 0 || call.params.profile === "demo-b")).toBe(true);
});

test("backend catalogue drives default and pinned selectors without moving an unsupported pin", async ({ page }) => {
  const context = { mode: "pinned", profile: "demo-a", account_id: profiles[0].account_id, region: "ap-south-1" };
  await bootSettings(page, { regions: ["eu-west-1", "us-east-1", "eu-central-1"], tiles: [{ ...tile, config: { context } }] });
  await expect(page.locator("#region-select option")).toHaveCount(3);
  await page.locator("#settings-btn").click();
  await expect(page.locator('#settings-default-region option[value="eu-central-1"]')).toHaveCount(1);
  await closeSettings(page);
  await page.locator('[data-widget="cfn-stacks"] .cfg-btn').click();
  await expect(page.locator("#cfg-override-region")).toHaveValue("ap-south-1");
  await expect(page.locator('#cfg-override-region option[value="ap-south-1"]')).toHaveText("ap-south-1 — unsupported");
  await expect(page.locator('#cfg-override-region option[value="eu-central-1"]')).toHaveCount(1);
  await page.locator('.color-swatch[data-color="blue"]').click();
  await page.locator("#cfg-save").click();
  await expect(page.locator("#cfg-context-error")).toContainText("Choose a supported region");
  expect(await callsFor(page, "dashboard_set")).toEqual([]);
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config.context)).toEqual(context);
});

test("an unavailable profile remains pinned after an unrelated color edit and reload", async ({ page }) => {
  const context = { mode: "pinned", profile: "synthetic-removed", account_id: "333333333333", region: "eu-west-1" };
  const remote = await bootSettings(page, { tiles: [{ ...tile, config: { context } }] });
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  const widget = page.locator('[data-widget="cfn-stacks"]');
  await widget.locator(".cfg-btn").click();
  await expect(page.locator("#cfg-use-override")).toBeChecked();
  await expect(page.locator("#cfg-override-profile")).toHaveValue(context.profile);
  await expect(page.locator("#cfg-override-profile option:checked")).toContainText("unavailable (saved pin)");
  await expect(page.locator("#cfg-context-error")).toContainText("pinned context is preserved");
  await page.locator('.color-swatch[data-color="blue"]').click();
  await page.locator("#cfg-save").click();
  await expect(page.locator("#widget-config-panel")).toHaveAttribute("aria-hidden", "true");
  await page.clock.fastForward(500);
  await expect(page.locator("#layout-save-status")).toHaveText("Layout saved.");
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config)).toMatchObject({ context, header_color: "blue" });
  expect((await callsFor(page, "widget_fetch")).every(call => JSON.stringify(call.params.context) === JSON.stringify(context))).toBe(true);
  await page.reload();
  await widget.locator(".cfg-btn").click();
  await expect(page.locator("#cfg-use-override")).toBeChecked();
  await expect(page.locator("#cfg-override-profile")).toHaveValue(context.profile);
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config.context)).toEqual(context);
  expect(remote).toEqual([]);
});

test("new or incomplete pinned context cannot silently fall back to the default account", async ({ page }) => {
  await bootSettings(page);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await page.locator('[data-widget="cfn-stacks"] .cfg-btn').click();
  await page.locator("#cfg-use-override").check();
  await expect(page.locator("#cfg-override-account")).toHaveValue("");
  await page.locator("#cfg-save").click();
  await expect(page.locator("#cfg-context-error")).toContainText("Choose a supported profile with an account");
  await expect(page.locator("#widget-config-panel")).toHaveAttribute("aria-hidden", "false");
  expect(await callsFor(page, "dashboard_set")).toEqual([]);

  await page.locator("#cfg-override-profile").selectOption("demo-b");
  await expect(page.locator("#cfg-override-account")).toHaveValue(profiles[1].account_id);
  await page.locator("#cfg-override-profile").selectOption("");
  await page.locator("#cfg-save").click();
  await expect(page.locator("#cfg-context-error")).toContainText("Choose a supported profile with an account");
  expect(await callsFor(page, "dashboard_set")).toEqual([]);

  await page.locator("#cfg-override-profile").selectOption("demo-b");
  await page.locator("#cfg-save").click();
  await page.clock.fastForward(500);
  await expect(page.locator("#layout-save-status")).toHaveText("Layout saved.");
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config.context)).toEqual({
    mode: "pinned", profile: "demo-b", account_id: profiles[1].account_id, region: "eu-west-1",
  });
});

test("an unavailable saved profile cannot be repurposed into a new pinned context", async ({ page }) => {
  const context = { mode: "pinned", profile: "synthetic-removed", account_id: "333333333333", region: "eu-west-1" };
  await bootSettings(page, { tiles: [{ ...tile, config: { context } }] });
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await page.locator('[data-widget="cfn-stacks"] .cfg-btn').click();
  await page.locator("#cfg-override-region").selectOption("us-east-1");
  await page.locator("#cfg-save").click();
  await expect(page.locator("#cfg-context-error")).toContainText("Choose an available supported profile");
  expect(await callsFor(page, "dashboard_set")).toEqual([]);
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config.context)).toEqual(context);
  await page.locator("#cfg-use-override").uncheck();
  await page.locator("#cfg-save").click();
  await page.clock.fastForward(500);
  await expect(page.locator("#layout-save-status")).toHaveText("Layout saved.");
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles[0].config?.context)).toBeUndefined();
});

test("credential source changes reverify the context while layout save states stay truthful", async ({ page }) => {
  await bootSettings(page, { settings: { default_profile: "demo-a" } });
  await expect(page.locator('[data-widget="cfn-stacks"]')).toContainText("evidence-demo-a");
  const before = (await callsFor(page, "aws_set_account")).length;
  await page.locator("#settings-btn").click();
  await page.locator("#settings-aws-config-path").fill("/synthetic/other-config");
  await page.locator("#settings-sso-session").fill("synthetic-other-session");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect.poll(async () => (await callsFor(page, "aws_set_account")).length).toBe(before + 1);
  await closeSettings(page);
  await page.evaluate(() => { window.__settingsFixture.holdLayout = true; window.__settingsFixture.failLayout = true; });
  await page.locator('[data-widget="cfn-stacks"] .rm-btn').click();
  await expect(page.locator("#layout-save-status")).toHaveText("Unsaved layout changes.");
  await page.clock.fastForward(500);
  await expect(page.locator("#layout-save-status")).toHaveText("Saving layout…");
  await page.evaluate(() => { window.__settingsFixture.releaseLayout(); });
  await expect(page.locator("#layout-save-status")).toContainText("Layout save failed");
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles)).toHaveLength(1);
  await page.evaluate(() => { window.__settingsFixture.holdLayout = false; window.__settingsFixture.failLayout = false; });
  await page.locator("#layout-save-warning").getByRole("button", { name: "Retry save" }).click();
  await expect(page.locator("#layout-save-status")).toHaveText("Layout saved.");
  expect(await page.evaluate(() => window.__settingsFixture.persisted.tiles)).toEqual([]);
});
