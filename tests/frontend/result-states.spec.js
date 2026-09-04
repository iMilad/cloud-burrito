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
        case "request_cancel": if (typeof params.request_id !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(params.request_id)) throw new Error("Invalid synthetic cancellation ID"); return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
        case "ping": return { ok: true, version: "synthetic-test" };
        case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
        case "settings_get": return settingsResponse("loaded");
        case "dashboard_get": return { tiles };
        case "dashboard_set": return { ok: true };
        case "settings_set":
          settings = Object.fromEntries(Object.entries(settingsDefaults).map(([name, fallback]) => [name, params[name]?.trim() || fallback]));
          return settingsResponse("saved");
        case "aws_list_profiles": return {
          config_path: "/synthetic/aws/config", file_exists: true, error: null, discovery_state: "ready",
          profiles: [A, B].map((identity) => ({
            name: identity.profile, account_id: identity.account_id, region: identity.region,
            role_name: "SyntheticReadOnly", sso_session: "synthetic-session",
            eligibility: "supported_sso", eligibility_reason: "Synthetic supported SSO profile",
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
          if (params.widget === "cloudwatch-logs" || (params.widget === "logs-insights" && params.inputs.mode === "groups")) {
            if (params.inputs.mode === "groups") return { ok: true, groups: [{ name: "/synthetic/group", arn: "synthetic-group-arn" }] };
            if (params.inputs.mode === "streams") return { ok: true, streams: [{ name: "synthetic-stream" }] };
            return { render: "log_stream", events: [] };
          }
          if (params.widget === "errors-by-stack") return { render: "errors_chart", rows: [{ stack: "synthetic-stack", errors: 2 }] };
          if (params.widget === "logs-insights") return table("message", "synthetic-query-row");
          if (params.widget === "log-tail") {
            if (params.inputs.mode === "list") return {
              ok: true,
              functions: ["one", "two"].map((name) => ({
                name: `synthetic-function-${name}`, log_group: `/synthetic/lambda/${name}`,
                handoffs: { logs: { status: "available", source: "lambda_logging_config", widget: "log-tail",
                  inputs: { mode: "streams", log_group: `/synthetic/lambda/${name}` }, reason: "Log group from Lambda logging configuration; existence is unverified." } },
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
          outcome: value.ok === false || value.error || value.data?.error ? "failed" : "succeeded",
          ...metadata,
        },
      };
    }
    window.__resultHarness = {
      calls, hold: (spec) => holds.push(spec),
      find: (spec) => calls.find((call) => pending.has(call.seq) && !call.claimed && matches(call, spec)),
      take: (spec) => {
        const call = window.__resultHarness.find(spec);
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

const hold = (page, spec) => page.evaluate((spec) => window.__resultHarness.hold(spec), spec);
async function next(page, spec) {
  await expect.poll(() => page.evaluate((spec) => !!window.__resultHarness.find(spec), spec)).toBe(true);
  return page.evaluate((spec) => window.__resultHarness.take(spec), spec);
}
// Drain the promise continuation and its DOM update, without an elapsed-time sleep.
const frame = (page) => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve())));
async function reply(page, call, response, metadata) {
  await page.evaluate(({ seq, response, metadata }) => window.__resultHarness.release(seq, response, metadata), { seq: call.seq, response, metadata });
  await frame(page);
}
async function reject(page, call, message) {
  await page.evaluate(({ seq, message }) => window.__resultHarness.reject(seq, message), { seq: call.seq, message });
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
const widgetCalls = (page, name) => page.evaluate((name) => window.__resultHarness.calls.filter(
  (call) => call.command === "widget_fetch" && call.params.widget === name
), name);


const coverage = (completeness = "complete", returned = 1, extra = {}) => ({
  completeness, has_more: completeness === "limited" ? true : completeness === "complete" ? false : null,
  counts: { returned, pages: 1 }, limits: { results: 10 }, reasons: [], ...extra,
});
const partial = (model, returned = 1) => ({ ...model, ok: false, partial: true, error_type: "PartialFailure",
  error: "A synthetic page failed. Returned evidence is retained.", coverage: coverage("unknown", returned) });
const status = host => host.locator(":scope > .result-status");
const received = host => status(host).locator("time").getAttribute("datetime");
async function refreshReply(page, surface, name, response, metadata) {
  await refresh(surface);
  const call = await next(page, requestFor(name));
  await reply(page, call, response, metadata);
}

test("limited evidence retains its receipt, original limits and context through loading, denied and cancelled refreshes", async ({ page }) => {
  const remote = await boot(page);
  const surface = widget(page, "cfn-stacks"), host = surface.locator(".cfn-stacks-body");
  await hold(page, requestFor("cfn-stacks"));
  await refreshReply(page, surface, "cfn-stacks", { ...stacks("bounded-evidence"), truncated: true,
    coverage: coverage("limited", 1, { reasons: [{ code: "next_page", message: "Additional stacks were not loaded." }] }) });
  await expect(status(host)).toHaveAttribute("data-state", "limited");
  const original = await received(host);
  await refresh(surface);
  const denied = await next(page, requestFor("cfn-stacks"));
  await expect(status(host)).toHaveAttribute("data-state", "loading");
  await expect(host).toContainText("bounded-evidence");
  await expect(status(host)).toContainText("Retained coverage: limited");
  expect(await received(host)).toBe(original);
  await reply(page, denied, { render: "permission_denied", action: "cloudformation:ListStacks", reason: "Synthetic policy denied." }, { outcome: "denied" });
  await expect(status(host)).toHaveAttribute("data-state", "stale");
  await expect(status(host)).toContainText("Refresh outcome: Denied");
  await expect(status(host)).toContainText("Results limit: 10");
  expect(await received(host)).toBe(original);
  await refreshReply(page, surface, "cfn-stacks", { render: "raw_json", data: { error: "Synthetic cancellation", error_type: "QueryCancelled" } }, { outcome: "cancelled" });
  await expect(status(host)).toContainText("Refresh outcome: Cancelled");
  await expect(status(host)).toContainText("does not confirm that remote work stopped");
  expect(await received(host)).toBe(original);
  expect(remote).toEqual([]);
});

test("partial tables keep returned rows, failed refreshes retain them, and a successful empty response replaces them", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks"), host = surface.locator(".cfn-stacks-body");
  await hold(page, requestFor("cfn-stacks"));
  await refreshReply(page, surface, "cfn-stacks", partial(stacks("partial-page-row")));
  await expect(status(host)).toHaveAttribute("data-state", "partial");
  await expect(host.locator(":scope > table > tbody > tr")).toHaveCount(1);
  const original = await received(host);
  await refreshReply(page, surface, "cfn-stacks", { render: "table", columns: ["stack"], rows: [], ok: false, error: "Synthetic page failure", coverage: coverage("unknown", 0) });
  await expect(status(host)).toHaveAttribute("data-state", "stale");
  await expect(host).toContainText("partial-page-row");
  expect(await received(host)).toBe(original);
  await refreshReply(page, surface, "cfn-stacks", { render: "table", columns: ["stack"], rows: [], coverage: coverage("complete", 0) });
  await expect(status(host)).toHaveAttribute("data-state", "empty");
  await expect(host).not.toContainText("partial-page-row");
  await expect(status(host)).toContainText("Coverage: complete");
});

test("malformed data, coverage and mismatched request context cannot replace accepted evidence", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks"), host = surface.locator(".cfn-stacks-body");
  await expect(host).toContainText("initial-demo-a");
  const original = await received(host);
  await hold(page, requestFor("cfn-stacks"));
  await refreshReply(page, surface, "cfn-stacks", { ...stacks("untrusted-metadata-row"), coverage: { completeness: "complete", has_more: false, counts: { returned: -1 }, limits: {}, reasons: [] } });
  await expect(status(host)).toContainText("coverage metadata was invalid");
  await expect(host).not.toContainText("untrusted-metadata-row");
  await refreshReply(page, surface, "cfn-stacks", { render: "table", columns: ["stack"], rows: "not-an-array" });
  await expect(status(host)).toContainText("data shape was invalid");
  await refreshReply(page, surface, "cfn-stacks", stacks("wrong-context-row"), { region: "us-east-1" });
  await expect(host).not.toContainText("wrong-context-row");
  await expect(host).toContainText("initial-demo-a");
  expect(await received(host)).toBe(original);
});

test("selection changes clear old evidence while the new context loads, including late failures", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks"), host = surface.locator(".cfn-stacks-body");
  await expect(host).toContainText("initial-demo-a");
  await hold(page, requestFor("cfn-stacks"));
  await refresh(surface);
  const old = await next(page, requestFor("cfn-stacks"));
  await chooseAccount(page, B.profile);
  const current = await next(page, requestFor("cfn-stacks", { profile: B.profile }));
  await expect(host).not.toContainText("initial-demo-a");
  await reject(page, old, "Obsolete synthetic failure");
  await reply(page, current, { render: "table", columns: ["stack"], rows: [], ok: false, error: "New-context failure", coverage: coverage("unknown", 0) });
  await expect(status(host)).toHaveAttribute("data-state", "failed");
  await expect(host).not.toContainText("initial-demo-a");
  await expect(host).not.toContainText("Obsolete");
});

test("stack details retain successful resources and label denied event sections without an empty success claim", async ({ page }) => {
  await boot(page);
  const surface = widget(page, "cfn-stacks");
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.locator(".cfn-stacks-body > table > tbody > tr").click();
  const call = await next(page, requestFor("cfn-stack-detail"));
  await reply(page, call, { ...partial({ render: "stack_detail", resources: [{ logical_id: "retained-resource", type: "AWS::Lambda::Function", status: "CREATE_COMPLETE", physical_id: "synthetic-resource" }], events: [] }),
    coverage: coverage("unknown", 1, { sections: { resources: coverage("complete", 1), events: coverage("unknown", 0, { reasons: [{ code: "policy_denied", message: "Stack events were denied." }] }) } }) });
  const detail = surface.locator(".row-detail > td");
  await expect(detail).toContainText("retained-resource");
  await expect(status(detail)).toHaveAttribute("data-state", "partial");
  await detail.getByRole("tab", { name: "Events" }).click();
  await expect(detail).toContainText("Events were denied");
  await expect(detail).not.toContainText("No recent stack events returned");
});

for (const kind of ["log-tail", "cloudwatch-logs"]) {
  test(`${kind} list, streams and events retain partial arrays with coverage and original refresh receipt`, async ({ page }) => {
    await boot(page, [tile(kind)]);
    const surface = widget(page, kind), list = surface.locator(".lambda-list");
    await expect(list.locator(".lambda-row").first()).toBeVisible();
    const mode = kind === "log-tail" ? "list" : "groups";
    const key = kind === "log-tail" ? "functions" : "groups";
    const item = kind === "log-tail" ? { name: "partial-function", log_group: "/synthetic/group",
      handoffs: { logs: { status: "available", source: "lambda_logging_config", widget: "log-tail",
        inputs: { mode: "streams", log_group: "/synthetic/group" }, reason: "Log group from Lambda logging configuration; existence is unverified." } } } : { name: "/synthetic/group" };
    await hold(page, requestFor(kind));
    await refreshReply(page, surface, kind, partial({ [key]: [item] }));
    await expect(status(list)).toHaveAttribute("data-state", "partial");
    await expect(list.locator(".lambda-row")).toHaveCount(1);
    const original = await received(list);
    await surface.getByRole("button", { name: "Reload", exact: true }).click();
    await reply(page, await next(page, requestFor(kind, { inputs: { mode } })), { ok: false, error: "Synthetic list failure" });
    await expect(status(list)).toHaveAttribute("data-state", "stale");
    expect(await received(list)).toBe(original);
    await list.locator(".lambda-row").click();
    if (kind === "log-tail") {
      await expect(surface.getByRole("button", { name: "Load streams", exact: true })).toBeDisabled();
      await expect(surface).toContainText("Refresh the function list before opening linked logs.");
      await surface.getByRole("button", { name: "Reload", exact: true }).click();
      await reply(page, await next(page, requestFor(kind, { inputs: { mode } })), partial({ [key]: [item] }));
      await list.locator(".lambda-row").click();
    }
    await reply(page, await next(page, requestFor(kind, { inputs: { mode: "streams" } })), partial({ streams: [{ name: "partial-stream" }] }));
    const streams = surface.locator(".lambda-stream-panel");
    await expect(status(streams)).toHaveAttribute("data-state", "partial");
    await expect(streams.locator("select")).toHaveValue("partial-stream");
    await surface.getByRole("button", { name: "View log", exact: true }).click();
    await reply(page, await next(page, requestFor(kind, { inputs: { mode: "events" } })), partial(events("retained-log-event")));
    const logs = surface.locator(".lambda-log-events");
    await expect(status(logs)).toHaveAttribute("data-state", "partial");
    await expect(logs).toContainText("retained-log-event");
    await expect(status(list)).toHaveAttribute("data-state", kind === "log-tail" ? "partial" : "stale");
  });
}

test("failed and bounded error counts never claim zero errors or exact lower bounds", async ({ page }) => {
  await boot(page, [tile("errors-by-stack")]);
  const surface = widget(page, "errors-by-stack"), host = surface.locator(".errors-body");
  await hold(page, requestFor("errors-by-stack"));
  await refreshReply(page, surface, "errors-by-stack", partial({ render: "errors_chart", rows: [{ stack: "bounded-stack", errors: 4, count_is_lower_bound: true }] }));
  await expect(host.locator(".bar-count")).toHaveText("≥ 4");
  await refreshReply(page, surface, "errors-by-stack", { render: "errors_chart", rows: [], status: "failed", ok: false, error: "Synthetic query failure", coverage: coverage("unknown", 0), cleanup: { status: "not_attempted", remote_queries_may_still_run: true } });
  await expect(status(host)).toHaveAttribute("data-state", "stale");
  await expect(host).not.toContainText("No error events");
  await expect(host).toContainText("Remote stop was not attempted");
  await expect(host).toContainText("Remote queries may still be running");
});

test("query autocomplete keeps partial suggestions; timeout cleanup is distinct from logical cancellation", async ({ page }) => {
  await boot(page, [tile("logs-insights")]);
  const surface = widget(page, "logs-insights");
  await hold(page, requestFor("logs-insights"));
  await surface.locator(".li-group").focus();
  await reply(page, await next(page, requestFor("logs-insights", { inputs: { mode: "groups" } })), partial({ groups: [{ name: "/partial/group" }] }));
  await expect(surface.locator("datalist option")).toHaveAttribute("value", "/partial/group");
  await expect(surface.locator(".result-input-status > .result-status")).toHaveAttribute("data-state", "partial");
  await surface.locator(".li-group").fill("/partial/group");
  await surface.getByRole("button", { name: "Run query", exact: true }).click();
  await reply(page, await next(page, requestFor("logs-insights", { inputs: { mode: "query" } })), { render: "raw_json", data: { error: "Synthetic query timeout", error_type: "QueryTimeout", cleanup: { status: "stopped", remote_stop_confirmed: true } }, coverage: coverage("unknown", 0) });
  const host = surface.locator(".logs-insights-rows");
  await expect(status(host)).toHaveAttribute("data-state", "failed");
  await expect(status(host)).toContainText("Remote query stop confirmed");
  await expect(host).not.toContainText("0 rows");
  await surface.getByRole("button", { name: "Run query", exact: true }).click();
  await reply(page, await next(page, requestFor("logs-insights", { inputs: { mode: "query" } })), { render: "raw_json", data: { error: "Synthetic query timeout", error_type: "QueryTimeout", cleanup: { status: "stopped", remote_stop_confirmed: false } } });
  await expect(status(host)).toContainText("Remote stop was not confirmed");
  await expect(status(host)).not.toContainText("Remote query stop confirmed");
});

test("CodeArtifact package and history partial data remain visible with separate coverage", async ({ page }) => {
  await boot(page, [tile("codeartifact-packages")]);
  const surface = widget(page, "codeartifact-packages");
  await surface.locator(".codeartifact-domain").fill("synthetic-domain");
  await surface.locator(".codeartifact-repository").fill("synthetic-repository");
  await surface.locator(".codeartifact-prefix").fill("synthetic");
  await hold(page, requestFor("codeartifact-packages"));
  await refreshReply(page, surface, "codeartifact-packages", partial({ render: "table", columns: ["package", "latest_version"], rows: [{ package: "synthetic-package", latest_version: "2.0.0" }] }));
  const host = surface.locator(".codeartifact-packages-rows");
  await expect(status(host)).toHaveAttribute("data-state", "partial");
  await hold(page, requestFor("codeartifact-package-version-history"));
  await host.locator(":scope > table > tbody > tr").click();
  await reply(page, await next(page, requestFor("codeartifact-package-version-history")), partial({ render: "codeartifact_version_history", versions: [{ version: "1.0.0", published: "2026-01-01T00:00:00Z" }] }));
  const detail = host.locator(".row-detail > td");
  await expect(status(detail)).toHaveAttribute("data-state", "partial");
  await expect(detail).toContainText("1.0.0");
});

for (const kind of ["pipeline-runs", "aws-cli"]) {
  test(`${kind} pinned partial evidence keeps its context and timestamp after a failed refresh`, async ({ page }) => {
    const isPipeline = kind === "pipeline-runs";
    const command = "aws cloudformation list-stacks";
    const inputs = isPipeline ? { pinned_pipelines: [{ ...A, pipeline_name: "synthetic-pipeline" }] }
      : { pinned_cli_commands: [{ ...A, command }] };
    await boot(page, [tile(kind, { inputs })]);
    const surface = widget(page, kind);
    await surface.locator(isPipeline ? '.pipeline-tab[data-tab="pinned"]' : '.cli-tab[data-tab="pinned"]').click();
    const card = surface.locator(".pipeline-pin-card"), host = card.locator(".pipeline-pin-result");
    await card.locator(".pipeline-pin-toggle").click();
    await expect(host.locator("table")).toBeVisible();
    await hold(page, requestFor(kind));
    const button = card.locator(isPipeline ? ".pipeline-pin-refresh" : ".cli-pin-refresh");
    await button.click();
    await reply(page, await next(page, requestFor(kind)), partial(isPipeline ? runs("pinned-partial-evidence") : cliRows("pinned-partial-evidence")));
    await expect(status(host)).toHaveAttribute("data-state", "partial");
    await expect(card.locator(".pipeline-pin-status")).toHaveText("Partial");
    const original = await received(host);
    await button.click();
    await reply(page, await next(page, requestFor(kind)), { render: "raw_json", data: { error: "Synthetic failure" } });
    await expect(status(host)).toHaveAttribute("data-state", "stale");
    await expect(host).toContainText("pinned-partial-evidence");
    expect(await received(host)).toBe(original);
  });
}

test("pipeline names preserve capped and partial choices across failed retries, then clear them for a new account", async ({ page }) => {
  await boot(page, [tile("pipeline-runs")]);
  const surface = widget(page, "pipeline-runs"), input = surface.locator(".pipeline-name-search");
  await expect(input).toBeEnabled();
  await hold(page, { command: "aws_list_pipelines" });
  const retry = surface.getByRole("button", { name: "Retry pipeline list" });
  await retry.click();
  await reply(page, await next(page, { command: "aws_list_pipelines" }), { ok: true, pipelines: [{ name: "capped-pipeline" }], truncated: true, coverage: coverage("limited", 1) });
  const host = surface.locator(".pipeline-name-search + .result-input-status");
  await expect(status(host)).toHaveAttribute("data-state", "limited");
  await retry.click();
  await reply(page, await next(page, { command: "aws_list_pipelines" }), partial({ pipelines: [{ name: "partial-pipeline" }] }));
  await expect(status(host)).toHaveAttribute("data-state", "partial");
  const original = await received(host);
  await retry.click();
  await reply(page, await next(page, { command: "aws_list_pipelines" }), { ok: false, pipelines: [], error: "Synthetic first-page failure", coverage: coverage("unknown", 0) });
  await expect(status(host)).toHaveAttribute("data-state", "stale");
  expect(await received(host)).toBe(original);
  await input.fill("partial-pipeline");
  await page.locator(".combo-item").filter({ hasText: "partial-pipeline" }).click();
  await expect(surface.locator(".pipeline-name-select")).toHaveValue("partial-pipeline");
  await chooseAccount(page, B.profile);
  const pending = await next(page, { command: "aws_list_pipelines", profile: B.profile });
  await expect(input).toBeDisabled();
  await expect(surface.locator(".pipeline-name-select option")).toHaveCount(0);
  await expect(surface).not.toContainText("Retained coverage");
  await reply(page, pending, { ok: false, pipelines: [], error: "Synthetic new-context failure", coverage: coverage("unknown", 0) });
  await expect(status(host)).toHaveAttribute("data-state", "failed");
  await expect(input).not.toHaveAttribute("placeholder", /no CodePipeline pipelines/);
});
