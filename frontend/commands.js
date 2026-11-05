// Pure parser and completion for the ":" command line. No DOM or bridge.
// Every thing command resolves one pinned profile, account and region.
(function (root) {
  "use strict";
  const THING_COMMANDS = { pipe: "pipeline", pkg: "package", logs: "logs", stack: "stack", stacks: "stacks-search",
    lookup: "lookup", errors: "errors", insights: "query", aws: "cli" };
  const APP_COMMANDS = ["add", "settings", "policy", "audit", "theme", "help", "classic"];
  const RANGES = { "15m": 900, "1h": 3600, "3h": 10800, "12h": 43200, "24h": 86400, "3d": 259200, "7d": 604800 };
  const MISSING = {
    pipeline: "Add a pipeline name, for example :pipe payments-api.",
    package: "Add a package name and its domain/repository.",
    logs: "Add a Lambda function name, or a log group that starts with /.",
    stack: "Add a stack name.",
    "stacks-search": "This stack search could not be read.",
    lookup: "Add a resource name, ARN or partial ID.",
    errors: "This error count could not be read.",
    query: "Add a log group and a quoted query, for example :insights /aws/lambda/fn \"fields @message | limit 20\".",
    cli: "Write a full command after :aws, for example :aws sts get-caller-identity.",
  };
  const HELP = [
    [":pipe <name> @profile", "Pipeline runs"],
    [":pkg <name> <domain>/<repository> @profile", "Package versions"],
    [":logs <function or /log/group> @profile", "Log streams and events"],
    [":stack <name> @profile", "Stack resources and events"],
    [":stacks [prefix] @profile", "Search stacks"],
    [":lookup <text> @profile", "Find tagged resources"],
    [":errors [pattern] [hours] @profile", "Error counts, runs Logs Insights and can cost money"],
    [":insights <log group> \"<query>\" [1h] @profile", "Logs Insights query, can cost money"],
    [":aws <command> @profile", "Reviewed read-only AWS CLI command"],
    [":add  :settings  :policy  :audit  :theme  :help", "App commands"],
  ];

  function tokenize(text) {
    const tokens = [];
    let current = "", quote = null, started = false;
    for (const ch of text) {
      if (quote) { if (ch === quote) quote = null; else current += ch; continue; }
      if (ch === '"' || ch === "'") { quote = ch; started = true; continue; }
      if (/\s/.test(ch)) { if (started) { tokens.push(current); current = ""; started = false; } continue; }
      current += ch;
      started = true;
    }
    if (quote) return null;
    if (started) tokens.push(current);
    return tokens;
  }

  function parse(input, env) {
    const text = String(input || "").replace(/^:/, "").trim();
    if (!text) return { ok: false, error: "Type a command, for example pipe payments-api @prod-core." };
    if (text.length > 16384 || /[\u0000-\u001f\u007f]/u.test(text)) return { ok: false, error: "The command is too long or contains control characters." };
    const name = text.split(/\s+/)[0].toLowerCase();
    if (APP_COMMANDS.includes(name)) return { ok: true, type: "app", name };
    const kind = THING_COMMANDS[name];
    if (!kind) return { ok: false, error: `Unknown command ${name}. Type :help to see all commands.` };
    let profileName = null, region = null, command = "";
    const kept = [];
    if (kind === "cli") {
      // The AWS text is passed through as typed. Only a trailing unquoted @profile is ours.
      const trailing = text.match(/\s+@([^\s"'\\]+)$/);
      profileName = trailing ? trailing[1] : null;
      command = trailing ? text.slice(0, trailing.index).trim() : text;
    } else {
      const tokens = tokenize(text.slice(name.length).trim());
      if (!tokens) return { ok: false, error: "A quote is not closed." };
      for (const token of tokens) {
        if (token.length > 1 && token.startsWith("@")) { profileName = token.slice(1); continue; }
        if ((env.allowedRegions || []).includes(token)) { region = token; continue; }
        kept.push(token);
      }
    }
    profileName = profileName || env.defaultProfile || null;
    if (!profileName) return { ok: false, error: "Add @profile, for example @prod-core." };
    const profile = (env.profiles || []).find(p => p.name === profileName);
    if (!profile) return { ok: false, error: `Unknown profile @${profileName}. Type @ and press Tab to see your profiles.` };
    region = region || ((env.allowedRegions || []).includes(profile.region) ? profile.region : env.defaultRegion);
    if (!region) return { ok: false, error: "Add a supported region, for example eu-west-1." };
    const context = { mode: "pinned", profile: profile.name, account_id: profile.account_id, region };
    let raw;
    switch (kind) {
      case "pipeline": raw = { pipeline_name: kept.join(" ") }; break;
      case "package": {
        const [pkg, repo] = kept;
        let domain = "", repository = "";
        if (repo && repo.includes("/")) [domain, repository] = repo.split("/", 2);
        else { const known = (env.packageRepos || []).find(r => r.profile === profile.name); if (known) ({ domain, repository } = known); }
        if (!pkg || !domain || !repository) return { ok: false, error: pkg ? "Add the repository as domain/repository, for example acme/pypi-internal." : MISSING.package };
        raw = { package_prefix: pkg, domain, repository };
        break;
      }
      case "logs": {
        const target = kept.join(" ");
        raw = target.startsWith("/") ? { source: "group", log_group: target }
          : { source: "lambda", function_name: target, log_group: target ? `/aws/lambda/${target}` : "" };
        break;
      }
      case "stack": raw = { stack_name: kept.join(" ") }; break;
      case "stacks-search": raw = { name_prefix: kept.join(" ") }; break;
      case "lookup": raw = { query: kept.join(" ") }; break;
      case "errors": {
        const hours = kept.find(t => /^\d+$/.test(t));
        raw = { log_group_pattern: kept.find(t => !/^\d+$/.test(t)) || "/aws/lambda/", hours: hours ? Number(hours) : 24 };
        break;
      }
      case "query": {
        const [group, query, range] = kept;
        if (range && !RANGES[range]) return { ok: false, error: "Use a range of 15m, 1h, 3h, 12h, 24h, 3d or 7d." };
        raw = { log_group: group, query, range_seconds: RANGES[range] || 3600 };
        break;
      }
      case "cli": raw = { command: command.length > name.length ? command : "" }; break;
    }
    const inputs = root.CloudBurritoThings.inputsFor(kind, raw);
    return inputs ? { ok: true, type: "thing", kind, context, inputs } : { ok: false, error: MISSING[kind] };
  }

  function complete(input, env) {
    const text = String(input || "");
    const word = (text.match(/(\S*)$/) || ["", ""])[1];
    const from = text.length - word.length;
    const head = text.replace(/^:/, "").trimStart();
    let items;
    if (word.startsWith("@")) {
      items = (env.profiles || []).filter(p => p.name.startsWith(word.slice(1)))
        .map(p => ({ value: `@${p.name}`, label: `@${p.name}`, detail: p.account_id }));
    } else if (!/\s/.test(head)) {
      const prefix = word.startsWith(":") ? ":" : "";
      items = [...Object.keys(THING_COMMANDS), ...APP_COMMANDS].filter(c => c.startsWith(word.replace(/^:/, "")))
        .map(c => ({ value: `${prefix}${c} `, label: c, detail: "" }));
    } else {
      const names = (env.names || {})[THING_COMMANDS[head.split(/\s+/)[0].toLowerCase()]] || [];
      items = names.filter(n => n.startsWith(word)).map(n => ({ value: n, label: n, detail: "" }));
    }
    return { from, to: text.length, items: items.slice(0, 8) };
  }

  root.CloudBurritoCommands = { parse, complete, tokenize, HELP, THING_COMMANDS, APP_COMMANDS, RANGES };
})(typeof window !== "undefined" ? window : globalThis);
