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
        case "audit_history": if (params.action !== "status") throw new Error("Unexpected audit history action"); return { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." };
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
    window.__investigationHarness = {
      calls, hold: (spec) => holds.push(spec),
      find: (spec) => calls.find((call) => pending.has(call.seq) && !call.claimed && matches(call, spec)),
      take: (spec) => {
        const call = window.__investigationHarness.find(spec);
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

const hold = (page, spec) => page.evaluate((spec) => window.__investigationHarness.hold(spec), spec);
async function next(page, spec) {
  await expect.poll(() => page.evaluate((spec) => !!window.__investigationHarness.find(spec), spec)).toBe(true);
  return page.evaluate((spec) => window.__investigationHarness.take(spec), spec);
}
// Drain the promise continuation and its DOM update, without an elapsed-time sleep.
const frame = (page) => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve())));
async function reply(page, call, response, metadata) {
  await page.evaluate(({ seq, response, metadata }) => window.__investigationHarness.release(seq, response, metadata), { seq: call.seq, response, metadata });
  await frame(page);
}
async function reject(page, call, message) {
  await page.evaluate(({ seq, message }) => window.__investigationHarness.reject(seq, message), { seq: call.seq, message });
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
const widgetCalls = (page, name) => page.evaluate((name) => window.__investigationHarness.calls.filter(
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

const link = (status, source, widget, inputs, reason) => ({ status, source, widget, inputs, reason });
const buildLink = id => link("available", "pipeline_build_execution", "codebuild-log", { build_id: id }, "AWS returned this build execution identifier for this pipeline action.");
const configuredStack = name => link("available", "pipeline_stack_configuration", "cfn-stack-detail", { stack_name: name }, "Configured stack target for this pipeline action; ownership and existence are not established.");
const stackUnknown = () => link("manual", "relationship_unknown", "resource-lookup", {}, "No stack relationship was returned. Search in this context and choose a candidate explicitly.");
const exactGroup = group => link("available", "cfn_log_group", "log-tail", { mode: "streams", log_group: group }, "CloudFormation returned this physical identifier for a log group resource.");
const unavailable = source => link("unavailable", source, null, {}, "The returned evidence does not establish this relationship.");
const detail = (suffix = "one") => ({ render: "execution_detail", coverage: coverage(), actions: [
  { stage: "Build", action: `failed-build-${suffix}`, status: "Failed", category: "Build", provider: "CodeBuild",
    external_execution_id: `build-${suffix}`, error: `Build failure ${suffix}`, handoffs: { build: buildLink(`build-${suffix}`) } },
  { stage: "Deploy", action: `deploy-${suffix}`, status: "Failed", category: "Deploy", provider: "CloudFormation",
    handoffs: { stack: configuredStack(`stack-${suffix}`) } },
] });
const stackDetail = (suffix = "one") => ({ render: "stack_detail", coverage: coverage(),
  resources: [{ logical_id: `Logs-${suffix}`, type: "AWS::Logs::LogGroup", status: "CREATE_COMPLETE", physical_id: `/synthetic/${suffix}`,
    handoffs: { logs: exactGroup(`/synthetic/${suffix}`) } }],
  events: [{ logical_id: `Deploy-${suffix}`, status: "CREATE_FAILED", reason: `Synthetic deployment failure ${suffix}` }],
});
async function pipeline(page, { config = {}, suffix = "one" } = {}) {
  const remote = await boot(page, [tile("pipeline-runs", config)]);
  const surface = widget(page, "pipeline-runs");
  await surface.locator(".pipeline-name-search").fill("synthetic-pipeline");
  await surface.locator(".pipeline-load-btn").click();
  await expect(surface.locator(".pipeline-runs-rows > table > tbody > tr:not(.row-detail)")).toHaveCount(1);
  await hold(page, requestFor("pipeline-execution-detail"));
  await surface.locator(".pipeline-runs-rows > table > tbody > tr:not(.row-detail)").click();
  const call = await next(page, requestFor("pipeline-execution-detail"));
  await reply(page, call, detail(suffix));
  return { surface, remote, call };
}

test("failed execution reaches build error, configured stack event and exact log evidence in one source context", async ({ page }) => {
  const { surface, remote } = await pipeline(page);
  expect(await widgetCalls(page, "codebuild-log")).toHaveLength(0);
  expect(await widgetCalls(page, "cfn-stack-detail")).toHaveLength(0);
  await hold(page, requestFor("codebuild-log"));
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  const build = await next(page, requestFor("codebuild-log"));
  expect(build.params.inputs).toEqual({ build_id: "build-one" });
  await reply(page, build, { ...partial(events("Compiler failed: synthetic build evidence")), handoffs: { stack: stackUnknown() } });
  await expect(surface).toContainText("Compiler failed: synthetic build evidence");
  await expect(surface).toContainText("No stack relationship was returned");
  await expect(surface.locator('.exec-log .result-status')).toHaveAttribute("data-state", "partial");
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Inspect configured stack" }).click();
  const stack = await next(page, requestFor("cfn-stack-detail"));
  expect(stack.params.inputs).toEqual({ stack_name: "stack-one" });
  await reply(page, stack, stackDetail());
  await expect(surface).toContainText("ownership and existence are not established");
  await surface.getByRole("tab", { name: "Events", exact: false }).click();
  await expect(surface).toContainText("Synthetic deployment failure one");
  await surface.getByRole("tab", { name: "Resources", exact: false }).click();
  await hold(page, requestFor("log-tail"));
  await surface.getByRole("button", { name: "Browse log streams" }).click();
  const streams = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  expect(streams.params.inputs).toEqual({ mode: "streams", log_group: "/synthetic/one", max_streams: 50 });
  await reply(page, streams, partial({ streams: [{ name: "exact-stream" }] }));
  await surface.getByRole("button", { name: "View linked log" }).click();
  const logs = await next(page, requestFor("log-tail", { inputs: { mode: "events" } }));
  expect(logs.params.inputs).toEqual({ mode: "events", log_group: "/synthetic/one", log_stream: "exact-stream", limit: 500 });
  await reply(page, logs, { ...events("Expected synthetic application error"), coverage: coverage("limited") });
  await expect(surface.locator(".evidence-log-output")).toContainText("Expected synthetic application error");
  await expect(surface.locator(".evidence-log-output > .result-status")).toHaveAttribute("data-state", "limited");
  for (const call of [build, stack, streams, logs]) expect(call.params.context).toEqual(pinned(A));
  expect(remote).toEqual([]);
});

test("manual lookup uses visible query, rejects prior replies and labels a typed stack as user selection", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("codebuild-log"));
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  await reply(page, await next(page, requestFor("codebuild-log")), { ...events("Build retained"), handoffs: { stack: stackUnknown() } });
  await surface.getByRole("button", { name: "Find a stack manually" }).click();
  expect(await widgetCalls(page, "resource-lookup")).toHaveLength(0);
  const input = surface.getByRole("searchbox", { name: "Resource query in source context" });
  await hold(page, requestFor("resource-lookup"));
  await input.fill("old-visible-query");
  await surface.getByRole("button", { name: "Search resources", exact: true }).click();
  const old = await next(page, requestFor("resource-lookup"));
  await input.fill("current-visible-query");
  await surface.getByRole("button", { name: "Search resources", exact: true }).click();
  const current = await next(page, requestFor("resource-lookup"));
  expect(current.params.inputs).toEqual({ query: "current-visible-query" });
  expect(current.params.context).toEqual(pinned(A));
  await reply(page, current, { render: "reverse_lookup", matches: [{ arn: "synthetic-current-resource", stack: null,
    handoffs: { stack: unavailable("ownership_ambiguous") } }], coverage: coverage() });
  await reply(page, old, { render: "reverse_lookup", matches: [{ arn: "obsolete-resource", stack: "unproven-stack" }] });
  await expect(surface).toContainText("Stack ownership is unknown");
  await expect(surface).not.toContainText("obsolete-resource");
  await expect(surface).not.toContainText("Owned by");
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("textbox", { name: "Stack name or ARN for manual inspection" }).fill("chosen-stack");
  await surface.getByRole("button", { name: "Inspect chosen stack" }).click();
  const chosen = await next(page, requestFor("cfn-stack-detail"));
  expect(chosen.params.inputs).toEqual({ stack_name: "chosen-stack" });
  expect(chosen.params.context).toEqual(pinned(A));
  await reply(page, chosen, stackDetail("chosen"));
  await expect(surface).toContainText("Your selection does not establish a relationship to the source build");
  await expect(surface.locator(".evidence-manual-stack")).toContainText("Logs-chosen");
});

test("denied and unknown build associations stay honest while confirmed lookup association can be inspected", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("codebuild-log"));
  await surface.getByRole("button", { name: "View log", exact: true }).click();
  await reply(page, await next(page, requestFor("codebuild-log")), {
    render: "permission_denied", action: "logs:GetLogEvents", reason: "Synthetic log read denied", handoffs: { stack: stackUnknown() },
  }, { outcome: "denied" });
  await expect(surface.locator(".exec-log > .evidence-content > .result-status")).toHaveAttribute("data-state", "denied");
  await surface.getByRole("button", { name: "Find a stack manually" }).click();
  await hold(page, requestFor("resource-lookup"));
  await surface.getByRole("searchbox", { name: "Resource query in source context" }).fill("exact-resource");
  await surface.getByRole("button", { name: "Search resources", exact: true }).click();
  await reply(page, await next(page, requestFor("resource-lookup")), { render: "reverse_lookup", matches: [{ arn: "synthetic-exact-resource",
    type: "AWS::Lambda::Function", handoffs: { stack: link("available", "cfn_ownership", "cfn-stack-detail", { stack_name: "exact-stack" }, "CloudFormation matched the exact resource identifier to this stack in the current context.") } }], coverage: coverage() });
  await expect(surface).toContainText("Matched stack: exact-stack");
  await expect(surface).toContainText("does not establish a relationship to this build");
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Open matched stack" }).click();
  const stack = await next(page, requestFor("cfn-stack-detail"));
  expect(stack.params.inputs).toEqual({ stack_name: "exact-stack" });
  await reply(page, stack, { ...partial(stackDetail()), coverage: coverage("unknown", 1, {
    sections: { resources: coverage(), events: coverage("unknown", 0, { reasons: [{ code: "policy_denied", message: "Stack events were denied." }] }) } }), events: [] });
  await surface.getByRole("tab", { name: "Events", exact: false }).click();
  await expect(surface).toContainText("Events were denied; no data is available for this section.");
});

