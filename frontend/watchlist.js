// Cloud Burrito watchlist: specific things from many accounts, loaded when the
// user looks. Every request is pinned to its thing's own profile, account and
// region; there is no inherited account on this page.
(function () {
  "use strict";
  const core = window.CloudBurritoCore;
  const T = window.CloudBurritoThings;
  const { $, el, clear } = core;
  const keymap = window.CloudBurritoKeys.createKeyMap();
  const SIGN_IN_TYPES = ["CredentialsError", "CredentialsExpired", "IdentityVerificationFailed"];

  const state = {
    things: [], profiles: [], profilesKnown: false, allowedRegions: ["eu-west-1", "us-east-1"], settings: null,
    selectedId: null, filter: "", focus: "list", help: false, rows: new Map(), loadedAt: null, saveSeq: 0, storageReady: false,
  };
  const actions = {};
  let inspector = null;
  let noticeTimer = null;

  const recoveries = new Map();
  let cliAvailable = null;
  const isQuery = (widget, inputs) => widget === "errors-by-stack" || widget === "logs-insights" && inputs?.mode === "query";
  const recovery = {
    key: (widget, inputs, context) => JSON.stringify([context?.profile, context?.account_id, context?.region, widget,
      inputs.log_group || "", inputs.query || "", inputs.range_seconds || 0, inputs.log_group_pattern || "", inputs.hours || 0]),
    get: key => recoveries.get(key) || null,
  };
  const validPending = list => Array.isArray(list) && list.length <= 2 ? list.filter(p => p && /^q-\d{1,24}$/.test(p.token)
    && typeof p.group === "string" && p.group.length <= 512 && !/[\u0000-\u001f\u007f]/u.test(p.group)
    && Number.isSafeInteger(p.window_seconds) && p.window_seconds > 0 && p.window_seconds <= 604800
    && typeof p.query_digest === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(p.query_digest)) : [];

  core.configure({
    contextFor: owner => core.contextOverrideFromElement(owner) || { mode: "inherit", profile: null, account_id: null, region: null },
    isIndependent: () => true,
    requestsAllowed: () => state.storageReady,
    resultTitle: host => host.closest?.("[data-result-title]")?.dataset.resultTitle || "Result",
    async beforeFetch({ widgetName, inputs, request }) {
      if (widgetName === "aws-cli") {
        if (cliAvailable !== true) {
          const info = await call("cli_availability");
          cliAvailable = info?.available !== false;
        }
        if (!request.current()) throw new Error("Request owner is no longer current.");
        if (!cliAvailable) { cliAvailable = null; throw new Error("AWS CLI v2 was not found. Install it, then press r."); }
      }
      if (isQuery(widgetName, inputs) && recoveries.has(recovery.key(widgetName, inputs, request.context)) && inputs.acknowledge_unknown !== true) {
        throw new Error("An earlier query may still be running. Review it before running again.");
      }
    },
    afterFetch({ widgetName, inputs, request, result }) {
      if (!isQuery(widgetName, inputs) || !request.current() || !request.accept(result)) return;
      const key = recovery.key(widgetName, inputs, request.context);
      const cleanup = result?.cleanup || result?.data?.cleanup;
      if (result?.recovery_required === true || cleanup?.recovery_required === true || cleanup?.remote_queries_may_still_run === true
          || result?.query_state === "recovery_mismatch") {
        recoveries.set(key, { pending: validPending(result.recovery_pending), canAcknowledge: result.can_acknowledge_unknown !== false });
      } else if (inputs.acknowledge_unknown === true && (cleanup?.remote_stop_confirmed === true
          || result && result.ok !== false && !result.error && result.render !== "permission_denied")) {
        recoveries.delete(key);
      }
    },
    browserInvoke: !core.isTauri && window.CloudBurritoDemo ? window.CloudBurritoDemo.invoke : null,
  });

  const call = (command, params) => core.invoke(command, params ? { params } : undefined);

  function notice(message) {
    const box = $("#wl-notice");
    box.textContent = message;
    box.hidden = false;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => { box.hidden = true; }, 6000);
  }

  function age(at) {
    const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
    if (seconds < 60) return "now";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.round(minutes / 60);
    return hours < 48 ? `${hours}h` : `${Math.round(hours / 24)}d`;
  }

  function accountProblem(thing) {
    if (!state.profilesKnown) return null;
    const profile = state.profiles.find(p => p.name === thing.context.profile);
    if (!profile) return "profile is not in your AWS config";
    if (profile.account_id !== thing.context.account_id) return "profile now points to another account";
    return null;
  }

  const rowEl = id => id ? document.querySelector(`#wl-list .wl-row[data-thing-id="${CSS.escape(id)}"]`) : null;
  const selectedThing = () => state.things.find(t => t.id === state.selectedId) || null;
  const visibleRows = () => Array.from(document.querySelectorAll("#wl-list .wl-row:not([hidden])"));

  function buildRow(thing) {
    const kind = T.KINDS[thing.kind];
    const owner = el("div", { class: "wl-row-owner", hidden: true });
    const detailOwner = el("div", { class: "wl-row-owner-detail", hidden: true });
    owner._widgetContextOverride = thing.context;
    detailOwner._widgetContextOverride = thing.context;
    const row = el("li", { class: "wl-row", role: "option", tabindex: "-1", "aria-selected": "false", "data-thing-id": thing.id },
      el("span", { class: `wl-kind wl-kind-${thing.kind}`, title: kind.title, "aria-hidden": "true" }, kind.badge),
      el("span", { class: "wl-row-name", title: T.name(thing) }, T.name(thing)),
      el("span", { class: "wl-acct", title: `${thing.context.profile}, account ${thing.context.account_id}, ${thing.context.region}` }, thing.context.profile),
      el("span", { class: "wl-row-state", "data-tone": "muted" }),
      owner, detailOwner);
    row._thing = thing;
    row.addEventListener("click", () => { select(thing.id); openSelected(); });
    return row;
  }

  function emptyState() {
    return el("div", { class: "wl-empty" },
      el("h2", {}, "Add your first thing"),
      el("p", {}, "Press a to add a pipeline, package, log group or stack from any of your accounts."),
      el("button", { type: "button", class: "btn", onclick: () => actions.add?.() }, "Add a thing"));
  }

  function renderList() {
    const host = $("#wl-list");
    const existing = new Map(Array.from(host.querySelectorAll(".wl-row")).map(node => [node.dataset.thingId, node]));
    const fragment = document.createDocumentFragment();
    if (!state.things.length) fragment.appendChild(emptyState());
    for (const group of T.groupThings(state.things)) {
      const list = el("ul", { class: "wl-rows", role: "listbox", "aria-label": group.label });
      for (const thing of group.things) {
        const node = existing.get(thing.id);
        // Keep a row (and its in-flight load) when only the order changed.
        const same = node && JSON.stringify([node._thing.kind, node._thing.context, node._thing.inputs])
          === JSON.stringify([thing.kind, thing.context, thing.inputs]);
        if (same) { node._thing = thing; existing.delete(thing.id); }
        list.appendChild(same ? node : buildRow(thing));
      }
      fragment.append(el("h2", { class: "wl-group", "data-group": group.key }, group.label, el("span", {}, String(group.things.length))), list);
    }
    for (const stale of existing.values()) core.invalidateRequests(stale);
    host.replaceChildren(fragment);
    for (const thing of state.things) paintRow(thing);
    applyFilter(state.filter);
    select(state.selectedId && rowEl(state.selectedId) ? state.selectedId : visibleRows()[0]?.dataset.thingId || null, { focus: false });
    renderSummary();
  }

  function applyFilter(text) {
    state.filter = text;
    const query = text.trim().toLowerCase();
    for (const node of document.querySelectorAll("#wl-list .wl-row")) {
      const t = node._thing;
      node.hidden = !!query && !`${T.name(t)} ${t.context.profile} ${T.KINDS[t.kind].title}`.toLowerCase().includes(query);
    }
    for (const heading of document.querySelectorAll("#wl-list .wl-group")) {
      const list = heading.nextElementSibling;
      heading.hidden = list.hidden = !list.querySelector(".wl-row:not([hidden])");
    }
    if (state.selectedId && rowEl(state.selectedId)?.hidden) select(visibleRows()[0]?.dataset.thingId || null, { focus: false });
  }

  function renderSummary() {
    const accounts = new Set(state.things.map(t => `${t.context.profile}|${t.context.account_id}`)).size;
    const count = state.things.length;
    const loaded = state.loadedAt ? `, loaded ${new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit" }).format(state.loadedAt)}` : "";
    $("#wl-summary").textContent = count ? `${count} thing${count === 1 ? "" : "s"} in ${accounts} account${accounts === 1 ? "" : "s"}${loaded}` : "";
  }

  function paintRow(thing) {
    const row = rowEl(thing.id);
    if (!row) return;
    const info = state.rows.get(thing.id) || {};
    const cell = row.querySelector(".wl-row-state");
    clear(cell);
    cell.dataset.tone = info.tone || info.summary?.tone || "muted";
    cell.append(el("span", { class: "wl-state-text" }, info.label ?? info.summary?.text ?? ""));
    if (info.stale) cell.append(el("span", { class: "wl-state-flag", title: info.error || "" }, "stale"));
    else if (info.partial) cell.append(el("span", { class: "wl-state-flag" }, "partial"));
    const at = info.summary?.at ?? info.receivedAt ?? null;
    if (at && !info.label) cell.append(el("time", { class: "wl-age", datetime: new Date(at).toISOString() }, age(at)));
  }

  function setRow(thing, info) {
    state.rows.set(thing.id, info);
    paintRow(thing);
  }

  async function enrichPackage(thing, row, list) {
    const rows = Array.isArray(list?.rows) ? list.rows : [];
    const exact = rows.find(r => r.package === thing.inputs.package_prefix);
    const targets = (exact ? [exact] : rows).filter(r => r.enrichment_state !== "complete").slice(0, 5).map(r => r.package);
    if (!targets.length || core.resultFailure(list)) return list;
    const owner = row.querySelector(".wl-row-owner-detail");
    const request = core.beginOwnedRequest(owner, thing.context);
    const i = thing.inputs;
    const enriched = await core.fetchWidgetData("codeartifact-packages", { mode: "enrich", domain: i.domain, repository: i.repository,
      package_prefix: i.package_prefix, ...(i.domain_owner ? { domain_owner: i.domain_owner } : {}), packages: targets },
      owner, thing.context, request);
    if (!request.accept(enriched)) return null;
    const byName = new Map((Array.isArray(enriched.rows) ? enriched.rows : []).map(r => [r.package, r]));
    return { ...list, rows: rows.map(r => byName.get(r.package) || r), ...(core.resultFailure(enriched) ? { partial: true } : {}) };
  }

  function applyRowResult(thing, result) {
    const previous = state.rows.get(thing.id) || {};
    const outcome = core.resultState(result);
    if (outcome === "denied") return setRow(thing, { label: `blocked by policy: ${result.action || "request"}`, tone: "fail" });
    if (SIGN_IN_TYPES.includes(result?.error_type) || result?.needs_sso_login === true || outcome === "expired") {
      return setRow(thing, { label: "sign in needed", tone: "warn", copy: `aws sso login --profile ${thing.context.profile}` });
    }
    if (outcome === "failed" || outcome === "cancelled") {
      const message = outcome === "cancelled" ? "cancelled" : "failed, press r to retry";
      if (previous.summary && !previous.label) return setRow(thing, { ...previous, stale: true, error: message });
      return setRow(thing, { label: message, tone: outcome === "cancelled" ? "muted" : "fail" });
    }
    setRow(thing, { summary: T.summarize(thing, result), partial: outcome === "partial" || outcome === "limited", receivedAt: Date.now(),
      copy: thing.kind === "package" ? T.summarize(thing, result).text : null });
  }

  function applyRowFailure(thing, message) {
    const previous = state.rows.get(thing.id) || {};
    if (previous.summary && !previous.label) return setRow(thing, { ...previous, stale: true, error: message });
    setRow(thing, { label: "failed, press r to retry", tone: "fail" });
  }

  async function loadRow(thing, { run = false } = {}) {
    const row = rowEl(thing.id);
    if (!row) return;
    const problem = accountProblem(thing);
    if (problem) return setRow(thing, { label: `needs account: ${problem}`, tone: "warn" });
    const previous = state.rows.get(thing.id) || {};
    if (!T.KINDS[thing.kind].autoload && !run) {
      return setRow(thing, previous.summary ? previous : { label: "not run yet", tone: "muted" });
    }
    const owner = row.querySelector(".wl-row-owner");
    const request = core.beginOwnedRequest(owner, thing.context);
    row.dataset.loading = "true";
    if (!previous.summary) setRow(thing, { label: "loading", tone: "muted" });
    try {
      const { widget, inputs } = T.rowRequest(thing);
      let result = await core.fetchWidgetData(widget, inputs, owner, thing.context, request);
      if (!request.accept(result)) return;
      if (thing.kind === "package") result = await enrichPackage(thing, row, result);
      if (result) applyRowResult(thing, result);
    } catch (error) {
      if (request.current()) applyRowFailure(thing, String(error?.message || error));
    } finally {
      row.dataset.loading = "false";
    }
  }

  function refreshAll() {
    state.loadedAt = Date.now();
    renderSummary();
    for (const group of T.groupThings(state.things)) for (const thing of group.things) loadRow(thing);
  }

  async function save() {
    const seq = ++state.saveSeq;
    try {
      const result = await call("watchlist_set", { things: T.toStored(state.things) });
      if (seq === state.saveSeq && result?._storage?.status !== "saved") throw new Error("not saved");
    } catch (_) {
      if (seq === state.saveSeq) notice("Changes could not be saved. The file on disk was not changed. Try again.");
    }
  }

  function select(id, { focus = true } = {}) {
    state.selectedId = id;
    for (const node of document.querySelectorAll("#wl-list .wl-row")) {
      const on = node.dataset.thingId === id;
      node.setAttribute("aria-selected", String(on));
      node.tabIndex = on ? 0 : -1;
    }
    if (focus && state.focus === "list") rowEl(id)?.focus({ preventScroll: false });
    window.CloudBurritoWatchlist?.onSelect?.(selectedThing());
  }

  function moveSelection(to) {
    const rows = visibleRows();
    if (!rows.length) return;
    const index = rows.findIndex(node => node.dataset.thingId === state.selectedId);
    const next = to === "top" ? 0 : to === "bottom" ? rows.length - 1 : Math.max(0, Math.min(rows.length - 1, index + to));
    select(rows[next].dataset.thingId);
  }

  function setFocus(area) {
    state.focus = area;
    document.body.dataset.focus = area;
    if (area === "list") {
      const node = rowEl(state.selectedId);
      if (node) node.focus({ preventScroll: true }); else $("#wl-list").focus();
    } else inspector.focus();
    renderKeys();
  }

  function openSelected() {
    const thing = selectedThing();
    if (!thing) return;
    inspector.show(thing);
    setFocus("inspector");
  }

  async function copyMain() {
    const text = state.focus === "inspector" ? inspector.copyMain() : state.rows.get(state.selectedId)?.copy || null;
    if (!text) { notice("Nothing to copy here."); return; }
    await core.copyToClipboard(String(text), null);
    notice(`Copied ${String(text).slice(0, 80)}`);
  }

  function removeSelected() {
    const thing = selectedThing();
    if (!thing) return;
    const rows = visibleRows();
    const index = rows.findIndex(node => node.dataset.thingId === thing.id);
    core.invalidateRequests(rowEl(thing.id));
    state.things = state.things.filter(t => t.id !== thing.id);
    state.rows.delete(thing.id);
    if (inspector.current()?.thing.id === thing.id) inspector.clear();
    state.selectedId = rows[index + 1]?.dataset.thingId || rows[index - 1]?.dataset.thingId || null;
    renderList();
    rowEl(state.selectedId)?.focus();
    notice(`Removed ${T.name(thing)}.`);
    save();
  }

  function moveThing(delta) {
    const thing = selectedThing();
    if (!thing) return;
    state.things = T.move(state.things, thing.id, delta);
    renderList();
    rowEl(thing.id)?.focus();
    save();
  }

  const KEYBAR = {
    list: [["j k", "Move"], ["Enter", "Open"], ["/", "Filter"], ["r", "Refresh"], ["R", "Refresh all"], ["a", "Add"], ["e", "Edit"], ["dd", "Remove"], ["?", "All keys"]],
    inspector: [["j k", "Move"], ["Enter", "Open"], ["Esc", "Back"], ["y", "Copy"], ["o", "Console"], ["n", "Next error"], ["r", "Refresh"], ["?", "All keys"]],
  };
  function renderKeys() {
    const host = $("#wl-keys");
    clear(host);
    for (const [keys, label] of (state.help ? window.CloudBurritoKeys.HELP : KEYBAR[state.focus])) {
      host.append(el("span", { class: "wl-key" }, ...keys.split(" ").map(k => el("kbd", {}, k)), label));
    }
  }

  const form = {
    FIELDS: {
      pipeline: [{ key: "pipeline_name", label: "Pipeline name", list: "pipelines" }],
      package: [{ key: "domain", label: "Domain" }, { key: "repository", label: "Repository" },
        { key: "package_prefix", label: "Package name or prefix" },
        { key: "domain_owner", label: "Domain owner account (optional)", optional: true }],
      logs: [{ key: "source", label: "Source", choices: [["lambda", "Lambda function"], ["group", "Log group"]] },
        { key: "name", label: "Function name or log group" }],
      stack: [{ key: "stack_name", label: "Stack name" }],
      "stacks-search": [{ key: "name_prefix", label: "Stack name prefix (optional)", optional: true }],
      lookup: [{ key: "query", label: "Resource name, ARN or partial ID" }],
      query: [{ key: "log_group", label: "Log group" }, { key: "query", label: "Query", multiline: true },
        { key: "range_seconds", label: "Time range", choices: [["900", "Last 15 minutes"], ["3600", "Last hour"], ["10800", "Last 3 hours"],
          ["43200", "Last 12 hours"], ["86400", "Last 24 hours"], ["259200", "Last 3 days"], ["604800", "Last 7 days"]] }],
      errors: [{ key: "log_group_pattern", label: "Log group names contain" }, { key: "hours", label: "Hours, 1 to 168" }],
      cli: [{ key: "command", label: "AWS CLI command" }],
    },
    defaults: {
      query: { range_seconds: "3600", query: "fields @timestamp, @message | sort @timestamp desc | limit 20" },
      errors: { log_group_pattern: "/aws/lambda/", hours: "24" },
    },
    toInputs: {
      logs: v => v.source === "lambda"
        ? { source: "lambda", function_name: v.name, log_group: v.name ? `/aws/lambda/${v.name}` : "" }
        : { source: "group", log_group: v.name },
    },
    fromInputs: {
      logs: i => ({ source: i.source, name: i.source === "lambda" ? i.function_name || i.log_group : i.log_group }),
    },
    notes: {
      query: "Runs only when you press r or Enter. Logs Insights queries can cost money.",
      errors: "Runs only when you press r or Enter. Error counts use Logs Insights queries, which can cost money.",
      cli: "Runs only when you press r or Enter. Only reviewed read-only AWS CLI commands are accepted.",
    },
  };
  let formThing = null;

  function fieldControl(field, value) {
    const id = `wl-field-${field.key}`;
    let control;
    if (field.choices) control = el("select", { id, name: field.key }, ...field.choices.map(([v, label]) => el("option", { value: v }, label)));
    else if (field.multiline) control = el("textarea", { id, name: field.key, spellcheck: "false" });
    else control = el("input", { id, name: field.key, type: "text", autocomplete: "off", spellcheck: "false", ...(field.list ? { list: "wl-form-list" } : {}) });
    if (value !== undefined && value !== null) control.value = String(value);
    const label = el("label", { class: "wl-field", for: id }, field.label, control);
    if (!field.list) return label;
    const listButton = el("button", { type: "button", class: "btn btn-ghost", onclick: listPipelines }, "List pipelines");
    return el("div", { class: "wl-field-row" }, label, listButton, el("datalist", { id: "wl-form-list" }));
  }

  function formContext() {
    const profile = state.profiles.find(p => p.name === $("#wl-form-profile").value);
    const region = $("#wl-form-region").value;
    return profile && region && state.allowedRegions.includes(region) ? { mode: "pinned", profile: profile.name, account_id: profile.account_id, region } : null;
  }

  async function listPipelines() {
    const context = formContext();
    const owner = $("#wl-form .wl-form-owner");
    if (!context) return;
    owner._widgetContextOverride = context;
    const request = core.beginOwnedRequest(owner, context);
    try {
      const result = await core.invokeOwnedRequest(request, "aws_list_pipelines", { context });
      if (!request.accept(result)) return;
      const names = (Array.isArray(result.pipelines) ? result.pipelines : []).map(p => p.name).filter(n => typeof n === "string");
      $("#wl-form-list")?.replaceChildren(...names.map(name => el("option", { value: name })));
      if (!names.length) showFormError(result.error || "No pipelines were returned for this account.");
    } catch (error) { if (request.current()) showFormError(String(error?.message || error)); }
  }

  // Drop any pending or shown pipeline listing: it belongs to an older choice.
  function resetListing() {
    core.invalidateRequests($("#wl-form .wl-form-owner"));
    $("#wl-form-list")?.replaceChildren();
  }

  function updateAccountNote() {
    const note = $("#wl-form-account-note");
    const profile = state.profiles.find(p => p.name === $("#wl-form-profile").value);
    const mismatch = formThing && profile && profile.account_id !== formThing.context.account_id;
    note.textContent = mismatch ? `This profile now points to account ${profile.account_id}. Saving pins this thing to it.` : "";
    note.hidden = !mismatch;
  }

  function showFormError(message) {
    const node = $("#wl-form-error");
    node.textContent = message || "";
    node.hidden = !message;
  }

  function renderFormFields(kind, inputs) {
    const values = inputs ? (form.fromInputs[kind] ? form.fromInputs[kind](inputs) : inputs) : (form.defaults[kind] || {});
    const host = $("#wl-form-fields");
    host.replaceChildren(...(form.notes[kind] ? [el("p", { class: "wl-form-note" }, form.notes[kind])] : []),
      ...form.FIELDS[kind].map(field => fieldControl(field, values[field.key])));
  }

  function fillRegions(preferred, keepUnsupported = false) {
    const select = $("#wl-form-region");
    select.replaceChildren(...state.allowedRegions.map(region => el("option", { value: region }, region)));
    if (keepUnsupported && preferred && !state.allowedRegions.includes(preferred)) {
      select.append(el("option", { value: preferred }, `${preferred} (not supported)`));
      select.value = preferred;
      return;
    }
    select.value = state.allowedRegions.includes(preferred) ? preferred
      : state.allowedRegions.includes(state.settings?.default_region) ? state.settings.default_region : state.allowedRegions[0];
  }

  function openForm(thing, preset = null) {
    formThing = thing;
    const source = thing || preset;
    const dialog = $("#wl-form");
    $("#wl-form-title").textContent = thing ? "Edit thing" : "Add a thing";
    showFormError("");
    const kindSelect = $("#wl-form-kind");
    kindSelect.replaceChildren(...Object.keys(form.FIELDS).map(kind => el("option", { value: kind }, T.KINDS[kind].title)));
    kindSelect.value = source?.kind && form.FIELDS[source.kind] ? source.kind : "pipeline";
    kindSelect.disabled = !!thing;
    const profileSelect = $("#wl-form-profile");
    profileSelect.replaceChildren(...state.profiles.map(p => el("option", { value: p.name }, `${p.name} (${p.account_id})`)));
    const profile = source?.context?.profile || state.settings?.default_profile;
    if (thing && profile && !state.profiles.some(p => p.name === profile)) {
      profileSelect.append(el("option", { value: profile, "data-missing": "true" }, `${profile} (not in your AWS config)`));
    }
    profileSelect.value = Array.from(profileSelect.options).some(o => o.value === profile) ? profile : state.profiles[0]?.name || "";
    fillRegions(source?.context?.region || state.profiles.find(p => p.name === profileSelect.value)?.region, !!thing);
    resetListing();
    updateAccountNote();
    renderFormFields(kindSelect.value, source?.inputs || null);
    $("#wl-form-save").disabled = !state.profiles.length && !thing;
    if (!state.profiles.length) showFormError("No supported SSO profiles were found. Check the AWS config path in Settings, then try again.");
    dialog.showModal();
    (dialog.querySelector("#wl-form-fields input, #wl-form-fields textarea") || kindSelect).focus();
  }

  function submitForm(event) {
    event.preventDefault();
    const kind = $("#wl-form-kind").value;
    if ($("#wl-form-profile").selectedOptions[0]?.dataset.missing) { showFormError("Choose a profile from your AWS config."); return; }
    if (!state.allowedRegions.includes($("#wl-form-region").value)) { showFormError("Choose a supported region."); return; }
    if (!formThing && state.things.length >= 200) { showFormError("The watchlist is full (200 things). Remove one, then try again."); return; }
    const context = formContext();
    const values = Object.fromEntries(form.FIELDS[kind].map(field => [field.key, $(`#wl-field-${field.key}`)?.value ?? ""]));
    const raw = form.toInputs[kind] ? form.toInputs[kind](values) : values;
    const inputs = context ? T.inputsFor(kind, raw) : null;
    const required = form.FIELDS[kind].filter(field => !field.optional && !field.choices).every(field => String(values[field.key] || "").trim());
    if (!context || !inputs || !required) { showFormError("Fill in the required fields."); return; }
    let next;
    if (formThing) {
      next = T.normalize({ ...formThing, context, inputs });
      state.things = state.things.map(t => t.id === formThing.id ? next : t);
      state.rows.delete(next.id);
    } else {
      next = T.create(kind, context, inputs, state.things);
      state.things = [...state.things, next];
    }
    $("#wl-form").close();
    renderList();
    select(next.id);
    loadRow(next);
    if (formThing && inspector.current()?.thing.id === next.id) inspector.show(next);
    notice(formThing ? `Updated ${T.name(next)}.` : `Added ${T.name(next)}.`);
    formThing = null;
    save();
  }

  const appCommands = { add: () => openForm(null), classic: () => { location.href = "index.html"; } };

  Object.assign(actions, {
    add: () => openForm(null),
    edit: () => { const thing = selectedThing(); if (thing) openForm(thing); },
    down: () => state.focus === "list" ? moveSelection(1) : inspector.move(1),
    up: () => state.focus === "list" ? moveSelection(-1) : inspector.move(-1),
    top: () => state.focus === "list" ? moveSelection("top") : inspector.move(-1000),
    bottom: () => state.focus === "list" ? moveSelection("bottom") : inspector.move(1000),
    open: () => state.focus === "list" ? openSelected() : inspector.drill(),
    back: () => {
      const filter = $("#wl-filter");
      if (document.activeElement === filter) { filter.value = ""; applyFilter(""); setFocus("list"); return; }
      if (state.help) { state.help = false; renderKeys(); return; }
      if (state.focus === "inspector" && !inspector.back()) setFocus("list");
    },
    focus: () => setFocus(state.focus === "list" ? "inspector" : "list"),
    filter: () => $("#wl-filter").focus(),
    refresh: () => {
      const shown = inspector.current()?.thing;
      if (state.focus === "inspector" && shown) {
        inspector.refresh();
        // On-demand things run once, in the inspector, which updates the row.
        if (!shown.unsaved && T.KINDS[shown.kind].autoload) loadRow(shown, { run: true });
        return;
      }
      const thing = selectedThing();
      if (!thing) return;
      if (shown?.id === thing.id) {
        inspector.refresh();
        if (T.KINDS[thing.kind].autoload) loadRow(thing, { run: true });
      } else loadRow(thing, { run: true });
    },
    "refresh-all": () => refreshAll(),
    copy: () => copyMain(),
    console: () => inspector.openConsole(),
    "next-error": () => inspector.jumpError(1),
    "previous-error": () => inspector.jumpError(-1),
    remove: () => removeSelected(),
    "move-down": () => moveThing(1),
    "move-up": () => moveThing(-1),
    help: () => { state.help = !state.help; renderKeys(); },
  });

  document.addEventListener("keydown", event => {
    if (event.defaultPrevented || document.querySelector("dialog[open]")) return;
    const target = event.target;
    const typing = target instanceof HTMLElement && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));
    const action = keymap.action({ key: event.key, metaKey: event.metaKey, ctrlKey: event.ctrlKey,
      altKey: event.altKey, shiftKey: event.shiftKey, repeat: event.repeat, typing });
    if (!action || !actions[action]) return;
    const onSurface = target === document.body || target.closest?.(".wl-row, .wl-item, #wl-list, #wl-inspector .wl-view, tr.expandable, tr[data-source-row], .exec-action-block, .log-line");
    if (["open", "focus"].includes(action) && !typing && !onSurface) return;
    // Let the browser activate a focused button or link itself.
    if (action === "open" && target.closest?.("button, a")) return;
    event.preventDefault();
    actions[action]();
  });

  async function migrateIfNeeded(loaded) {
    if (loaded?._storage?.status !== "missing") return;
    let dashboard;
    try { dashboard = await call("dashboard_get"); } catch (_) { return; }
    const tiles = Array.isArray(dashboard?.tiles) ? dashboard.tiles : [];
    if (!tiles.length || dashboard.ok === false) return;
    const result = T.migrateTiles(tiles, { defaultProfile: state.settings?.default_profile || "", profiles: state.profiles,
      allowedRegions: state.allowedRegions, defaultRegion: state.settings?.default_region || "" });
    const skipped = result.skippedNoAccount
      ? `${result.skippedNoAccount} widget${result.skippedNoAccount === 1 ? " needs" : "s need"} an account. Set a default profile in Settings, then add ${result.skippedNoAccount === 1 ? "it" : "them"} with a.`
      : "";
    if (!result.things.length) { if (skipped) notice(skipped); return; }
    const kept = result.things.slice(0, 200);
    const left = result.things.length - kept.length;
    state.things = kept;
    await save();
    notice([`Converted ${result.converted} widget${result.converted === 1 ? "" : "s"} into ${kept.length} thing${kept.length === 1 ? "" : "s"}.`,
      "Your old dashboard file was not changed.",
      left ? `${left} more did not fit, because the watchlist holds at most 200 things.` : "", skipped].filter(Boolean).join(" "));
  }

  async function boot() {
    $("#wl-mode").hidden = core.isTauri;
    if (!core.isTauri) $("#wl-mode").textContent = "Demo, sample data";
    $("#wl-classic").addEventListener("click", () => { location.href = "index.html"; });
    $("#wl-filter").addEventListener("input", event => applyFilter(event.target.value));
    $("#wl-inspector").addEventListener("focusin", () => { if (state.focus !== "inspector") { state.focus = "inspector"; document.body.dataset.focus = "inspector"; renderKeys(); } });
    $("#wl-list").addEventListener("focusin", () => { if (state.focus !== "list") { state.focus = "list"; document.body.dataset.focus = "list"; renderKeys(); } });
    inspector = window.CloudBurritoInspector.create({ host: $("#wl-inspector"), core, things: T, notice, recovery,
      onResult: (thing, result) => { if (state.things.some(t => t.id === thing.id)) applyRowResult(thing, result); } });
    window.CloudBurritoWatchlist.inspector = inspector;
    renderKeys();
    $("#wl-form-kind").addEventListener("change", event => { renderFormFields(event.target.value, null); resetListing(); });
    $("#wl-form-profile").addEventListener("change", event => {
      fillRegions(state.profiles.find(p => p.name === event.target.value)?.region);
      resetListing();
      updateAccountNote();
    });
    $("#wl-form-region").addEventListener("change", resetListing);
    $("#wl-form-body").addEventListener("submit", submitForm);
    $("#wl-form-cancel").addEventListener("click", () => { formThing = null; $("#wl-form").close(); });
    $("#wl-form").addEventListener("close", () => { formThing = null; resetListing(); setFocus("list"); });
    try {
      const settings = await call("settings_get");
      state.settings = settings;
      if (["dark", "light"].includes(settings?.theme)) document.documentElement.dataset.theme = settings.theme;
    } catch (_) { notice("Settings could not be loaded. Defaults are used."); }
    try {
      const info = await call("aws_list_profiles");
      state.profiles = (Array.isArray(info?.profiles) ? info.profiles : [])
        .filter(p => p.eligibility === "supported_sso" && p.name && p.account_id)
        .map(p => ({ name: p.name, account_id: p.account_id, region: p.region || "" }));
      state.profilesKnown = info?.ok !== false;
      if (Array.isArray(info?.allowed_regions) && info.allowed_regions.length) state.allowedRegions = info.allowed_regions;
    } catch (_) { state.profilesKnown = false; }
    let loaded;
    try { loaded = await call("watchlist_get"); } catch (_) { loaded = { ok: false }; }
    if (!loaded || loaded.ok === false) {
      $("#wl-list").replaceChildren(el("div", { class: "wl-empty" }, el("h2", {}, "Your watchlist could not be read"),
        el("p", {}, "The file ~/.cloud_burrito/watchlist.json was not changed. Fix or move it, then restart the app.")));
      document.body.dataset.ready = "true";
      return;
    }
    const raw = Array.isArray(loaded.things) ? loaded.things : [];
    state.things = raw.map(T.normalize).filter(Boolean);
    if (state.things.length !== raw.length) notice(`${raw.length - state.things.length} saved things could not be read and were skipped.`);
    state.storageReady = true;
    await migrateIfNeeded(loaded);
    renderList();
    setFocus("list");
    refreshAll();
    document.body.dataset.ready = "true";
  }

  window.CloudBurritoWatchlist = { state, actions, rowEl, selectedThing, select, setFocus, loadRow, refreshAll, renderList, renderKeys, appCommands,
    save, notice, call, form, openForm, inspector: null, onSelect: null };
  document.addEventListener("DOMContentLoaded", boot);
})();
