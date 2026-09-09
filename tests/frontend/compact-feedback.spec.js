import { expect, test } from "@playwright/test";

const identity = { profile: "demo-compact", account_id: "acct-compact-fixture", region: "eu-west-1" };
const connectionKey = "cloud-burrito.connection-details-open";
const displayKey = "cloud-burrito.display-preferences";
const coverage = (completeness = "complete", returned = 1) => ({
  completeness, has_more: completeness === "limited" ? true : completeness === "complete" ? false : null,
  counts: { returned, pages: 1 }, limits: { results: 10, retained_item_bytes: 2097152 }, reasons: [],
});
const table = (marker = "synthetic-current-stack", count = 1) => ({
  render: "table", columns: ["stack", "status"],
  rows: Array.from({ length: count }, (_, index) => ({ stack: `${marker}-${index + 1}`, status: "CREATE_COMPLETE" })),
  coverage: coverage("complete", count),
});

// Production UI with a synthetic native boundary. No credential files, CLI,
// native app, AWS endpoint, or browser request beyond the local server is used.
async function boot(page, options = {}) {
  const remote = [];
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remote.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  await page.addInitScript(({ identity, connectionKey, displayKey, options, initialResult }) => {
    if (options.blockPreferenceStorage) {
      const get = Storage.prototype.getItem, set = Storage.prototype.setItem;
      const blocked = key => key === connectionKey || key === displayKey;
      Storage.prototype.getItem = function (key) {
        if (this === localStorage && blocked(key)) throw new DOMException("Synthetic storage unavailable", "SecurityError");
        return get.call(this, key);
      };
      Storage.prototype.setItem = function (key, value) {
        if (this === localStorage && blocked(key)) throw new DOMException("Synthetic storage unavailable", "SecurityError");
        return set.call(this, key, value);
      };
    }
    const defaults = { aws_config_path: "/synthetic/aws/config", sso_session_name: "", default_profile: "",
      default_region: "eu-west-1", theme: "dark", audit_retention: "preserve" };
    const settings = { ...defaults, default_profile: identity.profile, _storage: { status: "loaded" },
      _settings: { defaults, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} } };
    const tiles = [
      { id: "cfn-stacks", widget: "cfn-stacks", x: 0, y: 0, w: 12, h: 6 },
      { id: "pipeline-runs", widget: "pipeline-runs", x: 0, y: 6, w: 12, h: 6 },
    ];
    const fixture = window.__compact = { calls: [], authFailure: null, holdWidget: false,
      response: initialResult, widgetReplies: 0 };
    let active = null, contextSerial = 0;
    function attach(value, params, context = active) {
      return { ...value, _request: {
        id: params.request_id || null, context_id: context ? `synthetic-compact-${contextSerial}` : null,
        provider_revision: context ? "synthetic-provider" : null, settings_revision: context ? "1" : null,
        profile: context?.profile || null, account_id: context?.account_id || null, region: context?.region || null,
        audit_id: "synthetic-audit", outcome: value.ok === false || value.error || value.data?.error ? "failed" : "succeeded",
      } };
    }
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      fixture.calls.push({ command, params: structuredClone(params) });
      switch (command) {
        case "ping": return { version: "synthetic" };
        case "settings_get": return settings;
        case "settings_set": return { ...settings, ...params, _storage: { status: "saved" } };
        case "dashboard_get": return { tiles, _storage: { status: "loaded" } };
        case "dashboard_set": return { ok: true };
        case "aws_list_profiles": return { discovery_state: "ready", file_exists: true, profiles: [{
          name: identity.profile, account_id: identity.account_id, region: identity.region,
          role_name: "SyntheticReadOnly", sso_session: "synthetic-session", eligibility: "supported_sso",
        }] };
        case "aws_set_account":
          active = { profile: params.profile, account_id: params.account_id, region: params.region };
          contextSerial++;
          return attach({ ok: true }, params);
        case "aws_auth_status":
          if (fixture.authFailure) active = null;
          return attach({ has_context: !!active, logged_in: !!active, ...active,
            connection_state: active ? "verified" : fixture.authFailure ? "expired" : "unselected",
            error_type: fixture.authFailure, needs_sso_login: fixture.authFailure === "CredentialsExpired" }, params);
        case "aws_list_pipelines": return attach({ ok: true, pipelines: [{ name: "synthetic-pipeline" }] }, params);
        case "widget_fetch":
          if (params.widget !== "cfn-stacks") return attach({ render: "table", columns: ["execution_id"], rows: [] }, params);
          if (fixture.holdWidget) await new Promise(resolve => { fixture.releaseWidget = resolve; });
          fixture.widgetReplies++;
          return attach(structuredClone(fixture.response), params);
        case "request_cancel": return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
        case "policy_get": return { raw: "Statement: []", valid: true, actions: [], path: "/synthetic/policy.yaml" };
        case "audit_history":
          if (params.action !== "status") throw new Error("Unexpected synthetic audit mutation");
          return { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0,
            total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." };
        case "audit_tail": return { entries: [] };
        default: throw new Error(`Unexpected synthetic invoke: ${command}`);
      }
    } } };
  }, { identity, connectionKey, displayKey, options, initialResult: table() });
  await page.goto(options.design === "classic" ? "/?design=classic" : "/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(resultHost(page)).toContainText("synthetic-current-stack-1");
  return remote;
}