test("retained source disables linked navigation during refresh and after failure; a context change rejects detached clicks", async ({ page }) => {
  const { surface } = await pipeline(page);
  const linkButton = surface.getByRole("button", { name: "View log", exact: true });
  await linkButton.evaluate(button => { window.__detachedEvidenceButton = button; });
  await hold(page, requestFor("pipeline-runs"));
  await refresh(surface);
  const refreshCall = await next(page, requestFor("pipeline-runs"));
  await expect(linkButton).toBeDisabled();
  await expect(surface).toContainText("Refresh the source before opening linked evidence.");
  await reply(page, refreshCall, { ok: false, error: "Synthetic refresh failed" });
  await expect(linkButton).toBeDisabled();
  await chooseAccount(page, B.profile);
  await expect(surface.getByRole("button", { name: "View log", exact: true })).toHaveCount(0);
  await page.evaluate(() => window.__detachedEvidenceButton.dispatchEvent(new MouseEvent("click")));
  await frame(page);
  expect(await widgetCalls(page, "codebuild-log")).toHaveLength(0);
});

test("closing and reopening linked stack rejects the previous reply; removing the tile rejects pending logs", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Inspect configured stack" }).click();
  const oldStack = await next(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Close linked evidence", exact: true }).click();
  await surface.getByRole("button", { name: "Inspect configured stack" }).click();
  const currentStack = await next(page, requestFor("cfn-stack-detail"));
  await reply(page, currentStack, stackDetail("current"));
  await reply(page, oldStack, stackDetail("obsolete"));
  await expect(surface).toContainText("Logs-current");
  await expect(surface).not.toContainText("Logs-obsolete");
  await hold(page, requestFor("log-tail"));
  await surface.getByRole("button", { name: "Browse log streams" }).click();
  const pending = await next(page, requestFor("log-tail"));
  await surface.locator(".rm-btn").click();
  await reply(page, pending, { streams: [{ name: "removed-stream" }] });
  await expect(page.locator("body")).not.toContainText("removed-stream");
});

