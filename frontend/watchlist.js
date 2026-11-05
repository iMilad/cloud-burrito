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

  core.configure({
    contextFor: owner => core.contextOverrideFromElement(owner) || { mode: "inherit", profile: null, account_id: null, region: null },
    isIndependent: () => true,
    requestsAllowed: () => state.storageReady,
    resultTitle: host => host.closest?.("[data-result-title]")?.dataset.resultTitle || "Result",
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

  Object.assign(actions, {
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
      const thing = selectedThing();
      if (!thing) return;
      loadRow(thing, { run: true });
      if (inspector.current()?.thing.id === thing.id) inspector.refresh();
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
    event.preventDefault();
    actions[action]();
  });

  async function boot() {
    $("#wl-mode").hidden = core.isTauri;
    if (!core.isTauri) $("#wl-mode").textContent = "Demo, sample data";
    $("#wl-classic").addEventListener("click", () => { location.href = "index.html"; });
    $("#wl-filter").addEventListener("input", event => applyFilter(event.target.value));
    $("#wl-inspector").addEventListener("focusin", () => { if (state.focus !== "inspector") { state.focus = "inspector"; document.body.dataset.focus = "inspector"; renderKeys(); } });
    $("#wl-list").addEventListener("focusin", () => { if (state.focus !== "list") { state.focus = "list"; document.body.dataset.focus = "list"; renderKeys(); } });
    inspector = window.CloudBurritoInspector.create({ host: $("#wl-inspector"), core, things: T, notice });
    window.CloudBurritoWatchlist.inspector = inspector;
    renderKeys();
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
    await window.CloudBurritoWatchlist.beforeFirstRender?.(loaded);
    renderList();
    setFocus("list");
    refreshAll();
    document.body.dataset.ready = "true";
  }

  window.CloudBurritoWatchlist = { state, actions, rowEl, selectedThing, select, setFocus, loadRow, refreshAll, renderList,
    save, notice, call, inspector: null, beforeFirstRender: null, onSelect: null };
  document.addEventListener("DOMContentLoaded", boot);
})();