const resultHost = page => page.locator('[data-widget="cfn-stacks"] .cfn-stacks-body');
const status = page => resultHost(page).locator(":scope > .result-status");
const allCalls = page => page.evaluate(() => window.__compact.calls.length);
const operationalCalls = page => page.evaluate(() => window.__compact.calls.filter(call =>
  ["settings_set", "aws_list_profiles", "aws_set_account", "widget_fetch"].includes(call.command)));
const stored = (page, key) => page.evaluate(key => localStorage.getItem(key), key);
async function openSettings(page) {
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-panel")).toHaveAttribute("aria-hidden", "false");
  await expect(page.locator("#policy-editor")).toHaveValue("Statement: []");
}
async function closeSettings(page) {
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
}
async function refreshWith(page, response) {
  const before = await page.evaluate(response => {
    window.__compact.response = response;
    return window.__compact.widgetReplies;
  }, response);
  await page.locator('[data-widget="cfn-stacks"] .widget-header button[title="Refresh"]').click();
  await expect.poll(() => page.evaluate(() => window.__compact.widgetReplies)).toBeGreaterThan(before);
}

for (const design of ["studio", "classic"]) {
  test(`${design}: connection disclosure closes without requests, persists, and stays closed through status polls`, async ({ page }) => {
    const remote = await boot(page, { design });
    const toggle = page.locator("#connection-toggle"), panel = page.locator("#connection-status");
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-controls", "connection-status");
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    const before = await allCalls(page);
    await page.getByRole("button", { name: "Close connection details", exact: true }).click();
    await expect(panel).toBeHidden();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
    expect(await stored(page, connectionKey)).toBe("false");
    await toggle.click();
    await expect(panel).toBeVisible();
    await toggle.click();
    expect(await allCalls(page)).toBe(before);
    await page.clock.fastForward(60_000);
    await expect(panel).toHaveAttribute("data-state", "verified");
    await expect(panel).toBeHidden();
    if (design === "studio") await expect(page.locator("#studio-mode-label")).toHaveAttribute("data-state", "verified");
    await page.reload();
    await expect(panel).toHaveAttribute("data-state", "verified");
    await expect(panel).toBeHidden();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(remote).toEqual([]);
  });
}

test("a newly expired connection opens recovery once, and repeated expiry polls respect dismissal", async ({ page }) => {
  await boot(page);
  await page.locator("#connection-close").click();
  await page.evaluate(() => { window.__compact.authFailure = "CredentialsExpired"; });
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "expired");
  await expect(page.locator("#connection-status")).toBeVisible();
  await expect(page.locator("#connection-toggle")).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#connection-retry")).toBeEnabled();
  await expect(page.locator("#connection-message")).toContainText("outside this app");
  await expect(resultHost(page)).not.toContainText("synthetic-current-stack-1");
  await page.locator("#connection-close").click();
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "expired");
  await expect(page.locator("#connection-status")).toBeHidden();
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "offline");
});

