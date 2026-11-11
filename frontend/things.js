// Pure watchlist model: kinds, validation, row requests, summaries, ordering.
// No DOM, bridge or AWS. Loaded as a plain script and by node unit tests.
(function (root) {
  "use strict";

  const GROUPS = [
    { key: "pipelines", label: "Pipelines" },
    { key: "packages", label: "Packages" },
    { key: "logs", label: "Logs" },
    { key: "stacks", label: "Stacks" },
    { key: "saved", label: "Saved commands" },
  ];
  const KINDS = {
    pipeline: { group: "pipelines", badge: "P", autoload: true, title: "Pipeline" },
    package: { group: "packages", badge: "A", autoload: true, title: "Package" },
    logs: { group: "logs", badge: "L", autoload: true, title: "Logs" },
    stack: { group: "stacks", badge: "S", autoload: true, title: "Stack" },
    "stacks-search": { group: "saved", badge: "S", autoload: true, title: "Stacks search" },
    lookup: { group: "saved", badge: "R", autoload: true, title: "Resource lookup" },
    query: { group: "saved", badge: "Q", autoload: false, title: "Logs Insights query", cost: true },
    errors: { group: "saved", badge: "E", autoload: false, title: "Error counts", cost: true },
    cli: { group: "saved", badge: "C", autoload: false, title: "AWS CLI command" },
  };
  const CONTROL = /[\u0000-\u001f\u007f]/u;

  function text(value, max = 2048) {
    if (typeof value !== "string") return "";
    const trimmed = value.trim();
    return trimmed && trimmed.length <= max && !CONTROL.test(trimmed) ? trimmed : "";
  }
  function multiline(value, max) {
    if (typeof value !== "string") return "";
    const trimmed = value.trim();
    return trimmed && trimmed.length <= max && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(trimmed) ? trimmed : "";
  }
  function int(value, min, max, fallback) {
    const number = typeof value === "string" ? Number.parseInt(value, 10) : value;
    return Number.isSafeInteger(number) && number >= min && number <= max ? number : fallback;
  }

  function pinned(raw) {
    if (!raw || typeof raw !== "object" || raw.mode !== "pinned") return null;
    const profile = text(raw.profile, 256), account = text(raw.account_id, 128), region = text(raw.region, 128);
    return profile && account && region ? { mode: "pinned", profile, account_id: account, region } : null;
  }

  function inputsFor(kind, raw) {
    const r = raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
    switch (kind) {
      case "pipeline": { const name = text(r.pipeline_name); return name ? { pipeline_name: name } : null; }
      case "package": {
        const domain = text(r.domain), repository = text(r.repository), prefix = text(r.package_prefix);
        if (!domain || !repository || !prefix) return null;
        const owner = typeof r.domain_owner === "string" ? r.domain_owner.trim() : "";
        if (owner && !/^\d{12}$/.test(owner)) return null;
        return { domain, repository, package_prefix: prefix, ...(owner ? { domain_owner: owner } : {}) };
      }
      case "logs": {
        const group = text(r.log_group);
        if (!group) return null;
        const source = r.source === "lambda" ? "lambda" : "group";
        const fn = source === "lambda" ? text(r.function_name, 256) : "";
        return { log_group: group, source, ...(fn ? { function_name: fn } : {}) };
      }
      case "stack": { const name = text(r.stack_name); return name ? { stack_name: name } : null; }
      case "stacks-search": {
        const prefix = typeof r.name_prefix === "string" && !CONTROL.test(r.name_prefix) ? r.name_prefix.trim().slice(0, 2048) : "";
        const filter = Array.isArray(r.status_filter) ? r.status_filter.map(s => text(s, 128)).filter(Boolean).slice(0, 64) : [];
        return { name_prefix: prefix, ...(filter.length ? { status_filter: filter } : {}) };
      }
      case "lookup": { const query = text(r.query); return query ? { query } : null; }
      case "query": {
        const group = text(r.log_group), query = multiline(r.query, 10240);
        return group && query ? { log_group: group, query, range_seconds: int(r.range_seconds, 60, 604800, 3600) } : null;
      }
      case "errors": {
        const pattern = typeof r.log_group_pattern === "string" && !CONTROL.test(r.log_group_pattern)
          ? r.log_group_pattern.trim().slice(0, 2048) : "/aws/lambda/";
        return { hours: int(r.hours, 1, 168, 24), log_group_pattern: pattern || "/aws/lambda/" };
      }
      case "cli": {
        const command = text(r.command, 16384);
        return /^aws\s+\S/.test(command) ? { command } : null;
      }
      default: return null;
    }
  }

  function normalize(raw) {
    if (!raw || typeof raw !== "object") return null;
    const id = text(raw.id, 256);
    const kind = Object.hasOwn(KINDS, raw.kind) ? raw.kind : null;
    const context = pinned(raw.context);
    const inputs = kind ? inputsFor(kind, raw.inputs) : null;
    if (!id || !kind || !context || !inputs) return null;
    return { id, kind, context, inputs, order: int(raw.order, 0, 100000, 0) };
  }

  function create(kind, context, inputs, existing = []) {
    const group = KINDS[kind].group;
    const order = existing.filter(t => KINDS[t.kind].group === group).reduce((max, t) => Math.max(max, t.order), -1) + 1;
    const random = Math.random().toString(36).slice(2, 8);
    return normalize({ id: `${kind}-${Date.now().toString(36)}-${random}`, kind, context, inputs, order });
  }

  function name(thing) {
    const i = thing.inputs;
    switch (thing.kind) {
      case "pipeline": return i.pipeline_name;
      case "package": return i.package_prefix;
      case "logs": return i.function_name || i.log_group;
      case "stack": return i.stack_name;
      case "stacks-search": return i.name_prefix ? `stacks ${i.name_prefix}` : "all stacks";
      case "lookup": return `lookup ${i.query}`;
      case "query": return `insights ${i.log_group}`;
      case "errors": return `errors ${i.log_group_pattern}`;
      case "cli": return i.command;
      default: return thing.id;
    }
  }

  function rowRequest(thing) {
    const i = thing.inputs;
    switch (thing.kind) {
      case "pipeline": return { widget: "pipeline-runs", inputs: { pipeline_name: i.pipeline_name, max_results: 5 } };
      case "package": return { widget: "codeartifact-packages", inputs: { mode: "list", domain: i.domain, repository: i.repository,
        package_prefix: i.package_prefix, max_packages: 5, ...(i.domain_owner ? { domain_owner: i.domain_owner } : {}) } };
      case "logs": return { widget: i.source === "lambda" ? "log-tail" : "cloudwatch-logs",
        inputs: { mode: "streams", log_group: i.log_group, max_streams: 1 } };
      case "stack": return { widget: "cfn-stack-detail", inputs: { stack_name: i.stack_name } };
      case "stacks-search": return { widget: "cfn-stacks", inputs: { name_prefix: i.name_prefix,
        ...(i.status_filter ? { status_filter: i.status_filter } : {}) } };
      case "lookup": return { widget: "resource-lookup", inputs: { query: i.query, max_results: 25 } };
      case "query": return { widget: "logs-insights", inputs: { mode: "query", log_group: i.log_group, query: i.query, range_seconds: i.range_seconds } };
      case "errors": return { widget: "errors-by-stack", inputs: { hours: i.hours, log_group_pattern: i.log_group_pattern } };
      case "cli": return { widget: "aws-cli", inputs: { command: i.command } };
      default: throw new Error("Unknown thing kind");
    }
  }

  function words(status) {
    const raw = String(status || "").replace(/([a-z0-9])([A-Z])/g, "$1 $2").replace(/[_-]+/g, " ").trim().toLowerCase();
    return raw ? raw.charAt(0).toUpperCase() + raw.slice(1) : "";
  }
  function statusTone(status) {
    const s = String(status || "").replace(/([a-z0-9])([A-Z])/g, "$1_$2").toUpperCase();
    if (s.includes("FAILED") || s.includes("ROLLBACK")) return "fail";
    if (s.includes("IN_PROGRESS") || s.includes("STOPPING") || s === "IN_PROGRESS") return "run";
    if (s.includes("COMPLETE") || s.includes("SUCCEEDED")) return "ok";
    return "muted";
  }
  const time = value => { const t = Date.parse(value); return Number.isFinite(t) ? t : null; };

  function summarize(thing, result) {
    const rows = Array.isArray(result?.rows) ? result.rows : [];
    switch (thing.kind) {
      case "pipeline": {
        if (!rows.length) return { tone: "muted", text: "No runs", at: null };
        return { tone: statusTone(rows[0].status), text: words(rows[0].status), at: time(rows[0].last_updated) };
      }
      case "package": {
        const exact = rows.find(r => r.package === thing.inputs.package_prefix);
        if (exact?.latest_version) return { tone: "muted", text: String(exact.latest_version), at: time(exact.last_published) };
        return { tone: "muted", text: `${rows.length} package${rows.length === 1 ? "" : "s"}`, at: null };
      }
      case "logs": {
        const stream = Array.isArray(result?.streams) ? result.streams[0] : null;
        return stream ? { tone: "muted", text: "last event", at: Number(stream.last_event_timestamp) || null }
          : { tone: "muted", text: "No streams", at: null };
      }
      case "stack": {
        const events = Array.isArray(result?.events) ? result.events : [];
        const own = events.find(e => e.logical_id === thing.inputs.stack_name);
        if (own) return { tone: statusTone(own.status), text: words(own.status), at: time(own.time) };
        const count = Array.isArray(result?.resources) ? result.resources.length : 0;
        return { tone: "muted", text: `${count} resource${count === 1 ? "" : "s"}`, at: null };
      }
      case "stacks-search": return { tone: "muted", text: `${rows.length} stack${rows.length === 1 ? "" : "s"}`, at: null };
      case "lookup": {
        const n = Array.isArray(result?.matches) ? result.matches.length : 0;
        return { tone: "muted", text: `${n} match${n === 1 ? "" : "es"}`, at: null };
      }
      case "errors": {
        // Failed or partial counts never say "no errors": absent rows prove nothing.
        const incomplete = result?.partial === true || result?.status === "partial" || result?.status === "failed"
          || !!result?.error || result?.coverage?.completeness === "unknown";
        if (!rows.length) return incomplete ? { tone: "warn", text: "counts incomplete", at: null } : { tone: "muted", text: "No errors returned", at: null };
        const top = rows.reduce((a, b) => (Number(b.errors) || 0) > (Number(a.errors) || 0) ? b : a);
        return { tone: "fail", text: `${top.stack}: ${Number(top.errors) || 0}`, at: null };
      }
      default: return { tone: "muted", text: `${rows.length} row${rows.length === 1 ? "" : "s"}`, at: null };
    }
  }

  function groupThings(things) {
    return GROUPS.map(group => ({ ...group, things: things.filter(t => KINDS[t.kind].group === group.key)
      .slice().sort((a, b) => a.order - b.order || a.id.localeCompare(b.id)) }))
      .filter(group => group.things.length);
  }

  function move(things, id, delta) {
    const target = things.find(t => t.id === id);
    if (!target) return things;
    const peers = things.filter(t => KINDS[t.kind].group === KINDS[target.kind].group)
      .slice().sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
    const index = peers.indexOf(target), next = index + delta;
    if (next < 0 || next >= peers.length) return things;
    [peers[index], peers[next]] = [peers[next], peers[index]];
    const orders = new Map(peers.map((t, n) => [t.id, n]));
    return things.map(t => orders.has(t.id) ? { ...t, order: orders.get(t.id) } : t);
  }

  function toStored(things) {
    return things.map(t => ({ id: t.id, kind: t.kind, context: { ...t.context }, inputs: { ...t.inputs }, order: t.order }));
  }

  function migrateTiles(tiles, env = {}) {
    const things = [];
    const seen = new Set();
    let converted = 0, skippedNoAccount = 0, skippedEmpty = 0;
    const profileContext = name => {
      const p = (env.profiles || []).find(x => x.name === name);
      if (!p) return null;
      const region = (env.allowedRegions || []).includes(p.region) ? p.region : env.defaultRegion;
      return region ? { mode: "pinned", profile: p.name, account_id: p.account_id, region } : null;
    };
    const add = (kind, context, inputs) => {
      const probe = normalize({ id: "probe", kind, context, inputs, order: 0 });
      if (!probe) return false;
      const key = JSON.stringify([kind, probe.context, probe.inputs]);
      if (!seen.has(key)) { seen.add(key); things.push(create(kind, probe.context, probe.inputs, things)); }
      return true;
    };
    for (const tile of Array.isArray(tiles) ? tiles : []) {
      if (!tile || typeof tile !== "object") continue;
      const widget = String(tile.widget || String(tile.id || "").split(":")[0]);
      const config = tile.config && typeof tile.config === "object" ? tile.config : {};
      const inputs = config.inputs && typeof config.inputs === "object" ? config.inputs : {};
      const raw = config.context || config.account_override;
      const own = raw && (raw.mode === "pinned" || !raw.mode && raw.profile) ? pinned({ ...raw, mode: "pinned" }) : null;
      const tileContext = own || (env.defaultProfile ? profileContext(env.defaultProfile) : null);
      let made = 0, needsAccount = false;
      const want = (kind, context, values) => {
        if (!context) { needsAccount = true; return; }
        if (add(kind, context, values)) made++;
      };
      const pins = key => Array.isArray(inputs[key]) ? inputs[key] : [];
      switch (widget) {
        case "pipeline-runs": pins("pinned_pipelines").forEach(pin => want("pipeline", pinned({ ...pin, mode: "pinned" }), { pipeline_name: pin.pipeline_name })); break;
        case "aws-cli":
          pins("pinned_cli_commands").forEach(pin => want("cli", pinned({ ...pin, mode: "pinned" }), { command: pin.command }));
          if (inputs.command) want("cli", tileContext, { command: inputs.command });
          break;
        case "codeartifact-packages": if (inputs.domain && inputs.repository && inputs.package_prefix) want("package", tileContext, inputs); break;
        case "logs-insights": if (inputs.log_group && inputs.query) want("query", tileContext, inputs); break;
        case "errors-by-stack": want("errors", tileContext, inputs); break;
        case "cfn-stacks": if (inputs.name_prefix || Array.isArray(inputs.status_filter) && inputs.status_filter.length) want("stacks-search", tileContext, inputs); break;
        case "resource-lookup": if (inputs.query) want("lookup", tileContext, inputs); break;
        case "log-tail": case "cloudwatch-logs":
          if (inputs.log_group) want("logs", tileContext, { log_group: inputs.log_group, source: widget === "log-tail" ? "lambda" : "group" });
          break;
        default: break;
      }
      if (made) converted++; else if (needsAccount) skippedNoAccount++; else skippedEmpty++;
    }
    return { things, converted, skippedNoAccount, skippedEmpty };
  }

  root.CloudBurritoThings = { GROUPS, KINDS, inputsFor, normalize, create, name, rowRequest, migrateTiles, summarize, statusTone,
    groupThings, move, toStored, words };
})(typeof window !== "undefined" ? window : globalThis);
