import { expect, test } from "@playwright/test";

// This suite loads production app.js. Only the native invoke boundary is fake;
// no AWS SDK, CLI, credential source, native app or remote endpoint is used.
const A = { profile: "demo-a", account_id: "acct-a-fixture", region: "eu-west-1" };
const B = { profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" };
const pinned = (identity = A) => ({ mode: "pinned", ...identity });
const tile = (widget, config = {}) => ({ id: widget, widget, x: 0, y: 0, w: 12, h: 8, config });
const widget = (page, name) => page.locator(`.widget[data-widget="${name}"]`);
const stacks = (marker) => ({
  render: "table", columns: ["stack", "status"],
  rows: [{ stack: marker, status: "CREATE_COMPLETE" }],
});
const runs = (marker) => ({
  render: "table", columns: ["execution_id", "status"],
  rows: [{ execution_id: marker, status: "Succeeded" }],
});
const cliRows = (marker) => ({ render: "table", columns: ["value"], rows: [{ value: marker }] });
const events = (marker) => ({ render: "log_stream", events: [{ ts: 1700000000000, msg: marker, level: "info" }] });
const pipelines = (name) => ({ ok: true, pipelines: [{ name, updated: "2026-01-01T00:00:00Z" }] });

async function boot(page, tiles = [tile("cfn-stacks")], { holdSelection = false } = {}) {
  const remoteRequests = [];
  await page.context().route("**/*", (route) => {
    const url = route.request().url();
    if (new URL(url).origin === "http://127.0.0.1:4173") return route.continue();
    remoteRequests.push(url);
    return route.abort("blockedbyclient");
  });
  await page.addInitScript(({ tiles, A, B, holdSelection }) => {
    let sequence = 0;
    let latestSelection = 0;
    let active = null;
    const calls = [];
    const holds = holdSelection ? [{ command: "aws_set_account" }] : [];
    const pending = new Map();
    const matches = (call, spec) => Object.entries(spec).every(([key, value]) => {
      if (key === "inputs") return Object.entries(value).every(([k, v]) => call.params.inputs?.[k] === v);
      if (key === "profile") return call.context.profile === value;
      if (key === "widget") return call.params.widget === value;
      return call[key] === value;
    });
    const table = (key, value) => ({ render: "table", columns: [key], rows: [{ [key]: value }] });
    const settingsDefaults = { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
    let settings = { ...settingsDefaults, default_profile: A.profile, default_region: A.region };
    const settingsResponse = status => ({ ...settings,
      _settings: { defaults: { ...settingsDefaults }, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} },
      _storage: { store: "settings", status } });
    function defaultResponse(call) {
      const { command, params, context } = call;
      switch (command) {
        case "ping": return { ok: true, version: "synthetic-test" };
        case "settings_get": return settingsResponse("loaded");
        case "dashboard_get": return { tiles };
        case "dashboard_set": return { ok: true };
        case "settings_set":
          settings = Object.fromEntries(Object.entries(settingsDefaults).map(([name, fallback]) => [name, params[name]?.trim() || fallback]));
          return settingsResponse("saved");
        case "aws_list_profiles": return {
          config_path: "/synthetic/aws/config", file_exists: true,
          profiles: [A, B].map((identity) => ({
            name: identity.profile, account_id: identity.account_id, region: identity.region,
            role_name: "SyntheticReadOnly", sso_session: "synthetic-session",
          })),
        };
        case "aws_set_account": return { ok: true, ...context };
        case "aws_auth_status": return {
          has_context: !!call.authContext, logged_in: !!call.authContext,
          connection_state: call.authContext ? "verified" : "unselected", ...call.authContext,
        };
        case "aws_list_pipelines": return { ok: true, pipelines: [{ name: "synthetic-pipeline", updated: "2026-01-01T00:00:00Z" }] };
        case "widget_fetch": {
          if (params.widget === "cfn-stacks") return table("stack", `initial-${context.profile}`);
          if (params.widget === "pipeline-runs") return {
            render: "table", columns: ["execution_id", "status"],
            rows: [{ execution_id: "synthetic-execution", status: "Succeeded" }],
          };
          if (params.widget === "aws-cli") return table("value", `initial-cli-${context.profile}`);
          if (params.widget === "codeartifact-packages") return {
            render: "table", columns: ["package", "latest_version"],
            rows: [{ package: `package-${context.profile}`, latest_version: "1.0.0" }],
          };
          if (params.widget === "log-tail") {
            if (params.inputs.mode === "list") return {
              ok: true,
              functions: ["one", "two"].map((name) => ({
                name: `synthetic-function-${name}`, log_group: `/synthetic/lambda/${name}`,
                arn: `arn:aws:lambda:eu-west-1:${context.account_id}:function:synthetic-${name}`,
                runtime: "synthetic", state: "Active",
              })),
            };
            if (params.inputs.mode === "streams") return { ok: true, streams: [{ name: "synthetic-stream", last_event_timestamp: 1700000000000 }] };
            return { render: "log_stream", events: [] };
          }
          return { render: "raw_json", data: {} };
        }
        default: throw new Error(`Unexpected synthetic invoke: ${command}`);
      }
    }
    function complete(call, response, metadata = {}) {
      const value = response ?? defaultResponse(call);
      if (call.command === "aws_set_account" && call.seq === latestSelection) {
        active = value.ok ? { ...call.context } : null;
      }
      if (!["widget_fetch", "aws_list_pipelines", "aws_set_account", "aws_auth_status"].includes(call.command)) return value;
      const verified = call.command === "aws_auth_status" ? value.has_context
        : call.command === "aws_set_account" ? value.ok : true;
      return {
        ...value,
        _request: {
          id: call.params.request_id ?? null,
          context_id: verified ? call.context.context_id : null,
          settings_revision: verified ? "1" : null,
          provider_revision: verified ? call.context.context_id : null,
          profile: verified ? call.context.profile : null,
          account_id: verified ? call.context.account_id : null,
          region: verified ? call.context.region : null,
          ...metadata,
        },
      };
    }
    window.__requestHarness = {
      calls, hold: (spec) => holds.push(spec),
      find: (spec) => calls.find((call) => pending.has(call.seq) && !call.claimed && matches(call, spec)),
      take: (spec) => {
        const call = window.__requestHarness.find(spec);
        if (!call) throw new Error("No matching deferred invocation");
        call.claimed = true;
        return structuredClone(call);
      },
      release: (seq, response, metadata) => {
        const item = pending.get(seq);
        if (!item) throw new Error("Invocation is not pending");
        pending.delete(seq);
        item.resolve(complete(item.call, response, metadata));
      },
      reject: (seq, message) => {
        const item = pending.get(seq);
        if (!item) throw new Error("Invocation is not pending");
        pending.delete(seq);
        item.reject(new Error(message));
      },
    };
    window.__TAURI__ = { core: { invoke(command, payload) {
      const params = structuredClone(payload?.params || {});
      const seq = ++sequence;
      let context;
      if (command === "aws_set_account") {
        latestSelection = seq;
        active = null;
        context = { profile: params.profile, account_id: params.account_id, region: params.region, context_id: String(seq) };
      } else if (params.context?.mode === "pinned") {
        context = { ...params.context, context_id: params.context.profile === A.profile ? "10001" : "10002" };
      } else {
        context = { ...(active || A), context_id: active?.context_id || "1" };
      }
      const call = {
        seq, command, params, context, claimed: false,
        authContext: command === "aws_auth_status" ? structuredClone(active) : null,
      };
      calls.push(call);
      if (holds.some((spec) => matches(call, spec))) {
        return new Promise((resolve, reject) => pending.set(seq, { call, resolve, reject }));
      }
      return Promise.resolve(complete(call));
    } } };
  }, { tiles, A, B, holdSelection });
  await page.goto("/");
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", holdSelection ? "checking" : "online");
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(tiles.length);
  await frame(page);
  return remoteRequests;
}

const hold = (page, spec) => page.evaluate((spec) => window.__requestHarness.hold(spec), spec);
async function next(page, spec) {
  await expect.poll(() => page.evaluate((spec) => !!window.__requestHarness.find(spec), spec)).toBe(true);
  return page.evaluate((spec) => window.__requestHarness.take(spec), spec);
}
// Drain the promise continuation and its DOM update, without an elapsed-time sleep.
const frame = (page) => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve())));
async function reply(page, call, response, metadata) {
  await page.evaluate(({ seq, response, metadata }) => window.__requestHarness.release(seq, response, metadata), { seq: call.seq, response, metadata });
  await frame(page);
}
async function reject(page, call, message) {
  await page.evaluate(({ seq, message }) => window.__requestHarness.reject(seq, message), { seq: call.seq, message });
  await frame(page);
}
async function chooseAccount(page, profile) {
  const input = page.getByRole("combobox", { name: "Default account" });
  await input.click();
  await input.fill(profile);
  await input.press("Enter");
  await expect(page.locator("#account-select")).toHaveValue(profile);
}
const refresh = (surface) => surface.locator('.widget-header button[title="Refresh"]').click();
const requestFor = (name, extra = {}) => ({ command: "widget_fetch", widget: name, ...extra });
const widgetCalls = (page, name) => page.evaluate((name) => window.__requestHarness.calls.filter(
  (call) => call.command === "widget_fetch" && call.params.widget === name
), name);

