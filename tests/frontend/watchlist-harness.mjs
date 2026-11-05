// Fake Tauri bridge for watchlist specs. Only the native invoke boundary is
// fake; production core.js and watchlist scripts run unchanged. No AWS.
export const A = { profile: "demo-a", account_id: "acct-a-fixture", region: "eu-west-1" };
export const B = { profile: "demo-b", account_id: "acct-b-fixture", region: "eu-west-1" };
export const pinned = identity => ({ mode: "pinned", ...identity });
export const thing = (id, kind, identity, inputs, order = 0) => ({ id, kind, order, context: pinned(identity), inputs });

export async function bootWatchlist(page, { things = [], storage = "loaded", tiles = [], profiles = [A, B], settings = {} } = {}) {
  await page.context().route("**/*", route => new URL(route.request().url()).origin === "http://127.0.0.1:4173"
    ? route.continue() : route.abort("blockedbyclient"));
  await page.addInitScript(({ things, storage, tiles, profiles, settings }) => {
    const calls = [];
    let saved = structuredClone(things);
    let status = storage;
    const failures = new Map();
    const holds = [];
    const meta = params => ({ id: params.request_id ?? null, context_id: `ctx-${params.context?.profile}`,
      provider_revision: "p1", settings_revision: "s1", profile: params.context?.profile ?? null,
      account_id: params.context?.account_id ?? null, region: params.context?.region ?? null, outcome: "succeeded" });
    const failed = params => ({ ...meta(params), context_id: null, provider_revision: null, settings_revision: null,
      profile: null, account_id: null, region: null, outcome: "failed" });
    const logs = i => i.mode === "streams"
      ? { ok: true, log_group: i.log_group, streams: [
        { name: "synthetic-stream-new", last_event_timestamp: 1700000600000, creation_time: 1700000000000 },
        { name: "synthetic-stream-old", last_event_timestamp: 1700000000000, creation_time: 1699990000000 }].slice(0, i.max_streams) }
      : { render: "log_stream", events: [
        { ts: 1700000600000, msg: `event from ${i.log_stream}`, level: "info" },
        { ts: 1700000601000, msg: "ERROR synthetic timeout", level: "error" }] };
    const widgets = {
      "log-tail": i => logs(i),
      "cloudwatch-logs": i => logs(i),
      "pipeline-runs": i => ({ render: "table", columns: ["execution_id", "status", "last_updated", "trigger"], rows: [
        { execution_id: "exec-0042", status: i.pipeline_name === "synthetic-failing" ? "Failed" : "Succeeded",
          last_updated: "2026-11-02T14:02:00Z", trigger: "Webhook: main@a1f3e9c" },
        { execution_id: "exec-0041", status: "Succeeded", last_updated: "2026-11-02T13:00:00Z", trigger: "Webhook: main@7c2d8b1" }] }),
      "pipeline-execution-detail": () => ({ render: "execution_detail", actions: [
        { stage: "Build", action: "Unit tests", status: "Succeeded",
          external_url: "https://eu-west-1.console.aws.amazon.com/codesuite/codebuild/synthetic" },
        { stage: "Deploy", action: "Integration tests", status: "Failed", error: "Build failed",
          handoffs: { build: { status: "available", source: "codebuild_logs", widget: "codebuild-log",
            inputs: { build_id: "synthetic-build:2" }, reason: "Synthetic build reference." } } }] }),
      "codebuild-log": () => ({ render: "log_stream", events: [
        { ts: 1700000000000, msg: "running tests", level: "info" },
        { ts: 1700000001000, msg: "FAILED synthetic_test", level: "error" },
        { ts: 1700000002000, msg: "cleanup", level: "info" },
        { ts: 1700000003000, msg: "Phase complete: FAILED", level: "error" }] }),
      "codeartifact-packages": i => i.mode === "enrich"
        ? { render: "table", mode: "enrich", columns: ["package", "latest_version", "last_published"],
          rows: i.packages.map(p => ({ package: p, latest_version: "4.12.0", last_published: "2026-11-02T11:02:00Z",
            versions: [{ version: "4.12.0", published: "2026-11-02T11:02:00Z" }, { version: "4.11.3", published: "2026-10-30T09:00:00Z" }],
            enrichment_state: "complete" })) }
        : { render: "table", mode: "list", columns: ["package", "latest_version", "last_published"], next_page_token: null,
          rows: [{ package: i.package_prefix, latest_version: "", last_published: "", versions: [], enrichment_state: "pending" }] },
      "codeartifact-package-version-history": i => ({ render: "codeartifact_version_history", versions: i.versions }),
      "cfn-stack-detail": i => ({ render: "stack_detail", stack: i.stack_name, resources: [
        { logical_id: "Table", type: "AWS::DynamoDB::Table", status: "UPDATE_FAILED", physical_id: `${i.stack_name}-table` }],
        events: [{ time: "2026-11-02T12:05:00Z", logical_id: i.stack_name, type: "AWS::CloudFormation::Stack",
          status: "UPDATE_ROLLBACK_COMPLETE", reason: "" }] }),
      "cfn-stacks": i => ({ render: "table", columns: ["stack", "status", "resources", "last_updated"],
        rows: [{ stack: `${i.name_prefix}one`, status: "UPDATE_COMPLETE", resources: 3, last_updated: "2026-11-02T10:00:00Z" },
          { stack: `${i.name_prefix}two`, status: "CREATE_COMPLETE", resources: 5, last_updated: "2026-11-01T10:00:00Z" }] }),
      "resource-lookup": i => ({ render: "reverse_lookup", query: i.query, scanned: 2,
        matches: [{ arn: `arn:aws:lambda:eu-west-1:acct-a-fixture:function:${i.query}`, type: "lambda:function" }] }),
      "aws-cli": () => ({ render: "table", columns: ["value"], rows: [{ value: "synthetic-cli" }] }),
    };
    function respond(command, params) {
      switch (command) {
        case "settings_get": return { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "demo-a",
          default_region: "eu-west-1", theme: "dark", audit_retention: "preserve", ...settings,
          _storage: { store: "settings", status: "loaded" } };
        case "aws_list_profiles": return { ok: true, allowed_regions: ["eu-west-1", "us-east-1"],
          profiles: profiles.map(p => ({ name: p.profile, account_id: p.account_id, region: p.region, eligibility: "supported_sso" })) };
        case "watchlist_get": return { version: 1, things: structuredClone(saved), _storage: { store: "watchlist", status } };
        case "watchlist_set": saved = structuredClone(params.things); status = "loaded";
          return { version: 1, things: structuredClone(saved), _storage: { store: "watchlist", status: "saved" } };
        case "dashboard_get": return { version: 1, tiles: structuredClone(tiles), _storage: { store: "dashboard", status: tiles.length ? "loaded" : "missing" } };
        case "request_cancel": return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
        case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
        case "aws_list_pipelines": return { ok: true, pipelines: [{ name: "synthetic-listed", updated: "2026-11-01T00:00:00Z" }], _request: meta(params) };
        case "widget_fetch": {
          const left = failures.get(params.widget) || 0;
          if (left > 0) {
            failures.set(params.widget, left - 1);
            return { ok: false, error_type: "ClientError", error: "Synthetic failure", render: "raw_json",
              data: { error: "Synthetic failure" }, _request: failed(params) };
          }
          if (!widgets[params.widget]) throw new Error(`Unexpected synthetic widget: ${params.widget}`);
          return { ...widgets[params.widget](params.inputs || {}), _request: meta(params) };
        }
        default: throw new Error(`Unexpected synthetic invoke: ${command}`);
      }
    }
    window.__watchlistHarness = {
      // `widgets` is the live answer table; a spec may replace one entry.
      calls, widgets, saved: () => structuredClone(saved), fail: (name, count = 1) => failures.set(name, count),
      hold: name => holds.push({ name }),
      release: name => { const item = holds.find(h => h.name === name && h.resolve); holds.splice(holds.indexOf(item), 1); item.resolve(); },
    };
    window.__TAURI__ = { core: { async invoke(command, payload) {
      const params = structuredClone(payload?.params || {});
      calls.push({ command, params });
      const holdName = command === "widget_fetch" ? params.widget : command;
      const hold = holds.find(h => h.name === holdName && !h.resolve);
      if (hold) await new Promise(resolve => { hold.resolve = resolve; });
      return respond(command, params);
    } } };
  }, { things, storage, tiles, profiles, settings });
  await page.goto("/watchlist.html");
  await page.waitForFunction(() => document.body.dataset.ready === "true");
}

export const calls = (page, command, widget) => page.evaluate(({ command, widget }) => window.__watchlistHarness.calls
  .filter(c => (!command || c.command === command) && (!widget || c.params.widget === widget)), { command, widget });
export const saved = page => page.evaluate(() => window.__watchlistHarness.saved());
export const row = (page, id) => page.locator(`.wl-row[data-thing-id="${id}"]`);
export const rowText = (page, id) => row(page, id).locator(".wl-state-text");
