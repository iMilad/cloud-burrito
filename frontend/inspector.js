// Inspector: a stack of views for one thing, shown with a breadcrumb trail.
// Kind-specific views register through register(); others use a generic view.
(function () {
  "use strict";
  const FACTORIES = {};
  function register(kind, factory) { FACTORIES[kind] = factory; }

  function create({ host, core, things: T, notice }) {
    const { el, clear } = core;
    let thing = null;
    let stack = [];
    const crumb = el("nav", { class: "wl-crumb", "aria-label": "Inspector path" });
    const account = el("span", { class: "wl-acct" });
    const body = el("div", { class: "wl-view", tabindex: "-1" });
    host.append(el("div", { class: "wl-inspector-head" }, crumb, account), body);
    const top = () => stack[stack.length - 1] || null;
    const helpers = { core, T, el, notice, push: view => push(view) };

    function genericView(t) {
      return { title: T.name(t), async render(view) {
        const { widget, inputs } = T.rowRequest(t);
        await core.fetchWidgetInto(view.host, widget, inputs, t.context);
      } };
    }
    function items(view) {
      const list = view?.items ? view.items(view.host) : [];
      list.forEach(node => { if (node.tabIndex < 0 && !node.hasAttribute("tabindex")) node.tabIndex = -1; });
      return list;
    }
    function highlight(view) {
      items(view).forEach((node, n) => {
        node.classList.toggle("wl-current", n === view.index);
        node.setAttribute("aria-current", String(n === view.index));
      });
    }
    function paint() {
      const view = top();
      clear(crumb);
      stack.forEach((v, n) => {
        if (n) crumb.append(el("span", { "aria-hidden": "true" }, "›"));
        crumb.append(el(n === stack.length - 1 ? "strong" : "span", {}, v.title));
      });
      account.textContent = thing ? `${thing.context.profile}, ${thing.context.region}` : "";
      account.title = thing ? `${thing.context.profile}, account ${thing.context.account_id}, ${thing.context.region}` : "";
      if (!view) { body.replaceChildren(el("p", { class: "wl-hint" }, "Select a thing and press Enter.")); return; }
      body.replaceChildren(view.host);
      if (!view.done) {
        Promise.resolve(view.render(view)).then(() => {
          if (view.host.isConnected) { view.done = true; highlight(view); }
        });
      } else highlight(view);
    }
    function push(view) {
      view.host = el("div", { class: "wl-view-host", "data-result-title": view.title });
      view.host._widgetContextOverride = thing.context;
      view.index = view.index || 0;
      view.done = false;
      stack.push(view);
      paint();
    }
    function show(t) {
      for (const v of stack) core.invalidateRequests(v.host);
      thing = t;
      stack = [];
      push((FACTORIES[t.kind] || genericView)(t, helpers));
    }
    function back() {
      if (stack.length <= 1) return false;
      core.invalidateRequests(stack.pop().host);
      paint();
      return true;
    }
    function move(delta) {
      const view = top();
      const list = items(view);
      if (!list.length) return;
      view.index = Math.max(0, Math.min(list.length - 1, (view.index || 0) + delta));
      highlight(view);
      list[view.index].focus({ preventScroll: true });
      list[view.index].scrollIntoView({ block: "nearest" });
    }
    function drill() { const view = top(); if (view?.open) view.open(view.index || 0, helpers); }
    function refresh() {
      const view = top();
      if (!view) return;
      core.invalidateRequests(view.host);
      clear(view.host);
      view.done = false;
      paint();
    }
    function copyMain() { const view = top(); return view?.copy ? view.copy(view.index || 0) : null; }
    function openConsole() {
      const view = top();
      const link = view?.consoleLink ? view.consoleLink(view.index || 0) : null;
      if (link) link.click(); else notice("No AWS console link is available here.");
    }
    function jumpError(delta) {
      const view = top();
      const lines = Array.from(view?.host.querySelectorAll(".log-line") || []);
      const errors = lines.map((line, n) => line.classList.contains("error") ? n : -1).filter(n => n >= 0);
      if (!errors.length) { notice("No error lines here."); return; }
      const current = view.index || 0;
      const next = delta > 0 ? errors.find(n => n > current) ?? errors[0] : [...errors].reverse().find(n => n < current) ?? errors[errors.length - 1];
      view.index = next;
      lines.forEach((line, n) => line.classList.toggle("wl-current", n === next));
      lines[next].scrollIntoView({ block: "center" });
    }
    function clearAll() {
      for (const v of stack) core.invalidateRequests(v.host);
      thing = null;
      stack = [];
      paint();
    }
    function focus() { const view = top(); const list = items(view); (list[view?.index || 0] || body).focus({ preventScroll: true }); }
    paint();
    return { show, back, move, drill, refresh, copyMain, openConsole, jumpError, clear: clearAll, focus,
      current: () => thing ? { thing, depth: stack.length } : null };
  }

  const shortId = id => { const tail = String(id || "").replace(/^exec-/, ""); return tail ? `exec-${tail.slice(-4)}` : ""; };

  function runsView(thing, h) {
    const view = {
      title: thing.inputs.pipeline_name,
      rows: [],
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData("pipeline-runs", { pipeline_name: thing.inputs.pipeline_name, max_results: 10 },
            v.host, thing.context, request);
          if (!request.accept(result)) return;
          view.rows = Array.isArray(result.rows) ? result.rows : [];
          h.core.renderWithResultState(v.host, result, () => {
            h.core.clear(v.host);
            if (result.render !== "table") { h.core.dispatchRenderContent(v.host, result); return; }
            if (!view.rows.length) {
              v.host.append(h.el("p", { class: "wl-hint" }, h.core.resultFailure(result)
                ? result.error || "Runs could not be loaded. Press r to retry." : "No runs were returned for this pipeline."));
              return;
            }
            v.host.append(h.el("ul", { class: "wl-items", role: "listbox", "aria-label": "Runs" }, ...view.rows.map((run, n) =>
              h.el("li", { class: "wl-item", role: "option", tabindex: "-1", "data-index": String(n) },
                h.el("span", { class: `wl-dot wl-tone-${h.T.statusTone(run.status)}`, "aria-hidden": "true" }),
                h.el("code", { class: "wl-id" }, run.execution_id),
                h.el("span", { class: "wl-muted" }, run.trigger || ""),
                h.el("span", { class: `badge ${h.core.statusToBadge(run.status)}` }, h.core.formatStatusLabel(run.status)),
                h.el("span", { class: "wl-muted" }, h.core.formatDisplayValue("last_updated", run.last_updated).text)))));
            v.host.querySelectorAll(".wl-item").forEach(item => item.addEventListener("click", () => {
              view.index = Number(item.dataset.index);
              view.open(view.index, h);
            }));
          });
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".wl-item")),
      copy: n => view.rows[n]?.execution_id || null,
      open(n, { push }) { const run = view.rows[n]; if (run) push(actionsView(thing, run, h)); },
    };
    return view;
  }

  function actionsView(thing, run, h) {
    const view = {
      title: `run ${shortId(run.execution_id)}`,
      actions: [],
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData("pipeline-execution-detail",
            { pipeline_name: thing.inputs.pipeline_name, execution_id: run.execution_id }, v.host, thing.context, request);
          if (!request.accept(result)) return;
          view.actions = Array.isArray(result.actions) ? result.actions : [];
          h.core.dispatchRender(v.host, result);
          const failed = view.actions.findIndex(a => /fail/i.test(String(a.status)));
          view.index = failed >= 0 ? failed : 0;
        } catch (error) { if (request.current()) h.core.renderError(v.host, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".exec-action-block")),
      consoleLink: n => view.host.querySelectorAll(".exec-action-block")[n]?.querySelector("a.exec-link") || null,
      copy: () => run.execution_id,
      open(n, { push, notice }) {
        const action = view.actions[n];
        const target = action ? h.core.reviewedHandoff(action.handoffs?.build, "build") : null;
        if (target?.status === "available" && target.widget === "codebuild-log") push(buildLogView(thing, target, h));
        else notice("This action has no build log to open.");
      },
    };
    return view;
  }

  function buildLogView(thing, target, h) {
    const view = {
      title: "Build log",
      async render(v) { await h.core.fetchWidgetInto(v.host, "codebuild-log", target.inputs, thing.context); },
      items: host => Array.from(host.querySelectorAll(".log-line")),
      copy: n => view.host.querySelectorAll(".log-line")[n]?.querySelector(".msg")?.textContent || null,
    };
    return view;
  }

  function packagesView(thing, h) {
    const view = {
      title: thing.inputs.package_prefix,
      rows: [],
      async render(v) {
        const i = thing.inputs;
        const base = { domain: i.domain, repository: i.repository, package_prefix: i.package_prefix,
          ...(i.domain_owner ? { domain_owner: i.domain_owner } : {}) };
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const list = await h.core.fetchWidgetData("codeartifact-packages", { ...base, mode: "list", max_packages: 50 },
            v.host, thing.context, request);
          if (!request.accept(list)) return;
          let rows = Array.isArray(list.rows) ? list.rows.map(r => ({ ...r })) : [];
          let partial = null;
          const pending = rows.filter(r => r.enrichment_state !== "complete").map(r => r.package);
          for (let start = 0; start < pending.length; start += 25) {
            const owner = h.el("div", { hidden: true });
            v.host.append(owner);
            const batch = h.core.beginOwnedRequest(owner, thing.context);
            const enriched = await h.core.fetchWidgetData("codeartifact-packages",
              { ...base, mode: "enrich", packages: pending.slice(start, start + 25) }, owner, thing.context, batch);
            if (!request.current() || !batch.accept(enriched)) return;
            const byName = new Map((Array.isArray(enriched.rows) ? enriched.rows : []).map(r => [r.package, r]));
            rows = rows.map(r => byName.has(r.package) ? { ...r, ...byName.get(r.package) } : r);
            if (h.core.resultFailure(enriched)) partial = "Some package details could not be loaded.";
            owner.remove();
          }
          view.rows = rows;
          h.core.renderCodeArtifactPackagesTable(v.host, { ...list, rows, ...(partial ? { ok: false, partial: true, error: partial } : {}) },
            { form: v.host, inputs: base, context: thing.context });
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll("tr.expandable")),
      copy: n => view.rows[n]?.latest_version || null,
      open(n) { view.host.querySelectorAll("tr.expandable")[n]?.click(); },
    };
    return view;
  }

  function logsView(thing, h) {
    const widget = thing.inputs.source === "lambda" ? "log-tail" : "cloudwatch-logs";
    const view = {
      title: thing.inputs.function_name || thing.inputs.log_group,
      streams: [],
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData(widget, { mode: "streams", log_group: thing.inputs.log_group, max_streams: 50 },
            v.host, thing.context, request);
          if (!request.accept(result)) return;
          h.core.renderArrayResult(v.host, result, "streams", streams => {
            h.core.clear(v.host);
            view.streams = streams.filter(s => typeof s.name === "string" && s.name);
            if (thing.inputs.source === "lambda") v.host.append(h.el("p", { class: "wl-hint" },
              `Log group ${thing.inputs.log_group} comes from the default Lambda naming convention. It is not verified.`));
            if (!view.streams.length) { v.host.append(h.el("p", { class: "wl-hint" }, "No log streams were returned for this log group.")); return; }
            v.host.append(h.el("ul", { class: "wl-items", role: "listbox", "aria-label": "Log streams" }, ...view.streams.map((s, n) =>
              h.el("li", { class: "wl-item", role: "option", tabindex: "-1", "data-index": String(n) },
                h.el("span", { class: "wl-dot", "aria-hidden": "true" }),
                h.el("code", { class: "wl-id" }, s.name),
                h.el("span", { class: "wl-muted" }, "last event"),
                h.el("span", { class: "wl-muted" }, Number(s.last_event_timestamp)
                  ? h.core.formatDisplayValue("time", new Date(Number(s.last_event_timestamp)).toISOString()).text : ""),
                h.el("span", {})))));
            v.host.querySelectorAll(".wl-item").forEach(item => item.addEventListener("click", () => {
              view.index = Number(item.dataset.index);
              view.open(view.index, h);
            }));
          });
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".wl-item")),
      copy: n => view.streams[n]?.name || null,
      open(n, { push }) { const stream = view.streams[n]; if (stream) push(eventsView(thing, widget, stream.name, h)); },
    };
    return view;
  }

  function eventsView(thing, widget, stream, h) {
    const view = {
      title: stream,
      async render(v) {
        await h.core.fetchWidgetInto(v.host, widget, { mode: "events", log_group: thing.inputs.log_group, log_stream: stream, limit: 500 }, thing.context);
      },
      items: host => Array.from(host.querySelectorAll(".log-line")),
      copy: n => view.host.querySelectorAll(".log-line")[n]?.querySelector(".msg")?.textContent || null,
    };
    return view;
  }

  register("pipeline", runsView);
  register("logs", logsView);
  register("package", packagesView);

  window.CloudBurritoInspector = { create, register };
})();