test("a saved live CLI command waits for verification and reloads under the next account", async ({ page }) => {
  const command = "aws sts get-caller-identity";
  await boot(page, [tile("aws-cli", { inputs: { command } })], { holdSelection: true });
  const surface = widget(page, "aws-cli");
  const initialSelection = await next(page, { command: "aws_set_account", profile: A.profile });
  expect(await widgetCalls(page, "aws-cli")).toEqual([]);
  await reply(page, initialSelection);
  await expect(surface.locator(".aws-cli-rows")).toContainText("initial-cli-demo-a");
  await chooseAccount(page, B.profile);
  const nextSelection = await next(page, { command: "aws_set_account", profile: B.profile });
  await expect(surface).not.toContainText("initial-cli-demo-a");
  expect((await widgetCalls(page, "aws-cli")).map((call) => call.context.profile)).not.toContain(B.profile);
  await reply(page, nextSelection);
  await expect(surface.locator(".aws-cli-rows")).toContainText("initial-cli-demo-b");
  await expect(surface.locator(".cli-command")).toHaveValue(command);
  const calls = await widgetCalls(page, "aws-cli");
  expect(calls.map((call) => call.context.profile)).toEqual([A.profile, B.profile]);
  for (const call of calls) {
    expect(call.params.inputs).toEqual({ command });
    expect(call.params.context.mode).toBe("inherit");
  }
});