test("explicit pipeline pins keep A and B investigation requests independent of the topbar", async ({ page }) => {
  await boot(page, [tile("pipeline-runs", { inputs: { pinned_pipelines: [
    { ...A, pipeline_name: "pipeline-a" }, { ...B, pipeline_name: "pipeline-b" },
  ] } })]);
  const surface = widget(page, "pipeline-runs");
  await surface.locator('.pipeline-tab[data-tab="pinned"]').click();
  const cards = surface.locator(".pipeline-pin-card");
  await expect(cards).toHaveCount(2);
  for (let index = 0; index < 2; index++) {
    await cards.nth(index).locator(".pipeline-pin-toggle").click();
    await expect(cards.nth(index).locator("table > tbody > tr:not(.row-detail)")).toHaveCount(1);
  }
  await hold(page, requestFor("pipeline-execution-detail"));
  await cards.nth(0).locator("table > tbody > tr:not(.row-detail)").click();
  const detailA = await next(page, requestFor("pipeline-execution-detail", { profile: A.profile }));
  await cards.nth(1).locator("table > tbody > tr:not(.row-detail)").click();
  const detailB = await next(page, requestFor("pipeline-execution-detail", { profile: B.profile }));
  await chooseAccount(page, B.profile);
  await reply(page, detailB, detail("b"));
  await reply(page, detailA, detail("a"));
  await hold(page, requestFor("codebuild-log"));
  await cards.nth(0).getByRole("button", { name: "View log", exact: true }).click();
  const buildA = await next(page, requestFor("codebuild-log", { profile: A.profile }));
  await cards.nth(1).getByRole("button", { name: "View log", exact: true }).click();
  const buildB = await next(page, requestFor("codebuild-log", { profile: B.profile }));
  expect(buildA.params.context).toEqual(pinned(A));
  expect(buildB.params.context).toEqual(pinned(B));
  await reply(page, buildB, events("build-only-b"));
  await reply(page, buildA, events("build-only-a"));
  await expect(cards.nth(0)).toContainText("build-only-a");
  await expect(cards.nth(0)).not.toContainText("build-only-b");
  await expect(cards.nth(1)).toContainText("build-only-b");
  await expect(cards.nth(1)).not.toContainText("build-only-a");
});

