import { expect, test } from "@playwright/test";

const initialSettings = { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "saved-demo", default_region: "eu-west-1", theme: "dark" };
const initialDashboard = { version: 1, tiles: [{ id: "resource-lookup", widget: "resource-lookup", x: 0, y: 0, w: 6, h: 4,
  config: { header_color: "blue", inputs: { query: "synthetic saved query" } } }] };

// This fresh browser context holds synthetic file bytes across page reloads.
// Only IPC is fake. No native filesystem, AWS SDK, CLI or credential source is
// opened. Actual atomic-replacement behavior is covered by backend tests.
async function boot(page, files = {}) {
  const remoteRequests = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remoteRequests.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  await page.addInitScript(({ files }) => {
    const key = "cloud-burrito-persistence-synthetic";
    const state = JSON.parse(sessionStorage.getItem(key) || "null") || {
      files: { settings: files.settings ?? null, dashboard: files.dashboard ?? null },
      readFailures: {}, writeFailures: {}, calls: [],
    };
    const persist = () => sessionStorage.setItem(key, JSON.stringify(state));
    const defaults = { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
    const settingsMetadata = () => ({ defaults: { ...defaults }, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} });
    const loadedResponse = (store, value, status) => ({
      ...(store === "settings" ? defaults : {}), ...value,
      ...(store === "settings" ? { _settings: settingsMetadata() } : {}),
      _storage: { status, store },
    });
    const failure = (store, error_type) => ({ ok: false, error_type, error: "Synthetic storage operation failed",
      ...(store === "settings" ? { _settings: settingsMetadata() } : {}),
      _storage: { status: "failed", store } });
    const load = store => {
      if (state.readFailures[store]) return failure(store, "StorageReadFailed");
      if (state.files[store] === null) return loadedResponse(store, store === "settings" ? defaults : { version: 1, tiles: [] }, "missing");
      try {
        const value = JSON.parse(state.files[store]);
        if (!value || typeof value !== "object" || Array.isArray(value)
            || (store === "dashboard" && !Array.isArray(value.tiles))) throw new Error("Invalid synthetic shape");
        return loadedResponse(store, value, "loaded");
      } catch (_) { return failure(store, "StorageInvalid"); }
    };
    const fixture = window.__persistenceFixture = { state, persist };
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      state.calls.push(command);
      persist();
      switch (command) {
        case "ping": return { version: "synthetic" };
        case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
        case "settings_get": return load("settings");
        case "dashboard_get": return load("dashboard");
        case "settings_set": case "dashboard_set": {
          const store = command === "settings_set" ? "settings" : "dashboard";
          if (store === "settings" && fixture.holdSettingsSave) {
            await new Promise(resolve => { fixture.finishSettingsSave = resolve; });
          }
          if (state.writeFailures[store]) return failure(store, state.writeFailures[store]);
          const value = store === "settings" ? Object.fromEntries(Object.entries(defaults).map(([name, fallback]) => [name, params[name]?.trim() || fallback]))
            : { version: 1, tiles: params.tiles };
          state.files[store] = JSON.stringify(value);
          persist();
          return loadedResponse(store, value, "saved");
        }
        case "aws_list_profiles": return { profiles: [], file_exists: false, error: null, discovery_state: "missing_config" };
        case "aws_auth_status": return { has_context: false, logged_in: false, connection_state: "disconnected" };
        case "policy_get": return { raw: "Statement: []", valid: true, actions: [], path: "/synthetic/policy.yaml" };
        case "audit_tail": return { entries: [] };
        default: throw new Error("Unexpected synthetic boundary invocation");
      }
    } } };
  }, { files });
  await page.goto("/");
  await expect.poll(() => page.evaluate(() => window.__persistenceFixture.state.calls.includes("settings_get")
    && window.__persistenceFixture.state.calls.includes("dashboard_get"))).toBe(true);
  return remoteRequests;
}

const snapshot = page => page.evaluate(() => structuredClone(window.__persistenceFixture.state));
const writes = state => state.calls.filter(command => command.endsWith("_set"));
const noAws = state => expect(state.calls.filter(command => ["aws_set_account", "aws_list_pipelines", "widget_fetch"].includes(command))).toEqual([]);

