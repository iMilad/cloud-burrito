// Inspector: a stack of views for one thing, shown with a breadcrumb trail.
// Kind-specific views register through register(); others use a generic view.
(function () {
  "use strict";
  const FACTORIES = {};
  function register(kind, factory) { FACTORIES[kind] = factory; }

  function create({ host, core, things: T, notice, recovery = null, onResult = null }) {
    const { el, clear } = core;
    let thing = null;
    let stack = [];
    const crumb = el("nav", { class: "wl-crumb", "aria-label": "Inspector path" });
    const account = el("span", { class: "wl-acct" });
    const body = el("div", { class: "wl-view", tabindex: "-1" });
    host.append(el("div", { class: "wl-inspector-head" }, crumb, account), body);
    const top = () => stack[stack.length - 1] || null;
    const helpers = { core, T, el, notice, recovery, onResult, push: view => push(view), refresh: () => refresh(),
      highlight: view => highlight(view) };

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
      if (thing?.unsaved) crumb.append(el("span", { class: "wl-state-flag" }, "not saved"));
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
      if ("armed" in view) view.armed = true;
      // Keep the accepted result on screen: a refresh that fails shows it as stale.
      core.invalidateRequests(view.host, true);
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

  // Rows carry their own data, so Enter and y act on what is shown, even when
  // a failed refresh leaves the older rows on screen as stale.
  const shownRow = (view, n) => view.items(view.host)[n]?._row || null;

  const shortId = id => { const tail = String(id || "").replace(/^exec-/, ""); return tail ? `exec-${tail.slice(-4)}` : ""; };

  function runsView(thing, h) {
    const view = {
      title: thing.inputs.pipeline_name,
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData("pipeline-runs", { pipeline_name: thing.inputs.pipeline_name, max_results: 10 },
            v.host, thing.context, request);
          if (!request.accept(result)) return;
          h.core.renderWithResultState(v.host, result, () => {
            const rows = Array.isArray(result.rows) ? result.rows : [];
            h.core.clear(v.host);
            if (result.render !== "table") { h.core.dispatchRenderContent(v.host, result); return; }
            if (!rows.length) {
              v.host.append(h.el("p", { class: "wl-hint" }, h.core.resultFailure(result)
                ? result.error || "Runs could not be loaded. Press r to retry." : "No runs were returned for this pipeline."));
              return;
            }
            v.host.append(h.el("ul", { class: "wl-items", role: "listbox", "aria-label": "Runs" }, ...rows.map((run, n) => Object.assign(
              h.el("li", { class: "wl-item", role: "option", tabindex: "-1", "data-index": String(n) },
                h.el("span", { class: `wl-dot wl-tone-${h.T.statusTone(run.status)}`, "aria-hidden": "true" }),
                h.el("code", { class: "wl-id" }, run.execution_id),
                h.el("span", { class: "wl-muted" }, run.trigger || ""),
                h.el("span", { class: `badge ${h.core.statusToBadge(run.status)}` }, h.core.formatStatusLabel(run.status)),
                h.el("span", { class: "wl-muted" }, h.core.formatDisplayValue("last_updated", run.last_updated).text)), { _row: run }))));
            v.host.querySelectorAll(".wl-item").forEach(item => item.addEventListener("click", () => {
              view.index = Number(item.dataset.index);
              view.open(view.index, h);
            }));
          });
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".wl-item")),
      copy: n => shownRow(view, n)?.execution_id || null,
      open(n, { push }) { const run = shownRow(view, n); if (run) push(actionsView(thing, run, h)); },
    };
    return view;
  }

  function actionsView(thing, run, h) {
    const view = {
      title: `run ${shortId(run.execution_id)}`,
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData("pipeline-execution-detail",
            { pipeline_name: thing.inputs.pipeline_name, execution_id: run.execution_id }, v.host, thing.context, request);
          if (!request.accept(result)) return;
          h.core.dispatchRender(v.host, result);
          const failed = view.items(v.host).findIndex(block => /fail/i.test(String(block._action?.status)));
          view.index = failed >= 0 ? failed : 0;
        } catch (error) { if (request.current()) h.core.renderError(v.host, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".exec-action-block")),
      consoleLink: n => view.host.querySelectorAll(".exec-action-block")[n]?.querySelector("a.exec-link") || null,
      copy: () => run.execution_id,
      open(n, { push, notice }) {
        const action = view.items(view.host)[n]?._action;
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
    const i = thing.inputs;
    const base = { domain: i.domain, repository: i.repository, package_prefix: i.package_prefix,
      ...(i.domain_owner ? { domain_owner: i.domain_owner } : {}) };
    const token = value => typeof value === "string" && value && value.length <= 4096 ? value : null;
    const view = {
      title: thing.inputs.package_prefix,
      rows: [],
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        view.request = request;
        try {
          const list = await h.core.fetchWidgetData("codeartifact-packages", { ...base, mode: "list", max_packages: 50 },
            v.host, thing.context, request);
          if (!request.accept(list)) return;
          const enriched = await enrich(v, Array.isArray(list.rows) ? list.rows.map(r => ({ ...r })) : [], request);
          if (!enriched) return;
          Object.assign(view, { list, rows: enriched.rows, partial: enriched.partial, coverage: list.coverage,
            next: token(list.next_page_token), tokens: new Set() });
          paintPackages(v);
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll("tr.expandable")),
      copy: n => shownRow(view, n)?.latest_version || null,
      open(n) { view.host.querySelectorAll("tr.expandable")[n]?.click(); },
    };

    // Load package details in batches of 25. Null when the view moved on.
    async function enrich(v, rows, request) {
      let partial = false;
      const pending = rows.filter(r => r.enrichment_state !== "complete").map(r => r.package);
      for (let start = 0; start < pending.length; start += 25) {
        const owner = h.el("div", { class: "wl-batch-owner", hidden: true });
        v.host.append(owner);
        const batch = h.core.beginOwnedRequest(owner, thing.context);
        const enriched = await h.core.fetchWidgetData("codeartifact-packages",
          { ...base, mode: "enrich", packages: pending.slice(start, start + 25) }, owner, thing.context, batch);
        if (!request.current() || !batch.accept(enriched)) return null;
        const byName = new Map((Array.isArray(enriched.rows) ? enriched.rows : []).map(r => [r.package, r]));
        rows = rows.map(r => byName.has(r.package) ? { ...r, ...byName.get(r.package) } : r);
        if (h.core.resultFailure(enriched)) partial = true;
        owner.remove();
      }
      return { rows, partial };
    }

    function paintPackages(v) {
      const coverage = view.coverage ? { ...view.coverage, counts: { ...view.coverage.counts, returned: view.rows.length } } : view.coverage;
      h.core.renderCodeArtifactPackagesTable(v.host, { ...view.list, rows: view.rows, coverage, next_page_token: view.next,
        ...(view.partial ? { ok: false, partial: true, error: "Some package details could not be loaded." } : {}) },
      { form: v.host, inputs: base, context: thing.context });
      if (view.next) v.host.append(h.el("button", { type: "button", class: "btn btn-ghost small wl-next-page",
        onclick: event => nextPage(v, event.currentTarget) }, "Next package page"));
      h.highlight(view);
    }

    // The next page is asked for only on this click, with the thing's own context.
    async function nextPage(v, button) {
      const request = view.request;
      const cursor = view.next;
      if (!request?.current() || !cursor || button.disabled) return;
      const keyboard = document.activeElement === button;
      button.disabled = true;
      v.host.querySelector(".wl-page-owner")?.remove();
      const owner = h.el("div", { class: "wl-page-owner" });
      button.before(owner);
      const page = h.core.beginOwnedRequest(owner, thing.context);
      try {
        const result = await h.core.fetchWidgetData("codeartifact-packages", { ...base, mode: "list", max_packages: 50, page_token: cursor },
          owner, thing.context, page);
        if (!request.current() || !page.accept(result)) return;
        if (h.core.resultFailure(result) || !h.core.validResultShape(result) || !h.core.validCoverage(result.coverage)) {
          h.core.dispatchRender(owner, result);
          button.disabled = false;
          return;
        }
        const known = new Set(view.rows.map(r => r.package));
        const fresh = result.rows.filter(r => typeof r.package === "string" && r.package && !known.has(r.package)).map(r => ({ ...r }));
        const enriched = await enrich(v, fresh, request);
        if (!enriched || !page.current()) return;
        view.tokens.add(cursor);
        const next = token(result.next_page_token);
        Object.assign(view, { rows: [...view.rows, ...enriched.rows], partial: view.partial || enriched.partial,
          coverage: result.coverage, next: next && !view.tokens.has(next) ? next : null });
        owner.remove();
        paintPackages(v);
        if (keyboard) h.core.focusSafely(v.host.querySelector(".wl-next-page") || view.items(v.host)[view.rows.length - enriched.rows.length]);
      } catch (error) {
        if (!page.current()) return;
        h.core.failResult(owner, page, String(error?.message || error));
        button.disabled = false;
      }
    }
    return view;
  }

  function logsView(thing, h) {
    const widget = thing.inputs.source === "lambda" ? "log-tail" : "cloudwatch-logs";
    const view = {
      title: thing.inputs.function_name || thing.inputs.log_group,
      async render(v) {
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData(widget, { mode: "streams", log_group: thing.inputs.log_group, max_streams: 50 },
            v.host, thing.context, request);
          if (!request.accept(result)) return;
          h.core.renderArrayResult(v.host, result, "streams", streams => {
            h.core.clear(v.host);
            const named = streams.filter(s => typeof s.name === "string" && s.name);
            // A group other than the /aws/lambda/<name> guess came from the function's reviewed logging config.
            if (thing.inputs.source === "lambda") v.host.append(h.el("p", { class: "wl-hint" },
              thing.inputs.log_group === `/aws/lambda/${thing.inputs.function_name}`
                ? `Log group ${thing.inputs.log_group} comes from the default Lambda naming convention. It is not verified.`
                : `Log group ${thing.inputs.log_group} comes from this function's logging config.`));
            if (!named.length) { v.host.append(h.el("p", { class: "wl-hint" }, "No log streams were returned for this log group.")); return; }
            v.host.append(h.el("ul", { class: "wl-items", role: "listbox", "aria-label": "Log streams" }, ...named.map((s, n) => Object.assign(
              h.el("li", { class: "wl-item", role: "option", tabindex: "-1", "data-index": String(n) },
                h.el("span", { class: "wl-dot", "aria-hidden": "true" }),
                h.el("code", { class: "wl-id" }, s.name),
                h.el("span", { class: "wl-muted" }, "last event"),
                h.el("span", { class: "wl-muted" }, Number(s.last_event_timestamp)
                  ? h.core.formatDisplayValue("time", new Date(Number(s.last_event_timestamp)).toISOString()).text : ""),
                h.el("span", {})), { _row: s }))));
            v.host.querySelectorAll(".wl-item").forEach(item => item.addEventListener("click", () => {
              view.index = Number(item.dataset.index);
              view.open(view.index, h);
            }));
          });
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      items: host => Array.from(host.querySelectorAll(".wl-item")),
      copy: n => shownRow(view, n)?.name || null,
      open(n, { push }) { const stream = shownRow(view, n); if (stream) push(eventsView(thing, widget, stream.name, h)); },
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

  function stackView(thing, h, stackName = thing.inputs.stack_name) {
    const view = {
      title: stackName,
      async render(v) { await h.core.fetchWidgetInto(v.host, "cfn-stack-detail", { stack_name: stackName }, thing.context); },
      items: host => Array.from(host.querySelectorAll(".stack-detail-panel:not([hidden]) tbody tr")),
      copy: n => view.host.querySelectorAll(".stack-detail-panel:not([hidden]) tbody tr")[n]?.querySelector(".stack-resource-physical")?.textContent || null,
    };
    return view;
  }

  function stacksSearchView(thing, h) {
    const view = {
      title: h.T.name(thing),
      async render(v) {
        const { widget, inputs } = h.T.rowRequest(thing);
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData(widget, inputs, v.host, thing.context, request);
          if (!request.accept(result)) return;
          // Today's table renderer: at most 100 mounted rows per page, and its
          // filter searches every row. Keeps large accounts fast.
          h.core.renderTable(v.host, result, { rowKey: row => row.stack, filterEmptyText: "No stacks match this filter." });
          if (!view.clickWired) {
            view.clickWired = true;
            v.host.addEventListener("click", event => {
              const tr = event.target.closest?.("tbody > tr[data-source-row]");
              if (!tr || event.target.closest("a, button, input")) return;
              view.index = view.items(v.host).indexOf(tr);
              view.open(view.index, h);
            });
          }
        } catch (error) { if (request.current()) h.core.failResult(v.host, request, String(error?.message || error)); }
      },
      // Only the rows of the current page are mounted.
      items: host => Array.from(host.querySelectorAll("tbody > tr[data-source-row]")),
      source: n => shownRow(view, n),
      copy: n => view.source(n)?.stack || null,
      open(n, { push }) { const s = view.source(n); if (s?.stack) push(stackView(thing, h, String(s.stack))); },
    };
    return view;
  }

  function lookupView(thing, h) {
    const view = {
      title: h.T.name(thing),
      async render(v) {
        const { widget, inputs } = h.T.rowRequest(thing);
        await h.core.fetchWidgetInto(v.host, widget, inputs, thing.context);
      },
      items: host => Array.from(host.querySelectorAll(".lookup-row")),
      copy: n => view.host.querySelectorAll(".lookup-row")[n]?.querySelector(".resource")?.textContent || null,
    };
    return view;
  }

  const ON_DEMAND = {
    query: "This runs a Logs Insights query. Queries scan log data and can cost money.",
    errors: "This counts errors with Logs Insights queries over sampled log groups. Queries can cost money.",
    cli: "This runs a reviewed read-only AWS CLI command with this account's verified credentials.",
  };

  function onDemandView(thing, h) {
    const view = {
      title: h.T.name(thing),
      armed: false,
      acknowledge: null,
      async render(v) {
        const { widget, inputs } = h.T.rowRequest(thing);
        if (!view.armed) {
          v.host.append(h.el("p", { class: "wl-hint" }, ON_DEMAND[thing.kind]));
          // A paused query shows why first; r and Enter cannot run it yet.
          if (h.recovery?.get(h.recovery.key(widget, inputs, thing.context))) showRecovery(view, widget, inputs);
          else v.host.append(h.el("p", { class: "wl-hint" }, "Press r or Enter to run it."));
          return;
        }
        const extra = view.acknowledge || {};
        view.acknowledge = null;
        const request = h.core.beginOwnedRequest(v.host, thing.context);
        try {
          const result = await h.core.fetchWidgetData(widget, { ...inputs, ...extra }, v.host, thing.context, request);
          if (request.accept(result)) {
            h.core.dispatchRender(v.host, result);
            if (h.onResult) h.onResult(thing, result);
          }
        } catch (error) { if (request.current()) h.core.renderError(v.host, String(error?.message || error)); }
        if (h.recovery) showRecovery(view, widget, inputs);
      },
      open() { if (view.armed) return; view.armed = true; h.refresh(); },
      items: host => Array.from(host.querySelectorAll("tbody tr, .bar-row")),
      copy: n => view.host.querySelectorAll("tbody tr, .bar-row")[n]?.textContent.trim() || null,
    };

    function showRecovery(v, widget, inputs) {
      v.host.querySelector(".query-recovery")?.remove();
      const item = h.recovery.get(h.recovery.key(widget, inputs, thing.context));
      if (!item) return;
      const box = h.el("section", { class: "query-recovery small", "aria-label": "Unresolved query recovery" },
        h.el("p", {}, "The earlier query may still be running. Runs are paused for this query. Cancelling locally does not prove that the query stopped."),
        h.el("p", { class: "wl-hint" }, "Running it again can overlap the earlier query and cost more. It does not stop the earlier query."));
      if (!item.canAcknowledge) { box.append(h.el("p", { class: "wl-hint" }, "Review the earlier query in the AWS console before running more work.")); v.host.prepend(box); return; }
      const select = item.pending.length ? h.el("select", { "aria-label": "Earlier query to review" },
        h.el("option", { value: "" }, "Choose the earlier query…"),
        ...item.pending.map(p => h.el("option", { value: p.token }, `${p.group}, ${p.window_seconds} seconds, query ${p.query_digest.slice(0, 12)}`))) : null;
      const confirm = h.el("input", { type: "checkbox" });
      const run = h.el("button", { type: "button", class: "btn btn-ghost small", disabled: true }, "Review and run again");
      const update = () => { run.disabled = !confirm.checked || (select && !select.value); };
      confirm.addEventListener("change", update);
      select?.addEventListener("change", () => { confirm.checked = false; update(); });
      run.addEventListener("click", () => {
        if (run.disabled) return;
        view.acknowledge = { acknowledge_unknown: true, ...(select ? { acknowledge_query: select.value } : {}) };
        view.armed = true;
        h.refresh();
      });
      if (select) box.append(select);
      box.append(h.el("label", {}, confirm, h.el("span", {}, "I understand the earlier query may still be running")), run);
      v.host.prepend(box);
    }
    return view;
  }

  register("query", onDemandView);
  register("errors", onDemandView);
  register("cli", onDemandView);

  register("pipeline", runsView);
  register("stack", (thing, h) => stackView(thing, h));
  register("stacks-search", stacksSearchView);
  register("lookup", lookupView);
  register("logs", logsView);
  register("package", packagesView);

  window.CloudBurritoInspector = { create, register };
})();
