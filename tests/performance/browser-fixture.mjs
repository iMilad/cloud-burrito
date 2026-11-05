// Versioned synthetic boundary for the production frontend performance runner.
// Keep this function self-contained: Playwright serializes it into a fresh page.
// v2: the workload is watchlist things, not grid tiles. Do not compare with v1 reports.
export const FIXTURE_REVISION = "p301-browser-v2";
export const FIXTURE_SEED = 7301;

export function installSyntheticBridge(options) {
  const profiles = ["a", "b"].map(name => ({ profile: `synthetic-${name}`, account_id: `acct-${name}-fixture`, region: "eu-west-1" }));
  const defaults = { aws_config_path: "/synthetic/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
  const settings = { ...defaults, default_profile: profiles[0].profile };
  const fixture = window.__performanceFixture = {
    revision: options.fixtureRevision, seed: options.seed, milestones: {}, events: [], errors: [],
    commands: {}, widgets: {}, active: 0, peakActive: 0, responseBytes: 0, completed: 0,
    logicalRequests: 0, widgetResponses: 0, responses: [], transport: "synthetic_invoke_only",
  };
  let sequence = 0;
  let active = null;
  let outlierUsed = false;
  let lastResultFramePending = false;
  let denyNext = false;
  let partialNext = false;
  fixture.scriptOutcome = outcome => { denyNext = outcome === "denied"; partialNext = outcome === "partial"; };
  const settingsResponse = status => ({ ...settings, _storage: { store: "settings", status },
    _settings: { defaults, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} } });
  const thing = (id, kind, inputs, profile = profiles[0]) => ({ id: `bench-${id}`, kind, order: 0, inputs,
    context: { mode: "pinned", ...profile } });
  const packageInputs = { domain: "synthetic-domain", repository: "synthetic-repository", package_prefix: "synthetic" };
  const things = options.kind === "six" ? [
      thing("stacks", "stacks-search", { name_prefix: "synthetic-" }),
      thing("errors", "errors", { hours: 1, log_group_pattern: "/aws/lambda/" }),
      thing("lambda", "logs", { source: "lambda", log_group: "/synthetic/benchmark" }),
      thing("logs", "logs", { source: "group", log_group: "/synthetic/benchmark" }),
      thing("packages", "package", packageInputs),
      thing("cli", "cli", { command: "aws sts get-caller-identity" }),
    ]
    : options.kind === "pins" ? Array.from({ length: 50 }, (_, index) => thing(`pipeline-${index}`, "pipeline",
      { pipeline_name: `synthetic-pipeline-${String(index).padStart(3, "0")}` }, profiles[index % 2]))
    : [options.kind === "packages" ? thing("packages", "package", packageInputs)
      : thing("stacks", "stacks-search", { name_prefix: "synthetic-" })];
  const completeCoverage = count => ({ completeness: "complete", has_more: false,
    counts: { returned: count, pages: 1 }, limits: {}, reasons: [] });
  const stackRows = Array.from({ length: options.rows }, (_, index) => ({
    stack: `synthetic-stack-${String(index).padStart(5, "0")}`,
    status: index % 7 === 0 ? "UPDATE_FAILED" : "CREATE_COMPLETE",
    description: `seed-${options.seed}-row-${index}-` + "synthetic-long-cell-content ".repeat(24),
    last_updated: "2026-01-01T00:00:00Z",
  }));
  const packages = Array.from({ length: options.rows }, (_, index) => ({
    package: `synthetic-package-${String(index).padStart(5, "0")}`,
    latest_version: "1.0.2", last_published: "2026-01-01T00:00:00Z",
    versions: ["1.0.2", "1.0.1", "1.0.0"],
  }));
  const contextFor = params => params.context?.mode === "pinned" ? params.context : active;
  const attach = (result, params, context) => ({ ...result, _request: {
    id: params.request_id || null,
    context_id: context ? `synthetic-context-${context.profile}-${context.region}` : null,
    provider_revision: context ? "synthetic-provider-v1" : null, settings_revision: context ? "1" : null,
    profile: context?.profile || null, account_id: context?.account_id || null, region: context?.region || null,
    outcome: result.render === "permission_denied" ? "denied" : result.ok === false ? "failed" : "succeeded",
  } });
  function widgetResponse(params) {
    if (denyNext) { denyNext = false; return { render: "permission_denied", action: "cloudformation:DescribeStacks", reason: "Scripted benchmark denial" }; }
    let result;
    switch (params.widget) {
      case "cfn-stacks": result = { render: "table", columns: ["stack", "status", "description", "last_updated"], rows: stackRows, coverage: completeCoverage(stackRows.length) }; break;
      case "cfn-stack-detail": result = { render: "stack_detail", resources: [{ logical_id: "SyntheticLogGroup", type: "AWS::Logs::LogGroup", physical_id: "/synthetic/benchmark", status: "CREATE_COMPLETE" }],
        events: [{ logical_id: "SyntheticLogGroup", status: "CREATE_COMPLETE", time: "2026-01-01T00:00:00Z", reason: "Synthetic detail fixture" }], coverage: completeCoverage(2) }; break;
      case "pipeline-runs": result = { render: "table", columns: ["execution_id", "status"], rows: [{ execution_id: "synthetic-execution", status: "Failed" }], coverage: completeCoverage(1) }; break;
      case "pipeline-execution-detail": result = { render: "execution_detail", actions: [{ stage: "Build", action: "SyntheticBuild", status: "Failed", provider: "CodeBuild", error: "Synthetic build error" }], coverage: completeCoverage(1) }; break;
      case "codeartifact-packages": {
        let rows = packages;
        if (params.inputs.mode === "list") {
          const token = params.inputs.page_token || "synthetic-page-0";
          if (!/^synthetic-page-\d+$/.test(token)) throw new Error("Unexpected synthetic page token");
          const start = Number(token.slice("synthetic-page-".length));
          rows = packages.slice(start, start + 50).map(row => ({ package: row.package, latest_version: "", last_published: "", versions: [], enrichment_state: "pending" }));
          const next = start + rows.length < packages.length ? `synthetic-page-${start + rows.length}` : null;
          result = { render: "table", columns: ["package", "latest_version", "last_published"], rows, next_page_token: next,
            coverage: { ...completeCoverage(rows.length), completeness: next ? "limited" : "complete", has_more: !!next } };
        } else if (params.inputs.mode === "enrich") {
          if (!Array.isArray(params.inputs.packages) || params.inputs.packages.length > 25
              || params.inputs.packages.some(name => !packages.some(row => row.package === name))) throw new Error("Unexpected synthetic enrichment input");
          rows = packages.filter(row => params.inputs.packages.includes(row.package)).map(row => ({ ...row, enrichment_state: "complete" }));
          result = { render: "table", columns: ["package", "latest_version", "last_published"], rows, coverage: completeCoverage(rows.length) };
        } else if (params.inputs.mode === undefined) result = { render: "table", columns: ["package", "latest_version", "last_published"], rows, coverage: completeCoverage(rows.length) };
        break;
      }
      case "codeartifact-package-version-history": result = { render: "codeartifact_version_history", package: params.inputs.package,
        versions: ["1.0.2", "1.0.1", "1.0.0"].map(version => ({ version, published: "2026-01-01T00:00:00Z" })), coverage: completeCoverage(3) }; break;
      case "aws-cli": result = { render: "table", columns: ["fixture"], rows: [{ fixture: "Synthetic CLI output; no process executed" }], coverage: completeCoverage(1) }; break;
      case "errors-by-stack": result = { render: "errors_chart", rows: [{ stack: "/synthetic/benchmark", errors: 4 }], hours: 1, coverage: completeCoverage(1) }; break;
      case "log-tail":
        if (params.inputs.mode === "list") result = { functions: [{ name: "synthetic-function", log_group: "/synthetic/benchmark", runtime: "synthetic", state: "Active", handoffs: { logs: {
          status: "available", source: "lambda_logging_config", widget: "log-tail", inputs: { mode: "streams", log_group: "/synthetic/benchmark" }, reason: "Synthetic configured group" } } }], coverage: completeCoverage(1) };
        else if (params.inputs.mode === "streams") result = { streams: [{ name: "synthetic-stream" }], coverage: completeCoverage(1) };
        else if (params.inputs.mode === "events") result = { render: "log_stream", events: [{ ts: 1700000000000, msg: "Synthetic benchmark event", level: "info" }], coverage: completeCoverage(1) };
        break;
      case "cloudwatch-logs":
        if (params.inputs.mode === "groups") result = { groups: [{ name: "/synthetic/benchmark", arn: "synthetic-group" }], coverage: completeCoverage(1) };
        else if (params.inputs.mode === "streams") result = { streams: [{ name: "synthetic-stream" }], coverage: completeCoverage(1) };
        else if (params.inputs.mode === "events") result = { render: "log_stream", events: [{ ts: 1700000000000, msg: "Synthetic benchmark event", level: "info" }], coverage: completeCoverage(1) };
        break;
    }
    if (!result) { fixture.errors.push("Unexpected synthetic widget operation"); throw new Error("Unexpected synthetic widget operation"); }
    if (partialNext) {
      partialNext = false;
      result = { ...result, ok: false, partial: true, error: "Scripted later-page failure; returned evidence retained", error_type: "PartialFailure",
        coverage: { ...(result.coverage || completeCoverage(0)), completeness: "unknown", has_more: null,
          reasons: [{ code: "request_failed", message: "Scripted later-page failure" }] } };
    }
    return result;
  }
  window.__TAURI__ = { core: { invoke: async (command, payload) => {
    const params = payload?.params || {};
    const started = performance.now();
    const id = ++sequence;
    fixture.logicalRequests++;
    fixture.commands[command] = (fixture.commands[command] || 0) + 1;
    if (command === "widget_fetch") fixture.widgets[params.widget] = (fixture.widgets[params.widget] || 0) + 1;
    fixture.active++;
    fixture.peakActive = Math.max(fixture.peakActive, fixture.active);
    try {
      let delay = ["aws_set_account", "widget_fetch"].includes(command) ? options.delayMs : 0;
      if (options.outlier && !outlierUsed && command === "widget_fetch") { outlierUsed = true; delay = 5000; }
      if (command === "aws_set_account") { active = null; fixture.milestones.connectStarted ??= started; }
      // Pinned things verify on their first request; there is no separate connect step.
      if (command === "widget_fetch") fixture.milestones.connectStarted ??= started;
      if (delay) await new Promise(resolve => setTimeout(resolve, delay));
      let result;
      switch (command) {
        case "request_cancel":
          if (typeof params.request_id !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(params.request_id)) throw new Error("Invalid synthetic cancellation ID");
          result = { ok: true, cancelled_locally: true, cleanup_confirmed: false }; break;
        case "ping": result = { ok: true, version: "synthetic-benchmark" }; break;
        case "settings_get": result = settingsResponse("loaded"); break;
        case "settings_set": Object.assign(settings, params); result = settingsResponse("saved"); break;
        case "watchlist_get": result = { version: 1, things, _storage: { store: "watchlist", status: "loaded" } }; break;
        case "watchlist_set": result = { version: 1, things: params.things, _storage: { store: "watchlist", status: "saved" } }; break;
        case "aws_list_profiles": result = { discovery_state: "ready", file_exists: true, profiles: profiles.map(profile => ({
          name: profile.profile, account_id: profile.account_id, region: profile.region, role_name: "SyntheticReadOnly",
          sso_session: "synthetic-session", eligibility: "supported_sso", eligibility_reason: null,
        })) }; break;
        case "aws_set_account": active = { profile: params.profile, account_id: params.account_id, region: params.region }; result = attach({ ok: true }, params, active); fixture.milestones.identityResponse = performance.now(); break;
        case "aws_auth_status": result = attach({ has_context: !!active, logged_in: !!active, connection_state: active ? "verified" : "unselected", ...active }, params, active); break;
        case "aws_list_pipelines": result = attach({ pipelines: [{ name: "synthetic-pipeline", updated: "2026-01-01T00:00:00Z" }], coverage: completeCoverage(1) }, params, contextFor(params)); break;
        case "cli_availability": result = { ok: true, status: "available", available: true, version_verified: false }; break;
        case "widget_fetch": result = attach(widgetResponse(params), params, contextFor(params)); fixture.widgetResponses++; break;
        case "policy_get": case "policy_set": result = { raw: "statements: []", valid: true, actions: [], path: "/synthetic/policy" }; break;
        case "audit_history": if (params.action !== "status") throw new Error("Unexpected synthetic audit action"); result = { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." }; break;
        case "audit_tail": result = { entries: [] }; break;
        case "widget_get_source": result = { ok: true, yaml: "name: synthetic", py: "// Synthetic source" }; break;
        default: fixture.errors.push("Unexpected synthetic bridge command"); throw new Error("Unexpected synthetic bridge command");
      }
      const bytes = new TextEncoder().encode(JSON.stringify(result)).byteLength;
      fixture.responseBytes += bytes;
      fixture.completed++;
      fixture.responses.push({ id, command, widget: command === "widget_fetch" ? params.widget : null,
        started_ms: started, response_ready_ms: performance.now(), delay_ms: delay, bytes,
        outcome: result._request?.outcome || (result.ok === false ? "failed" : "succeeded") });
      return result;
    } finally { fixture.active--; }
  } } };
  const observe = () => {
    if (lastResultFramePending) return;
    lastResultFramePending = true;
    requestAnimationFrame(() => {
      lastResultFramePending = false;
      const now = performance.now();
      if (document.readyState !== "loading" && document.body.dataset.ready === "true") fixture.milestones.interactive ??= now;
      if (fixture.widgetResponses > 0) fixture.milestones.verified ??= now;
      const useful = document.querySelector('.wl-row[data-loading="false"]');
      if (useful) fixture.milestones.firstUseful ??= now;
      if (useful && fixture.active === 0 && fixture.widgetResponses > 0) fixture.milestones.lastSettledResultFrame = now;
    });
  };
  document.addEventListener("DOMContentLoaded", () => {
    fixture.milestones.domContentLoaded = performance.now();
    new MutationObserver(observe).observe(document.body, { subtree: true, childList: true, attributes: true });
    observe();
  });
}