test("CodeArtifact stays manual until first load then reloads loaded inputs after account changes", async ({ page }) => {
  await boot(page, [tile("codeartifact-packages")]);
  const surface = widget(page, "codeartifact-packages");
  expect(await widgetCalls(page, "codeartifact-packages")).toEqual([]);
  await chooseAccount(page, B.profile);
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  await frame(page);
  expect(await widgetCalls(page, "codeartifact-packages")).toEqual([]);
  await chooseAccount(page, A.profile);
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  await surface.locator(".codeartifact-domain").fill("synthetic-domain");
  await surface.locator(".codeartifact-repository").fill("synthetic-repository");
  await surface.locator(".codeartifact-prefix").fill("package");
  await surface.getByRole("button", { name: "Load or refresh packages" }).click();
  await expect(surface.locator(".codeartifact-packages-rows")).toContainText("package-demo-a");
  const first = (await widgetCalls(page, "codeartifact-packages"))[0];
  await hold(page, { command: "aws_set_account" });
  await chooseAccount(page, B.profile);
  const selection = await next(page, { command: "aws_set_account", profile: B.profile });
  await expect(surface).not.toContainText("package-demo-a");
  expect(await widgetCalls(page, "codeartifact-packages")).toHaveLength(1);
  await reply(page, selection);
  await expect(surface.locator(".codeartifact-packages-rows")).toContainText("package-demo-b");
  const calls = await widgetCalls(page, "codeartifact-packages");
  expect(calls).toHaveLength(2);
  expect(calls[1].params.inputs).toEqual(first.params.inputs);
  expect(calls[1].params.context.mode).toBe("inherit");
  expect(calls[1].context.profile).toBe(B.profile);
  expect(calls[1].params.request_id).not.toBe(first.params.request_id);
});

