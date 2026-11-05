// Browser demo only: answers the watchlist's invoke calls with sample data so
// the page works when opened directly in a browser. Inactive inside Tauri.
(function () {
  "use strict";
  if (typeof window.__TAURI__ !== "undefined") return;
  const PROFILES = [
    { name: "prod-payments", account_id: "acct-payments-demo", region: "eu-west-1" },
    { name: "prod-core", account_id: "acct-core-demo", region: "eu-west-1" },
    { name: "staging", account_id: "acct-staging-demo", region: "eu-west-1" },
    { name: "shared-tools", account_id: "acct-tools-demo", region: "eu-west-1" },
  ];
  const pin = name => { const p = PROFILES.find(x => x.name === name); return { mode: "pinned", profile: p.name, account_id: p.account_id, region: p.region }; };
  let things = [
    { id: "demo-pipe-1", kind: "pipeline", order: 0, context: pin("prod-payments"), inputs: { pipeline_name: "payments-api" } },
    { id: "demo-pipe-2", kind: "pipeline", order: 1, context: pin("prod-core"), inputs: { pipeline_name: "web-frontend" } },
    { id: "demo-pipe-3", kind: "pipeline", order: 2, context: pin("prod-core"), inputs: { pipeline_name: "ledger-worker" } },
    { id: "demo-pkg-1", kind: "package", order: 0, context: pin("shared-tools"), inputs: { domain: "acme", repository: "pypi-internal", package_prefix: "acme-core" } },
    { id: "demo-pkg-2", kind: "package", order: 1, context: pin("shared-tools"), inputs: { domain: "acme", repository: "pypi-internal", package_prefix: "acme-ledger" } },
    { id: "demo-logs-1", kind: "logs", order: 0, context: pin("prod-payments"), inputs: { log_group: "/aws/lambda/payments-handler", source: "lambda", function_name: "payments-handler" } },
    { id: "demo-stack-1", kind: "stack", order: 0, context: pin("prod-payments"), inputs: { stack_name: "payment-service-prod" } },
    { id: "demo-stack-2", kind: "stack", order: 1, context: pin("staging"), inputs: { stack_name: "ledger-db-stg" } },
  ];
  const ago = minutes => new Date(Date.now() - minutes * 60000).toISOString();
  const meta = params => { const c = params.context || {}; return { id: params.request_id ?? null, context_id: `demo-${c.profile}`,
    provider_revision: "demo", settings_revision: "demo", profile: c.profile ?? null, account_id: c.account_id ?? null,
    region: c.region ?? null, outcome: "succeeded" }; };
  const build = (id, why) => ({ build: { status: "available", source: "codebuild_logs", widget: "codebuild-log", inputs: { build_id: id }, reason: why } });
  function widget(params) {
    const i = params.inputs || {};
    switch (params.widget) {
      case "pipeline-runs": return { render: "table", columns: ["execution_id", "status", "last_updated", "trigger"], rows: [
        { execution_id: "exec-9f2a8c1b-0042", status: i.pipeline_name === "payments-api" ? "Failed" : "Succeeded", last_updated: ago(2), trigger: "Webhook: main@a1f3e9c" },
        { execution_id: "exec-9f2a8c1b-0041", status: "Succeeded", last_updated: ago(64), trigger: "Webhook: main@7c2d8b1" },
        { execution_id: "exec-9f2a8c1b-0040", status: "Succeeded", last_updated: ago(190), trigger: "Webhook: main@b9e4c0f" }].slice(0, i.max_results || 3) };
      case "pipeline-execution-detail": return { render: "execution_detail", actions: [
        { stage: "Source", action: "GitHub source", status: "Succeeded", category: "Source", provider: "CodeStarSourceConnection", last_updated: ago(8) },
        { stage: "Build", action: "Unit tests", status: "Succeeded", category: "Build", provider: "CodeBuild", last_updated: ago(6), handoffs: build("unit-tests:demo-0001", "Build ID reported by the action execution.") },
        { stage: "Deploy", action: "Integration tests", status: "Failed", category: "Test", provider: "CodeBuild", last_updated: ago(2), error: "Build failed: exit status 1", handoffs: build("integration-tests:demo-0002", "Build ID reported by the action execution.") }] };
      case "codebuild-log": return { render: "log_stream", events: [
        { ts: Date.now() - 120000, msg: "[Container] Entering phase BUILD", level: "info" },
        { ts: Date.now() - 119000, msg: "Running command pytest tests/integration -q", level: "info" },
        { ts: Date.now() - 105000, msg: "FAILED test_charge_timeout - ProviderTimeout: provider did not answer in 3000ms", level: "error" },
        { ts: Date.now() - 104000, msg: "1 failed, 37 passed in 13.82s", level: "info" },
        { ts: Date.now() - 103000, msg: "[Container] Phase complete: BUILD State: FAILED", level: "error" }] };
      case "codeartifact-packages": {
        const latest = { "acme-core": "4.12.0", "acme-core-cli": "1.3.2", "acme-ledger": "2.4.0", "acme-ledger-client": "2.3.1" };
        if (i.mode === "enrich") return { render: "table", mode: "enrich", columns: ["package", "latest_version", "last_published"],
          rows: (i.packages || []).map(name => ({ package: name, latest_version: latest[name] || "1.0.0", last_published: ago(200),
            versions: [{ version: latest[name] || "1.0.0", published: ago(200) }, { version: "0.9.0", published: ago(9000) }], enrichment_state: "complete" })) };
        return { render: "table", mode: "list", columns: ["package", "latest_version", "last_published"], next_page_token: null,
          rows: Object.keys(latest).filter(name => name.startsWith(i.package_prefix || "")).slice(0, i.max_packages || 50)
            .map(name => ({ package: name, latest_version: "", last_published: "", versions: [], enrichment_state: "pending" })) };
      }
      case "codeartifact-package-version-history": return { render: "codeartifact_version_history",
        versions: (i.versions || []).map(v => ({ version: v.version, published: v.published || ago(300) })) };
      case "log-tail": case "cloudwatch-logs":
        if (i.mode === "streams") return { ok: true, log_group: i.log_group, streams: [
          { name: "2026/11/02/[$LATEST]7a3e1c9b", last_event_timestamp: Date.now() - 60000, creation_time: Date.now() - 3600000 },
          { name: "2026/11/02/[$LATEST]52bd09e4", last_event_timestamp: Date.now() - 900000, creation_time: Date.now() - 7200000 }].slice(0, i.max_streams || 50) };
        if (i.mode === "events") return { render: "log_stream", events: [
          { ts: Date.now() - 60000, msg: "calling provider.charge(orderId=ord-8821)", level: "info" },
          { ts: Date.now() - 59000, msg: "provider.charge took 1840ms (p95 budget 1200ms)", level: "warn" },
          { ts: Date.now() - 58000, msg: "ProviderTimeout after 3000ms invoking https://api.example.com/charge", level: "error" }] };
        return { ok: true, groups: [], functions: [] };
      case "cfn-stack-detail": {
        const rollback = i.stack_name === "ledger-db-stg";
        return { render: "stack_detail", stack: i.stack_name, resources: [
          { logical_id: "Handler", type: "AWS::Lambda::Function", status: "UPDATE_COMPLETE", physical_id: `${i.stack_name}-handler` },
          { logical_id: "Table", type: "AWS::DynamoDB::Table", status: rollback ? "UPDATE_FAILED" : "UPDATE_COMPLETE", physical_id: `${i.stack_name}-table` }],
        events: [
          { time: ago(30), logical_id: i.stack_name, type: "AWS::CloudFormation::Stack", status: rollback ? "UPDATE_ROLLBACK_COMPLETE" : "UPDATE_COMPLETE", reason: "" },
          { time: ago(34), logical_id: "Table", type: "AWS::DynamoDB::Table", status: rollback ? "UPDATE_FAILED" : "UPDATE_COMPLETE", reason: rollback ? "Resource limit exceeded" : "" }] };
      }
      case "cfn-stacks": return { render: "table", columns: ["stack", "status", "resources", "last_updated"], rows: [
        { stack: "ledger-db-stg", status: "UPDATE_ROLLBACK_COMPLETE", resources: 9, last_updated: ago(30) },
        { stack: "web-frontend-stg", status: "UPDATE_IN_PROGRESS", resources: 14, last_updated: ago(3) }].filter(r => r.stack.startsWith(i.name_prefix || "")) };
      case "resource-lookup": return { render: "reverse_lookup", query: i.query, scanned: 120, matches: [{ arn: `arn:aws:lambda:eu-west-1:demo:function:${i.query}`, type: "lambda:function" }] };
      case "logs-insights": return { render: "table", columns: ["@timestamp", "@message"], rows: [{ "@timestamp": ago(5), "@message": "ERROR ProviderTimeout" }] };
      case "errors-by-stack": return { render: "errors_chart", hours: i.hours || 24, rows: [{ stack: "payment-service-prod", errors: 247 }, { stack: "order-service-prod", errors: 89 }] };
      case "aws-cli": return { render: "table", columns: ["Account", "Arn"], rows: [{ Account: "acct-demo", Arn: "arn:aws:sts::demo:assumed-role/DemoReadOnly/demo" }] };
      default: return { render: "raw_json", data: { error: "This demo has no sample data for that request." } };
    }
  }
  async function invoke(command, args) {
    const params = structuredClone(args?.params || {});
    switch (command) {
      case "settings_get": return { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "prod-core", default_region: "eu-west-1",
        theme: "dark", audit_retention: "preserve", _storage: { store: "settings", status: "loaded" } };
      case "aws_list_profiles": return { ok: true, allowed_regions: ["eu-west-1", "us-east-1"],
        profiles: PROFILES.map(p => ({ ...p, role_name: "DemoReadOnly", sso_session: "demo-session", eligibility: "supported_sso" })) };
      case "watchlist_get": return { version: 1, things: structuredClone(things), _storage: { store: "watchlist", status: "loaded" } };
      case "watchlist_set": things = structuredClone(params.things || []);
        return { version: 1, things: structuredClone(things), _storage: { store: "watchlist", status: "saved" } };
      case "dashboard_get": return { version: 1, tiles: [], _storage: { store: "dashboard", status: "missing" } };
      case "aws_list_pipelines": return { ok: true, pipelines: ["auth-service", "ledger-worker", "payments-api", "web-frontend"]
        .map(name => ({ name, updated: ago(60) })), _request: meta(params) };
      case "widget_fetch": return { ...widget(params), _request: meta(params) };
      case "request_cancel": return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
      case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
      case "aws_auth_status": { const c = params.context || {}; return { has_context: true, logged_in: true, connection_state: "verified",
        profile: c.profile, account_id: c.account_id, region: c.region,
        sso_token_expires_at: new Date(Date.now() + 26640000).toISOString(), expires_at: new Date(Date.now() + 3600000).toISOString(), _request: meta(params) }; }
      default: throw new Error("This demo has no sample data for that request.");
    }
  }
  window.CloudBurritoDemo = { invoke };
})();