test("compact results keep status and receipt visible while technical details disclose on demand and survive refresh", async ({ page }) => {
  await boot(page);
  const feedback = status(page), details = feedback.locator(".result-details");
  await expect(feedback.getByText("Updated", { exact: true })).toBeVisible();
  await expect(feedback.locator("time")).toBeVisible();
  await expect(details.locator("summary")).toHaveText("Result details");
  await expect(details).not.toHaveAttribute("open", "");
  await expect(details.locator(".result-context")).toBeHidden();
  await expect(feedback.getByText(/Retained Item Bytes limit/)).toBeHidden();
  await expect(page.locator('[data-widget="cfn-stacks"] .widget-context')).toBeVisible();
  await details.locator("summary").click();
  await expect(details.locator(".result-context")).toBeVisible();
  await expect(details.locator(".result-context")).toContainText(identity.account_id);
  await expect(feedback.getByText(/Retained Item Bytes limit/)).toBeVisible();
  const original = await feedback.locator("time").getAttribute("datetime");
  await page.evaluate(() => { window.__compact.holdWidget = true; });
  await page.locator('[data-widget="cfn-stacks"] .widget-header button[title="Refresh"]').click();
  await expect(feedback).toHaveAttribute("data-state", "loading");
  await expect.poll(() => page.evaluate(() => typeof window.__compact.releaseWidget)).toBe("function");
  await expect(details.locator(".result-context")).toBeVisible();
  expect(await feedback.locator("time").getAttribute("datetime")).toBe(original);
  await details.locator("summary").focus();
  await expect(details.locator("summary")).toBeFocused();
  await page.evaluate(() => { window.__compact.holdWidget = false; window.__compact.releaseWidget(); });
  await expect(feedback).toHaveAttribute("data-state", "success");
  await expect(details.locator(".result-context")).toBeVisible();
  await expect(details.locator("summary")).toBeFocused();
});

test("a failed refresh preserves disclosure focus and later completion cannot steal focus from Settings", async ({ page }) => {
  await boot(page);
  const feedback = status(page), summary = feedback.locator(".result-details summary");
  const holdRefresh = async () => {
    await page.evaluate(() => { window.__compact.holdWidget = true; window.__compact.releaseWidget = null; });
    await page.locator('[data-widget="cfn-stacks"] .widget-header button[title="Refresh"]').click();
    await expect(feedback).toHaveAttribute("data-state", "loading");
    await expect.poll(() => page.evaluate(() => typeof window.__compact.releaseWidget)).toBe("function");
  };
  const releaseFailure = () => page.evaluate(() => {
    window.__compact.response = { render: "raw_json", data: { error: "Synthetic focus-preserving refresh failure." } };
    window.__compact.holdWidget = false;
    window.__compact.releaseWidget();
  });
  await holdRefresh();
  await summary.focus();
  await expect(summary).toBeFocused();
  await releaseFailure();
  await expect(feedback).toHaveAttribute("data-state", "stale");
  await expect(summary).toBeFocused();
  await expect(feedback.getByText("Synthetic focus-preserving refresh failure.", { exact: true })).toBeVisible();
  await holdRefresh();
  await summary.focus();
  await openSettings(page);
  const preference = page.locator("#settings-show-tips");
  await preference.focus();
  await expect(preference).toBeFocused();
  await releaseFailure();
  await expect(feedback).toHaveAttribute("data-state", "stale");
  await expect(preference).toBeFocused();
  await closeSettings(page);
  await expect(page.locator("#settings-btn")).toBeFocused();
});

test("the topbar Connection button reveals usable details from a fullscreen widget", async ({ page }) => {
  await boot(page, { design: "classic" });
  await page.locator("#connection-close").click();
  const widget = page.locator('[data-widget="cfn-stacks"]');
  await widget.locator(".fs-btn").click();
  await expect(widget).toHaveClass(/fullscreen/);
  await expect(page.locator("body")).toHaveClass(/has-fullscreen-widget/);
  const toggle = page.locator("#connection-toggle"), panel = page.locator("#connection-status");
  const before = await operationalCalls(page);
  await toggle.click();
  await expect(widget).not.toHaveClass(/fullscreen/);
  await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(toggle).toBeFocused();
  await expect(panel).toBeInViewport();
  await page.locator("#connection-close").click();
  await expect(panel).toBeHidden();
  await expect(toggle).toBeFocused();
  expect(await operationalCalls(page)).toEqual(before);
});