test("A to B to A discards the first A result despite equal identity labels", async ({ page }) => {
  const remote = await boot(page);
  const surface = widget(page, "cfn-stacks");
  await expect(surface).toContainText("initial-demo-a");
  await hold(page, requestFor("cfn-stacks"));
  await refresh(surface);
  const oldA = await next(page, requestFor("cfn-stacks", { profile: A.profile }));
  await chooseAccount(page, B.profile);
  const b = await next(page, requestFor("cfn-stacks", { profile: B.profile }));
  await chooseAccount(page, A.profile);
  const newA = await next(page, requestFor("cfn-stacks", { profile: A.profile }));
  expect(newA.params.request_id).not.toBe(oldA.params.request_id);
  await reply(page, newA, stacks("new-A-result"));
  await reply(page, oldA, stacks("obsolete-first-A"));
  await reject(page, b, "obsolete-B-error");
  await expect(surface).toContainText("new-A-result");
  await expect(surface).not.toContainText("obsolete");
  expect(remote).toEqual([]);
});

test("same-context refreshes accept the newest response and reject mismatched metadata", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks");
  await expect(surface).toContainText("initial-demo-a");
  await hold(page, requestFor("cfn-stacks"));
  await refresh(surface);
  const first = await next(page, requestFor("cfn-stacks"));
  await refresh(surface);
  const second = await next(page, requestFor("cfn-stacks"));
  await reply(page, second, stacks("newest-refresh"));
  await reply(page, first, stacks("obsolete-refresh"));
  await expect(surface).toContainText("newest-refresh");
  await expect(surface).not.toContainText("obsolete-refresh");
  for (const [metadata, error] of [
    [{ id: "wrong-request-id" }, "Response request identity did not match"],
    [{ account_id: B.account_id }, "Response AWS context did not match"],
    [{ context_id: null, provider_revision: null, settings_revision: null }, "Response verified context was missing"],
    [{ context_id: 1 }, "Response verified context was missing"],
  ]) {
    await refresh(surface);
    const call = await next(page, requestFor("cfn-stacks"));
    await reply(page, call, stacks("untrusted-response-data"), metadata);
    await expect(surface).toContainText(error);
    await expect(surface).not.toContainText("untrusted-response-data");
  }
});

test("verification clears inherited data immediately and a failed selection keeps it cleared", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks");
  await expect(surface).toContainText("initial-demo-a");
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.locator(".cfn-stacks-body > table > tbody > tr:not(.row-detail)").click();
  const previous = await next(page, requestFor("cfn-stack-detail"));
  await expect(surface).toContainText("initial-demo-a");
  await hold(page, { command: "aws_set_account" });
  await chooseAccount(page, B.profile);
  const selection = await next(page, { command: "aws_set_account", profile: B.profile });
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "checking");
  await expect(surface).not.toContainText("initial-demo-a");
  await reply(page, selection, { ok: false, error: "Synthetic verification failed" });
  await reply(page, previous, {
    render: "stack_detail", resources: [{ logical_id: "obsolete-before-failure" }], events: [],
  });
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "offline");
  await expect(surface).not.toContainText("obsolete-before-failure");
  const subsequentReads = await page.evaluate((seq) => window.__requestHarness.calls.filter((c) => c.seq > seq && c.command === "widget_fetch"), selection.seq);
  expect(subsequentReads).toEqual([]);
});

