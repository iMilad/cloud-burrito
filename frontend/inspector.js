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

  window.CloudBurritoInspector = { create, register };
})();