test("missing or malformed handoffs never fall back to raw provider identifiers", async ({ page }) => {
  const { surface } = await pipeline(page);
  const row = surface.locator(".pipeline-runs-rows > table > tbody > tr:not(.row-detail)");
  await row.click();
  await row.click();
  await reply(page, await next(page, requestFor("pipeline-execution-detail")), { render: "execution_detail", actions: [
    { provider: "CodeBuild", external_execution_id: "unreviewed-build" },
    { provider: "CodeBuild", external_execution_id: "bad-build", handoffs: { build: {
      ...buildLink("bad-build"), widget: "aws-cli", inputs: { command: "unreviewed command" },
    } } },
  ] });
  await expect(surface.getByRole("button", { name: "View log", exact: true })).toHaveCount(0);
  await expect(surface).toContainText("Linked targets are unknown");
  await expect(surface).toContainText("response did not establish a reviewed target");
  expect(await widgetCalls(page, "codebuild-log")).toHaveLength(0);
  expect(await widgetCalls(page, "aws-cli")).toHaveLength(0);
});

test("Lambda default convention is visibly unverified and needs an explicit attempt", async ({ page }) => {
  await boot(page, [tile("log-tail")]);
  const surface = widget(page, "log-tail");
  await hold(page, requestFor("log-tail"));
  await refreshReply(page, surface, "log-tail", { functions: [
    { name: "conventional-function", log_group: "/aws/lambda/conventional-function", handoffs: { logs:
      link("manual", "lambda_default_convention", "log-tail", { mode: "streams", log_group: "/aws/lambda/conventional-function" },
        "Default naming convention only; this log group is not verified. Check it explicitly.") } },
    { name: "foreign-function", log_group: "/synthetic/foreign", handoffs: { logs: unavailable("context_mismatch") } },
  ], coverage: coverage() });
  await surface.locator(".lambda-row").filter({ hasText: "conventional-function" }).click();
  await expect(surface).toContainText("Default naming convention only");
  expect((await widgetCalls(page, "log-tail")).filter(call => call.params.inputs.mode === "streams")).toHaveLength(0);
  await surface.getByRole("button", { name: "Try conventional log group" }).click();
  const streams = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  expect(streams.params.inputs.log_group).toBe("/aws/lambda/conventional-function");
  await surface.locator(".lambda-row").filter({ hasText: "foreign-function" }).click();
  await expect(surface.getByRole("button", { name: "Load streams", exact: true })).toBeDisabled();
  await reply(page, streams, { streams: [{ name: "obsolete-convention-stream" }] });
  await expect(surface).not.toContainText("obsolete-convention-stream");
});

