// Presentation only: no native bridge, resource requests, or dashboard writes.
(function () {
  "use strict";

  const root = document.documentElement;
  const preferenceKey = "cb.presentation.v1";
  const validDesign = value => value === "studio" || value === "classic";
  let preferred = "studio";
  try {
    const saved = localStorage.getItem(preferenceKey);
    if (validDesign(saved)) preferred = saved;
  } catch (_) { /* A blocked preference store must not block the workspace. */ }
  const queryDesign = new URLSearchParams(location.search).get("design");
  root.dataset.design = validDesign(queryDesign) ? queryDesign : preferred;
  root.dataset.density = "comfortable";

  let modal = null;
  let panel = null;
  let search = null;
  let results = null;
  let initialized = false;
  const $ = selector => document.querySelector(selector);
  const actionTargets = { settings: "#settings-btn", audit: "#audit-btn", add: "#add-widget-btn" };
  const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const tiles = () => Array.from($("#grid-stack")?.children || [])
    .filter(item => item.classList.contains("grid-stack-item") && item.querySelector(".widget"));
  const isOpen = () => panel?.getAttribute("aria-hidden") === "false";
  const available = button => !!button && !button.disabled && !button.closest("[hidden], [inert]");

  function close(options = {}) {
    if (modal && panel) modal.hidePanel(panel, options);
    // Closed commands must not retain removed widgets through click closures.
    results?.replaceChildren();
  }

  function chooseDesign(design) {
    if (!validDesign(design)) return;
    close();
    root.dataset.design = design;
    try { localStorage.setItem(preferenceKey, design); } catch (_) { /* Session choice still works. */ }
    const destination = $(design === "classic" ? "#studio-return" : "#studio-classic-switch");
    destination?.focus({ preventScroll: true });
  }

  function exitWidgetFullscreen() {
    $(".widget.fullscreen .fs-btn")?.click();
  }

  function markNavigation(type) {
    document.querySelectorAll("#studio-nav [data-studio-jump]").forEach(button => {
      if (button.dataset.studioJump === type) button.setAttribute("aria-current", "location");
      else button.removeAttribute("aria-current");
    });
  }

  function jumpTo(item) {
    if (!item?.isConnected) return;
    const widget = item.querySelector(".widget");
    if (!widget) return;
    close();
    exitWidgetFullscreen();
    markNavigation(widget.dataset.widget);
    widget.setAttribute("tabindex", "-1");
    widget.focus({ preventScroll: true });
    widget.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block: "center" });
    widget.classList.remove("studio-highlight");
    // Restart only this short presentation effect; no layout state changes.
    requestAnimationFrame(() => {
      if (!widget.isConnected) return;
      widget.classList.add("studio-highlight");
      clearTimeout(widget._studioHighlightTimer);
      widget._studioHighlightTimer = setTimeout(() => widget.classList.remove("studio-highlight"), 1100);
    });
  }

  function overview() {
    close();
    exitWidgetFullscreen();
    markNavigation("overview");
    const heading = $(".studio-overview h1") || $("#studio-launcher");
    if (heading) {
      if (heading.tagName === "H1") heading.setAttribute("tabindex", "-1");
      heading.focus({ preventScroll: true });
    }
    window.scrollTo({ top: 0, behavior: reducedMotion() ? "auto" : "smooth" });
  }

  function delegateAction(action) {
    const button = $(actionTargets[action] || "#no-studio-action");
    // The modal makes background controls inert until it closes.
    close();
    if (!available(button)) return;
    button.focus({ preventScroll: true });
    button.click();
  }

  function commands() {
    const entries = [{ key: "overview", label: "Workspace overview", detail: "Navigate", run: overview }];
    const current = tiles();
    const names = current.map(item => item.querySelector(".widget-title")?.textContent.trim() || "Untitled tool");
    const totals = new Map();
    names.forEach(name => totals.set(name, (totals.get(name) || 0) + 1));
    const seen = new Map();
    current.forEach((item, index) => {
      const name = names[index];
      const ordinal = (seen.get(name) || 0) + 1;
      seen.set(name, ordinal);
      entries.push({ key: `tile:${item.getAttribute("gs-id") || index}`,
        label: totals.get(name) > 1 ? `${name} · ${ordinal}` : name,
        detail: "Open tool", search: `${name} ${item.getAttribute("gs-id") || ""}`,
        run: () => jumpTo(item) });
    });
    for (const [action, label] of [["add", "Open widget library"], ["settings", "Open settings"], ["audit", "Open audit log"]]) {
      const button = $(actionTargets[action]);
      // Ignore modal-induced inertness here; explicit action closes it first.
      if (button && !button.disabled && !button.closest("[hidden]")) {
        entries.push({ key: action, label, detail: "Open panel", run: () => delegateAction(action) });
      }
    }
    return entries;
  }

  function renderCommands() {
    if (!results || !search) return;
    const focused = results.contains(document.activeElement) ? document.activeElement.dataset.commandKey : null;
    const query = search.value.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
    const matching = commands().filter(command => {
      const text = `${command.label} ${command.search || ""}`.toLocaleLowerCase();
      return query.every(token => text.includes(token));
    });
    const fragment = document.createDocumentFragment();
    matching.forEach(command => {
      const row = document.createElement("div");
      row.setAttribute("role", "listitem");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "studio-command";
      button.dataset.commandKey = command.key;
      const title = document.createElement("span");
      title.className = "studio-command-title";
      title.textContent = command.label;
      const detail = document.createElement("span");
      detail.className = "studio-command-detail";
      detail.textContent = command.detail;
      button.append(title, detail);
      button.addEventListener("click", command.run);
      row.append(button);
      fragment.append(row);
    });
    results.replaceChildren(fragment);
    $("#command-empty").hidden = matching.length !== 0;
    if (focused) {
      const replacement = Array.from(results.querySelectorAll("button"))
        .find(button => button.dataset.commandKey === focused);
      (replacement || search).focus({ preventScroll: true });
    }
  }

  function open() {
    if (root.dataset.design !== "studio" || !modal || !panel) return;
    if (document.querySelector('[role="dialog"][aria-hidden="false"]') && !isOpen()) return;
    search.value = "";
    renderCommands();
    modal.showPanel(panel);
    search.focus({ preventScroll: true });
  }

  function updateWorkspace() {
    const current = tiles();
    $("#studio-tool-count").textContent = String(current.length);
    $("#studio-summary").textContent = `${current.length} open tool${current.length === 1 ? "" : "s"}`;
    const types = new Set(current.map(item => item.querySelector(".widget").dataset.widget));
    document.querySelectorAll("[data-studio-jump]").forEach(button => {
      button.disabled = button.dataset.studioJump !== "overview" && !types.has(button.dataset.studioJump);
      if (button.disabled) button.removeAttribute("aria-current");
    });
    document.querySelectorAll("[data-studio-action]").forEach(button => {
      const target = $(actionTargets[button.dataset.studioAction] || "#no-studio-action");
      button.disabled = !target || target.disabled || !!target.closest("[hidden]");
    });
    if (isOpen()) renderCommands();
  }

  function updateMode() {
    const native = typeof window.__TAURI__ !== "undefined";
    const connection = $("#connection-status");
    const verified = native && connection && !connection.hidden && connection.dataset.state === "verified";
    const label = $("#studio-mode-label");
    label.textContent = !native ? "Demo · synthetic data"
      : verified ? "Desktop · identity verified" : "Desktop · identity required";
    label.dataset.state = !native ? "demo" : verified ? "verified" : "unverified";
  }

  function init(api) {
    if (initialized) return;
    modal = api;
    panel = $("#command-panel");
    search = $("#command-search");
    results = $("#command-results");
    if (!panel || !search || !results || !api?.showPanel || !api?.hidePanel) return;
    initialized = true;
    results.setAttribute("role", "list");
    results.setAttribute("aria-label", "Matching commands");
    $("#studio-launcher")?.addEventListener("click", open);
    $("#command-close")?.addEventListener("click", () => close());
    $("#studio-classic-switch")?.addEventListener("click", () => chooseDesign("classic"));
    $("#studio-return")?.addEventListener("click", () => chooseDesign("studio"));
    $("#studio-density")?.addEventListener("click", event => {
      const compact = root.dataset.density !== "compact";
      root.dataset.density = compact ? "compact" : "comfortable";
      event.currentTarget.setAttribute("aria-pressed", String(compact));
    });
    document.addEventListener("click", event => {
      if (root.dataset.design !== "studio" || !(event.target instanceof Element)) return;
      const button = event.target.closest("[data-studio-jump], [data-studio-action]");
      if (!available(button)) return;
      if (button.dataset.studioAction) delegateAction(button.dataset.studioAction);
      else if (button.dataset.studioJump === "overview") overview();
      else jumpTo(tiles().find(item => item.querySelector(".widget").dataset.widget === button.dataset.studioJump));
    });
    search.addEventListener("input", renderCommands);
    panel.addEventListener("keydown", event => {
      if (event.defaultPrevented || event.isComposing) return;
      const buttons = Array.from(results.querySelectorAll("button"));
      if (!buttons.length) return;
      const index = buttons.indexOf(document.activeElement);
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const next = index < 0 ? (event.key === "ArrowDown" ? 0 : buttons.length - 1)
          : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next].focus();
      } else if (event.key === "Enter" && document.activeElement === search) {
        event.preventDefault();
        buttons[0].click();
      }
    });
    document.addEventListener("keydown", event => {
      if (event.defaultPrevented || event.isComposing || event.repeat || event.altKey
          || root.dataset.design !== "studio" || !(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "k") return;
      if (document.querySelector('[role="dialog"][aria-hidden="false"]') && !isOpen()) return;
      event.preventDefault();
      if (isOpen()) close(); else open();
    });
    const grid = $("#grid-stack");
    if (grid) new MutationObserver(updateWorkspace).observe(grid, { childList: true });
    const connection = $("#connection-status");
    if (connection) new MutationObserver(updateMode).observe(connection, { attributes: true, attributeFilter: ["data-state", "hidden"] });
    updateWorkspace();
    updateMode();
  }

  window.CloudBurritoStudio = { init, close };
})();