test("reconfiguring and removing a tile invalidates requests from its previous lifetime", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks");
  await expect(surface).toContainText("initial-demo-a");
  await hold(page, requestFor("cfn-stacks"));
  await refresh(surface);
  const previous = await next(page, requestFor("cfn-stacks"));
  await surface.locator(".cfg-btn").click();
  await page.locator("#cfg-use-override").check();
  await page.locator("#cfg-override-profile").selectOption(B.profile);
  await expect(page.locator("#cfg-override-account")).toHaveValue(B.account_id);
  await page.locator("#cfg-save").click();
  const reconfigured = await next(page, requestFor("cfn-stacks", { profile: B.profile }));
  expect(reconfigured.params.context).toEqual(pinned(B));
  await reply(page, reconfigured, stacks("pinned-B-result"));
  await reply(page, previous, stacks("obsolete-before-config"));
  await expect(surface).toContainText("pinned-B-result");
  await expect(surface).not.toContainText("obsolete-before-config");
  await refresh(surface);
  const removed = await next(page, requestFor("cfn-stacks"));
  await surface.locator(".rm-btn").click();
  await expect(surface).toHaveCount(0);
  await reply(page, removed, stacks("obsolete-removed-tile"));
  await expect(page.locator("body")).not.toContainText("obsolete-removed-tile");
});

test("nested execution and log reopenings ignore results for closed detail surfaces", async ({ page }) => {
  await boot(page, [tile("pipeline-runs")]);
  const surface = widget(page, "pipeline-runs");
  const input = surface.locator(".pipeline-name-search");
  await expect(input).toBeEnabled();
  await input.fill("synthetic-pipeline");
  await surface.locator(".pipeline-load-btn").click();
  const row = surface.locator(".pipeline-runs-rows > table > tbody > tr:not(.row-detail)");
  await expect(row).toHaveCount(1);
  await hold(page, requestFor("pipeline-execution-detail"));
  await row.click();
  const oldDetail = await next(page, requestFor("pipeline-execution-detail"));
  await row.click();
  await expect(surface.locator(".row-detail")).toHaveCount(0);
  await row.click();
  const newDetail = await next(page, requestFor("pipeline-execution-detail"));
  const detail = (action) => ({ render: "execution_detail", actions: [{
    stage: "Build", action, status: "Succeeded", category: "Build", provider: "CodeBuild",
    external_execution_id: "synthetic-build-id",
  }] });
  await reply(page, newDetail, detail("new-detail"));
  await reply(page, oldDetail, detail("obsolete-detail"));
  await expect(surface.locator(".row-detail")).toContainText("new-detail");
  await expect(surface).not.toContainText("obsolete-detail");
  await hold(page, requestFor("codebuild-log"));
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  const oldLog = await next(page, requestFor("codebuild-log"));
  await surface.getByRole("button", { name: "Hide log", exact: true }).click();
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  const newLog = await next(page, requestFor("codebuild-log"));
  await reply(page, newLog, events("new-build-log"));
  await reject(page, oldLog, "obsolete-build-log-error");
  await expect(surface.locator(".exec-log")).toContainText("new-build-log");
  await expect(surface).not.toContainText("obsolete-build-log-error");
});

for (const kind of ["pipeline-runs", "aws-cli"]) {
  test(`${kind} pin survives topbar failure while stale own refreshes and removed cards stay rejected`, async ({ page }) => {
    const isPipeline = kind === "pipeline-runs";
    const inputs = isPipeline
      ? { pinned_pipelines: [{ ...A, pipeline_name: "synthetic-pipeline" }] }
      : { pinned_cli_commands: [{ ...A, command: "aws sts get-caller-identity" }] };
    await boot(page, [tile(kind, { inputs })]);
    const surface = widget(page, kind);
    // Existing static pipeline markup is initially wired before layout restore;
    // activate the saved pin tab through the same control a user would click.
    await surface.locator(isPipeline ? '.pipeline-tab[data-tab="pinned"]' : '.cli-tab[data-tab="pinned"]').click();
    const card = surface.locator(".pipeline-pin-card");
    await expect(card).toHaveCount(1);
    await hold(page, requestFor(kind));
    await card.locator(".pipeline-pin-toggle").click();
    const first = await next(page, requestFor(kind));
    expect(first.params.context).toEqual(pinned());
    await hold(page, { command: "aws_set_account" });
    await chooseAccount(page, B.profile);
    const selection = await next(page, { command: "aws_set_account" });
    await reply(page, first, isPipeline ? runs("pinned-during-verification") : cliRows("pinned-during-verification"));
    await expect(card).toContainText("pinned-during-verification");
    await reply(page, selection, { ok: false, error: "Synthetic topbar failure" });
    await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "offline");
    await expect(card).toContainText("pinned-during-verification");
    const ownRefresh = card.locator(isPipeline ? ".pipeline-pin-refresh" : ".cli-pin-refresh");
    await ownRefresh.click();
    const oldRefresh = await next(page, requestFor(kind));
    await ownRefresh.click();
    const newRefresh = await next(page, requestFor(kind));
    expect(newRefresh.params.context).toEqual(pinned());
    await reply(page, newRefresh, isPipeline ? runs("new-pin-result") : cliRows("new-pin-result"));
    await reject(page, oldRefresh, "obsolete-pin-error");
    await expect(card).toContainText("new-pin-result");
    await expect(card).not.toContainText("obsolete-pin-error");
    await expect(card.locator(".pipeline-pin-status")).not.toHaveText(/Loading|Running|Error/);
    await ownRefresh.click();
    const removed = await next(page, requestFor(kind));
    await card.locator(isPipeline ? ".pipeline-pin-remove" : ".cli-pin-remove").click();
    await reply(page, removed, isPipeline ? runs("removed-pin-result") : cliRows("removed-pin-result"));
    await expect(card).toHaveCount(0);
    await expect(surface).not.toContainText("removed-pin-result");
  });
}