test("missing files keep first-run defaults without writing, while loaded empty layout stays empty", async ({ page }) => {
  const remote = await boot(page);
  await expect(page.locator("#grid-stack > .grid-stack-item")).not.toHaveCount(0);
  await expect(page.locator(".storage-load-warning")).toHaveCount(0);
  await page.clock.fastForward(1000);
  expect(writes(await snapshot(page))).toEqual([]);
  await page.evaluate(() => {
    const fixture = window.__persistenceFixture;
    fixture.state.files.dashboard = JSON.stringify({ version: 1, tiles: [] });
    fixture.persist();
  });
  await page.reload();
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(0);
  await page.clock.fastForward(1000);
  const state = await snapshot(page);
  expect(writes(state)).toEqual([]);
  noAws(state);
  expect(remote).toEqual([]);
});

test("corrupt files show recovery and UI changes cannot autosave over them", async ({ page }) => {
  const files = { settings: "{synthetic malformed settings", dashboard: "{synthetic malformed dashboard" };
  await boot(page, files);
  await expect(page.locator("#settings-storage-warning")).toBeVisible();
  await expect(page.locator("#dashboard-storage-warning")).toBeVisible();
  await page.locator(".grid-stack-item .rm-btn").first().click();
  await page.clock.fastForward(1000);
  const state = await snapshot(page);
  expect(state.files).toEqual(files);
  expect(writes(state)).toEqual([]);
  noAws(state);
  await page.evaluate(() => {
    const fixture = window.__persistenceFixture;
    fixture.state.files.dashboard = JSON.stringify({ version: 1, tiles: [] });
    fixture.persist();
  });
  await page.locator("#dashboard-storage-warning").getByRole("button", { name: "Retry load" }).click();
  await expect(page.locator("#dashboard-storage-warning")).toHaveCount(0);
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(0);
  await page.clock.fastForward(1000);
  expect(writes(await snapshot(page))).toEqual([]);
});

test("settings read failure preserves an edited draft and blocks save until recovery", async ({ page }) => {
  await boot(page, { settings: JSON.stringify(initialSettings), dashboard: JSON.stringify(initialDashboard) });
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("saved-demo");
  await page.locator("#settings-default-profile").fill("edited-draft");
  await page.evaluate(() => {
    window.__persistenceFixture.state.readFailures.settings = true;
    window.__persistenceFixture.persist();
  });
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-storage-warning")).toBeVisible();
  await expect(page.locator("#settings-default-profile")).toHaveValue("edited-draft");
  await expect(page.locator("#settings-save")).toBeDisabled();
  await page.evaluate(() => document.querySelector("#settings-form").dispatchEvent(new Event("submit", { cancelable: true })));
  const state = await snapshot(page);
  expect(state.files.settings).toBe(JSON.stringify(initialSettings));
  expect(state.calls.filter(command => command === "settings_set")).toEqual([]);
  noAws(state);
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.evaluate(() => { window.__persistenceFixture.state.readFailures.settings = false; });
  await page.locator("#settings-storage-warning").getByRole("button", { name: "Retry load" }).click();
  await expect(page.locator("#settings-storage-warning")).toHaveCount(0);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("edited-draft");
  await expect(page.locator("#settings-save")).toBeEnabled();
  await expect(page.locator("#settings-status")).toHaveText("Current edits are not saved.");
  expect(writes(await snapshot(page))).toEqual([]);
});