test("a failed Lambda stream refresh cannot lend its request identity to retained stream controls", async ({ page }) => {
  await boot(page, [tile("log-tail")]);
  const surface = widget(page, "log-tail");
  await hold(page, requestFor("log-tail"));
  await surface.locator(".lambda-row").first().click();
  await reply(page, await next(page, requestFor("log-tail", { inputs: { mode: "streams" } })),
    { streams: [{ name: "retained-stream" }], coverage: coverage() });
  const view = surface.getByRole("button", { name: "View log", exact: true });
  await view.evaluate(button => { window.__staleStreamButton = button; });
  await surface.getByRole("button", { name: "Load streams", exact: true }).click();
  const retry = await next(page, requestFor("log-tail", { inputs: { mode: "streams" } }));
  await expect(view).toBeDisabled();
  await reply(page, retry, { ok: false, error: "Synthetic stream read failed" });
  await expect(view).toBeDisabled();
  await expect(surface.locator(".lambda-stream-panel")).toContainText("Refresh the source before opening linked evidence.");
  await page.evaluate(() => window.__staleStreamButton.dispatchEvent(new MouseEvent("click")));
  expect((await widgetCalls(page, "log-tail")).filter(call => call.params.inputs.mode === "events")).toHaveLength(0);
  await surface.getByRole("button", { name: "Load streams", exact: true }).click();
  await reply(page, await next(page, requestFor("log-tail", { inputs: { mode: "streams" } })),
    { streams: [{ name: "current-stream" }], coverage: coverage() });
  await expect(view).toBeEnabled();
});

test("reviewed detail reuse shows original capture time and keeps the current verified envelope", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Inspect configured stack" }).click();
  const request = await next(page, requestFor("cfn-stack-detail"));
  expect(request.params.reuse_result).toBe(true);
  const captured = Math.floor(Date.now() / 1000) - 10;
  await reply(page, request, { ...stackDetail("cached"), _cache: { hit: true, captured_at: captured, max_age_seconds: 15 } });
  const result = surface.locator(".result-status").filter({ has: page.locator(".result-cached") }).last();
  await expect(result).toContainText("Cached evidence · original capture time");
  await expect(result.locator("time")).toHaveAttribute("datetime", new Date(captured * 1000).toISOString());
  await expect(result).toContainText(A.account_id);
  await expect(surface).toContainText("Logs-cached");
});

test("invalid cached capture metadata cannot promote an old detail into newly received evidence", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("cfn-stack-detail"));
  await surface.getByRole("button", { name: "Inspect configured stack" }).click();
  const request = await next(page, requestFor("cfn-stack-detail"));
  await reply(page, request, { ...stackDetail("invalid-cache"), _cache: { hit: true, captured_at: -1, max_age_seconds: 15 } });
  await expect(surface).toContainText("Response freshness metadata was invalid.");
  await expect(surface).not.toContainText("Logs-invalid-cache");
});

test("manual parent Refresh requests current evidence without cache reuse opt-in", async ({ page }) => {
  const { surface } = await pipeline(page);
  await hold(page, requestFor("pipeline-runs"));
  await refresh(surface);
  const request = await next(page, requestFor("pipeline-runs"));
  expect(request.params).not.toHaveProperty("reuse_result");
  await reply(page, request, runs("fresh-parent-execution"));
  await expect(surface.locator(".pipeline-runs-rows")).toContainText("fresh-parent-execution");
  await expect(surface.locator(".result-cached")).toHaveCount(0);
});