test("limited, partial, stale, and remote-cleanup warnings stay visible with technical details closed", async ({ page }) => {
  await boot(page);
  const limited = { ...table("synthetic-limited", 10), coverage: coverage("limited", 10) };
  await refreshWith(page, limited);
  await expect(status(page)).toHaveAttribute("data-state", "limited");
  await expect(status(page).getByText("Showing 10 results. More were not loaded.", { exact: true })).toBeVisible();
  await expect(status(page).locator(".result-context")).toBeHidden();
  await refreshWith(page, { ...table("synthetic-partial"), ok: false, partial: true,
    error: "Synthetic second page could not be loaded.", coverage: coverage("unknown") });
  await expect(status(page)).toHaveAttribute("data-state", "partial");
  await expect(resultHost(page).locator(".table-error")).toContainText("Synthetic second page could not be loaded.");
  await expect(resultHost(page).locator(".table-error")).toBeVisible();
  const original = await status(page).locator("time").getAttribute("datetime");
  await refreshWith(page, { render: "raw_json", data: { error: "Synthetic refresh failed." }, coverage: coverage("unknown", 0) });
  await expect(status(page)).toHaveAttribute("data-state", "stale");
  await expect(status(page).getByText("Synthetic refresh failed.", { exact: true })).toBeVisible();
  await expect(status(page).getByText(/Refresh did not complete/)).toBeVisible();
  expect(await status(page).locator("time").getAttribute("datetime")).toBe(original);
  await expect(resultHost(page)).toContainText("synthetic-partial-1");
  await refreshWith(page, { render: "raw_json", data: { error: "Synthetic query cancellation.", error_type: "QueryCancelled" },
    cleanup: { status: "not_confirmed", remote_queries_may_still_run: true } });
  await expect(status(page).getByText("Remote stop was not confirmed.", { exact: true })).toBeVisible();
  await expect(status(page).getByText("Remote queries may still be running.", { exact: true })).toBeVisible();
  await expect(status(page).locator(".result-context")).toBeHidden();
});

test("display preferences save immediately without AWS settings writes, survive reload, and apply to later results", async ({ page }) => {
  await boot(page);
  const hint = page.locator('[data-widget="pipeline-runs"] .ui-hint');
  await expect(hint).toHaveCount(1);
  await expect(hint).toBeHidden();
  await openSettings(page);
  const tips = page.locator("#settings-show-tips"), expanded = page.locator("#settings-expand-result-details");
  await expect(tips).not.toBeChecked();
  await expect(expanded).not.toBeChecked();
  expect(await tips.evaluate(node => !!node.closest("#settings-form"))).toBe(false);
  expect(await expanded.evaluate(node => !!node.closest("#settings-form"))).toBe(false);
  const before = await operationalCalls(page);
  await tips.check();
  await expanded.check();
  expect(JSON.parse(await stored(page, displayKey))).toEqual({ showTips: true, expandResultDetails: true });
  expect(await operationalCalls(page)).toEqual(before);
  await closeSettings(page);
  await expect(hint).toBeVisible();
  await expect(status(page).locator(".result-context")).toBeVisible();
  await refreshWith(page, table("synthetic-after-preference"));
  await expect(status(page).locator(".result-context")).toBeVisible();
  await page.reload();
  await expect(resultHost(page)).toContainText("synthetic-current-stack-1");
  await expect(hint).toBeVisible();
  await expect(status(page).locator(".result-context")).toBeVisible();
  await openSettings(page);
  await expect(tips).toBeChecked();
  await expect(expanded).toBeChecked();
  await tips.uncheck();
  await expanded.uncheck();
  await closeSettings(page);
  await expect(hint).toBeHidden();
  await expect(status(page).locator(".result-context")).toBeHidden();
});

test("unavailable preference storage preserves session choices with an honest persistence notice", async ({ page }) => {
  const remote = await boot(page, { blockPreferenceStorage: true });
  await openSettings(page);
  const before = await operationalCalls(page);
  await page.locator("#settings-show-tips").check();
  await page.locator("#settings-expand-result-details").check();
  await expect(page.locator("#display-preferences-status")).toContainText(/session/i);
  expect(await operationalCalls(page)).toEqual(before);
  await closeSettings(page);
  await expect(page.locator('[data-widget="pipeline-runs"] .ui-hint')).toBeVisible();
  await expect(status(page).locator(".result-context")).toBeVisible();
  await page.locator("#connection-close").click();
  await page.clock.fastForward(60_000);
  await expect(page.locator("#connection-status")).toBeHidden();
  expect(remote).toEqual([]);
});