test("failed settings replacement retains the prior file and draft, then retry survives reopen", async ({ page }) => {
  await boot(page, { settings: JSON.stringify(initialSettings), dashboard: JSON.stringify(initialDashboard) });
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("saved-demo");
  await page.locator("#settings-default-profile").fill("edited-demo");
  await page.evaluate(() => { window.__persistenceFixture.state.writeFailures.settings = "StorageWriteFailed"; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes could not be saved");
  await expect(page.locator("#settings-default-profile")).toHaveValue("edited-demo");
  expect((await snapshot(page)).files.settings).toBe(JSON.stringify(initialSettings));
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("edited-demo");
  await expect(page.locator("#settings-status")).toContainText("Changes could not be saved");
  await expect(page.locator("#settings-save")).toBeEnabled();
  await page.evaluate(() => { window.__persistenceFixture.state.writeFailures.settings = false; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await page.reload();
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("edited-demo");
  noAws(await snapshot(page));
});

test("reopening during a settings save does not load old values, and later edits remain unsaved", async ({ page }) => {
  await boot(page, { settings: JSON.stringify(initialSettings), dashboard: JSON.stringify(initialDashboard) });
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("saved-demo");
  await page.locator("#settings-default-profile").fill("first-draft");
  await page.evaluate(() => { window.__persistenceFixture.holdSettingsSave = true; });
  await page.locator("#settings-save").click();
  await expect.poll(() => page.evaluate(() => typeof window.__persistenceFixture.finishSettingsSave)).toBe("function");
  const reads = (await snapshot(page)).calls.filter(command => command === "settings_get").length;
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("first-draft");
  await expect(page.locator("#settings-save")).toBeDisabled();
  await expect(page.locator("#settings-status")).toHaveText("Saving…");
  expect((await snapshot(page)).calls.filter(command => command === "settings_get")).toHaveLength(reads);
  expect((await snapshot(page)).files.settings).toBe(JSON.stringify(initialSettings));
  await page.evaluate(() => { window.__persistenceFixture.finishSettingsSave(); });
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  expect(JSON.parse((await snapshot(page)).files.settings).default_profile).toBe("first-draft");

  await page.locator("#settings-default-profile").fill("second-draft");
  await page.evaluate(() => { delete window.__persistenceFixture.finishSettingsSave; });
  await page.locator("#settings-save").click();
  await expect.poll(() => page.evaluate(() => typeof window.__persistenceFixture.finishSettingsSave)).toBe("function");
  await page.locator("#settings-default-profile").fill("third-draft");
  await page.evaluate(() => { window.__persistenceFixture.finishSettingsSave(); });
  await expect(page.locator("#settings-status")).toHaveText("Previous values saved. Current edits are not saved.");
  await expect(page.locator("#settings-default-profile")).toHaveValue("third-draft");
  expect(JSON.parse((await snapshot(page)).files.settings).default_profile).toBe("second-draft");
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("third-draft");
  await expect(page.locator("#settings-status")).toHaveText("Previous values saved. Current edits are not saved.");
  noAws(await snapshot(page));
});

test("failed layout save preserves the prior file and explicit retry saves the current empty layout", async ({ page }) => {
  await boot(page, { settings: JSON.stringify(initialSettings), dashboard: JSON.stringify(initialDashboard) });
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(1);
  await page.evaluate(() => { window.__persistenceFixture.state.writeFailures.dashboard = "StorageWriteFailed"; });
  await page.locator(".grid-stack-item .rm-btn").click();
  await page.clock.fastForward(500);
  await expect(page.locator("#layout-save-warning")).toBeVisible();
  expect((await snapshot(page)).files.dashboard).toBe(JSON.stringify(initialDashboard));
  await page.evaluate(() => { window.__persistenceFixture.state.writeFailures.dashboard = false; });
  await page.locator("#layout-save-warning").getByRole("button", { name: "Retry save" }).click();
  await expect(page.locator("#layout-save-warning")).toHaveCount(0);
  await page.reload();
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(0);
  expect(JSON.parse((await snapshot(page)).files.dashboard).tiles).toEqual([]);
  noAws(await snapshot(page));
});

test("explicit recovery replaces corrupt settings and resets dashboard to saved starter descriptors", async ({ page }) => {
  await boot(page, { settings: "{bad synthetic settings", dashboard: "{bad synthetic dashboard" });
  await expect(page.locator("#settings-storage-warning")).toBeVisible();
  expect(writes(await snapshot(page))).toEqual([]);
  await page.locator("#settings-storage-warning").getByRole("button", { name: "Replace settings with defaults" }).click();
  await expect(page.locator("#settings-storage-warning")).toHaveCount(0);
  expect(JSON.parse((await snapshot(page)).files.settings).default_profile).toBe("");
  await page.locator("#dashboard-storage-warning").getByRole("button", { name: "Reset layout" }).click();
  await expect(page.locator("#dashboard-storage-warning")).toHaveCount(0);
  await expect(page.locator("#grid-stack > .grid-stack-item")).not.toHaveCount(0);
  const state = await snapshot(page);
  expect(JSON.parse(state.files.dashboard).tiles.length).toBeGreaterThan(0);
  expect(state.calls.filter(command => command === "dashboard_set")).toHaveLength(1);
  noAws(state);
});