test("pinned Lambda logs survive topbar changes and reject streams or events for an old function", async ({ page }) => {
  await boot(page, [tile("log-tail", { context: pinned() })]);
  const surface = widget(page, "log-tail");
  await expect(surface.locator(".lambda-row")).toHaveCount(2);
  await hold(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  await surface.locator(".lambda-row").filter({ hasText: "synthetic-function-one" }).click();
  const firstStreams = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  await surface.locator(".lambda-row").filter({ hasText: "synthetic-function-two" }).click();
  const secondStreams = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  await chooseAccount(page, B.profile);
  await reply(page, secondStreams, { ok: true, streams: [{ name: "current-function-stream" }] });
  await reply(page, firstStreams, { ok: true, streams: [{ name: "obsolete-function-stream" }] });
  await expect(surface.locator(".lambda-stream-select")).toHaveValue("current-function-stream");
  await expect(surface).not.toContainText("obsolete-function-stream");
  expect(secondStreams.params.context).toEqual(pinned());
  await hold(page, requestFor("log-tail", { inputs: { mode: "events" } }));
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  const oldEvents = await next(page, requestFor("log-tail", { inputs: { mode: "events" } }));
  await surface.locator(".lambda-row").filter({ hasText: "synthetic-function-one" }).click();
  const currentStreams = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  await reply(page, currentStreams, { ok: true, streams: [{ name: "reopened-function-stream" }] });
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  const newEvents = await next(page, requestFor("log-tail", { inputs: { mode: "events" } }));
  await reply(page, newEvents, events("current-function-events"));
  await reply(page, oldEvents, events("obsolete-function-events"));
  await expect(surface.locator(".lambda-log-events")).toContainText("current-function-events");
  await expect(surface).not.toContainText("obsolete-function-events");
});

test("a late pipeline list cannot replace the current account selector options", async ({ page }) => {
  await boot(page, [tile("pipeline-runs")]);
  const surface = widget(page, "pipeline-runs");
  const input = surface.locator(".pipeline-name-search");
  await expect(input).toBeEnabled();
  await hold(page, { command: "aws_list_pipelines" });
  await chooseAccount(page, B.profile);
  const b = await next(page, { command: "aws_list_pipelines", profile: B.profile });
  await chooseAccount(page, A.profile);
  const a = await next(page, { command: "aws_list_pipelines", profile: A.profile });
  await reply(page, a, pipelines("current-A-pipeline"));
  await reply(page, b, pipelines("obsolete-B-pipeline"));
  await expect(input).toBeEnabled();
  await expect(surface.locator(".pipeline-name-select option")).toHaveText(["current-A-pipeline"]);
  await input.click();
  await input.fill("obsolete-B");
  await expect(page.getByRole("option", { name: "obsolete-B-pipeline", exact: true })).toHaveCount(0);
});
