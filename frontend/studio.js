// Presentation only: no native bridge, resource requests, or dashboard writes.
(function () {
  "use strict";

  const root = document.documentElement;
  const preferenceKey = "cb.presentation.v1";
  const appearancePreferenceKey = "cb.studio.appearance.v1";
  const validDesign = value => value === "studio" || value === "classic";
  const validDensity = value => value === "compact" || value === "comfortable";
  const validOverview = value => ["compact", "expanded", "hidden"].includes(value);
  const workspaceStorageFailures = new Set();
  function readWorkspacePreference(key, validate, fallback) {
    try {
      const saved = localStorage.getItem(key);
      return validate(saved) ? saved : fallback;
    } catch (_) {
      workspaceStorageFailures.add(key);
      return fallback;
    }
  }
  const appearances = {
    original: { label: "Studio Original", asset: "assets/cloud-burrito-style-current.svg" },
    precision: { label: "Precision", asset: "assets/cloud-burrito-style-precision.svg" },
    paper: { label: "Paper", asset: "assets/cloud-burrito-style-paper.svg" },
    night: { label: "Night Shift", asset: "assets/cloud-burrito-style-night.svg" },
  };
  const validAppearance = value => Object.hasOwn(appearances, value);
  let preferred = "studio";
  let preferredAppearance = "paper";
  let appearanceStorageAvailable = true;
  let appearanceHasSavedPreference = false;
  try {
    const saved = localStorage.getItem(preferenceKey);
    if (validDesign(saved)) preferred = saved;
    const savedAppearance = localStorage.getItem(appearancePreferenceKey);
    if (validAppearance(savedAppearance)) {
      preferredAppearance = savedAppearance;
      appearanceHasSavedPreference = true;
    }
  } catch (_) {
    appearanceStorageAvailable = false;
    /* A blocked preference store must not block the workspace. */
  }
  const query = new URLSearchParams(location.search);
  const queryDesign = query.get("design");
  const queryAppearance = query.get("appearance");
  root.dataset.design = validDesign(queryDesign) ? queryDesign : preferred;
  root.dataset.appearance = validAppearance(queryAppearance) ? queryAppearance : preferredAppearance;
  root.dataset.density = readWorkspacePreference("ui.density", validDensity, "compact");
  root.dataset.overview = readWorkspacePreference("ui.overview", validOverview, "compact");

  let modal = null;
  let panel = null;
  let search = null;
  let results = null;
  let initialized = false;
  const $ = selector => document.querySelector(selector);
  const actionTargets = { appearance: "#appearance-btn", settings: "#settings-btn", audit: "#audit-btn", add: "#add-widget-btn" };
  const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const tiles = () => Array.from($("#grid-stack")?.children || [])
    .filter(item => item.classList.contains("grid-stack-item") && item.querySelector(".widget"));
  const isOpen = () => panel?.getAttribute("aria-hidden") === "false";
  const available = button => !!button && !button.disabled && !button.closest("[hidden], [inert]");

  function syncWorkspacePreferences() {
    const hidden = root.dataset.overview === "hidden";
    const expanded = root.dataset.overview === "expanded";
    const hero = $("#studio-overview");
    if (hero) { hero.hidden = hidden; hero.inert = hidden; }
    const toggle = $("#studio-overview-toggle");
    if (toggle) {
      toggle.textContent = expanded ? "Compact overview" : "Expand overview";
      toggle.setAttribute("aria-expanded", String(expanded));
    }
    const show = $("#studio-overview-show");
    if (show) show.hidden = !hidden;
    const mode = $("#studio-workspace-mode");
    if (mode) mode.hidden = !hidden;
    $("#studio-density")?.setAttribute("aria-pressed", String(root.dataset.density === "compact"));
    const choice = $("#appearance-overview");
    if (choice) choice.value = root.dataset.overview;
    const status = $("#workspace-preferences-status");
    if (status) status.textContent = workspaceStorageFailures.size
      ? "Some workspace choices apply for this session only. Local storage is unavailable."
      : "Workspace choices are saved on this device.";
  }

  function persistWorkspacePreference(key, value) {
    try { localStorage.setItem(key, value); workspaceStorageFailures.delete(key); }
    catch (_) { workspaceStorageFailures.add(key); }
    syncWorkspacePreferences();
  }

  function chooseOverview(value) {
    if (!validOverview(value)) return;
    const focus = document.activeElement;
    const wasInside = $("#studio-overview")?.contains(focus);
    const wasShow = focus === $("#studio-overview-show");
    root.dataset.overview = value;
    persistWorkspacePreference("ui.overview", value);
    if (value === "hidden" && wasInside) $("#studio-overview-show")?.focus({ preventScroll: true });
    else if (value !== "hidden" && wasShow) $("#studio-overview-toggle")?.focus({ preventScroll: true });
  }

  function close(options = {}) {
    if (modal && panel) modal.hidePanel(panel, options);
    // Closed commands must not retain removed widgets through click closures.
    results?.replaceChildren();
  }

  function chooseDesign(design) {
    if (!validDesign(design)) return;
    close();
    exitWidgetFullscreen();
    root.dataset.design = design;
    try { localStorage.setItem(preferenceKey, design); } catch (_) { /* Session choice still works. */ }
    // A deliberate choice finishes a URL preview, so reload keeps that choice.
    if (validDesign(queryDesign)) {
      const url = new URL(location.href);
      url.searchParams.delete("design");
      try { history.replaceState(history.state, "", url); } catch (_) { /* Restricted history must not block switching. */ }
    }
    const destination = $(design === "classic" ? "#studio-return" : "#studio-classic-switch");
    destination?.focus({ preventScroll: true });
  }

  function syncAppearance() {
    const appearance = validAppearance(root.dataset.appearance) ? root.dataset.appearance : "paper";
    const definition = appearances[appearance];
    document.querySelectorAll("[data-appearance-choice]").forEach(button => {
      const checked = button.dataset.appearanceChoice === appearance;
      button.setAttribute("aria-checked", String(checked));
      button.tabIndex = checked ? 0 : -1;
    });
    document.querySelectorAll("[data-studio-mark]").forEach(image => { image.src = definition.asset; });
    const icon = $("#app-icon");
    if (icon) icon.href = definition.asset;
    const status = $("#appearance-status");
    const message = validAppearance(queryAppearance) && appearance === queryAppearance
      ? `${definition.label} URL preview is active.`
      : !appearanceStorageAvailable ? `${definition.label} is active for this session.`
      : appearanceHasSavedPreference ? `${definition.label} is active and saved on this device.`
      : `${definition.label} is active as the default.`;
    if (status && status.textContent !== message) status.textContent = message;
  }

  function chooseAppearance(appearance, options = {}) {
    if (!validAppearance(appearance)) return;
    root.dataset.appearance = appearance;
    if (options.persist !== false) {
      try {
        localStorage.setItem(appearancePreferenceKey, appearance);
        appearanceStorageAvailable = true;
        appearanceHasSavedPreference = true;
      } catch (_) { appearanceStorageAvailable = false; }
    }
    syncAppearance();
    if (options.focus !== false) $("[data-appearance-choice][aria-checked='true']")?.focus({ preventScroll: true });
  }

  function openAppearance() {
    close();
    const appearancePanel = $("#appearance-panel");
    if (modal && appearancePanel) modal.showPanel(appearancePanel);
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
    const heading = root.dataset.overview === "hidden" ? $("#studio-workspace-heading") : $(".studio-overview h1");
    if (heading) {
      heading.setAttribute("tabindex", "-1");
      heading.focus({ preventScroll: true });
    }
    window.scrollTo({ top: 0, behavior: reducedMotion() ? "auto" : "smooth" });
  }

  function delegateAction(action) {
    if (action === "appearance") {
      openAppearance();
      return;
    }
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
    for (const [action, label] of [["add", "Open widget library"], ["appearance", "Open appearance"], ["settings", "Open settings"], ["audit", "Open audit log"]]) {
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
    // Disclosure visibility is a user preference, not an identity signal.
    const verified = native && connection?.dataset.state === "verified";
    for (const label of [$("#studio-mode-label"), $("#studio-workspace-mode")]) {
      if (!label) continue;
      label.textContent = !native ? "Demo · sample data"
        : verified ? "Desktop · identity verified" : "Desktop · identity required";
      label.dataset.state = !native ? "demo" : verified ? "verified" : "unverified";
      label.title = !native ? "Sample data only. No AWS connection."
        : verified ? "Your selected AWS identity has been verified." : "Select and verify an AWS identity to load resources.";
    }
    const description = $("#studio-demo-description");
    if (description) description.hidden = native;
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
    $("#appearance-btn")?.addEventListener("click", openAppearance);
    $("#appearance-panel-close")?.addEventListener("click", () => modal.hidePanel($("#appearance-panel")));
    document.querySelectorAll("[data-appearance-choice]").forEach(button => {
      button.addEventListener("click", () => chooseAppearance(button.dataset.appearanceChoice));
      button.addEventListener("keydown", event => {
        if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const keys = Object.keys(appearances);
        const current = keys.indexOf(button.dataset.appearanceChoice);
        const next = event.key === "Home" ? 0 : event.key === "End" ? keys.length - 1
          : (current + (["ArrowRight", "ArrowDown"].includes(event.key) ? 1 : -1) + keys.length) % keys.length;
        chooseAppearance(keys[next]);
      });
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
    syncAppearance();
  }

  function initPresentation() {
    // View navigation must remain usable while native settings are still loading.
    $("#studio-classic-switch")?.addEventListener("click", () => chooseDesign("classic"));
    $("#studio-return")?.addEventListener("click", () => chooseDesign("studio"));
    $("#studio-density")?.addEventListener("click", () => {
      root.dataset.density = root.dataset.density === "compact" ? "comfortable" : "compact";
      persistWorkspacePreference("ui.density", root.dataset.density);
    });
    $("#studio-overview-toggle")?.addEventListener("click", () => chooseOverview(root.dataset.overview === "expanded" ? "compact" : "expanded"));
    $("#studio-overview-hide")?.addEventListener("click", () => chooseOverview("hidden"));
    $("#studio-overview-show")?.addEventListener("click", () => chooseOverview("compact"));
    $("#appearance-overview")?.addEventListener("change", event => chooseOverview(event.target.value));
    syncWorkspacePreferences();
    updateMode();
    syncAppearance();
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", initPresentation, { once: true });
  else initPresentation();

  window.CloudBurritoStudio = { init, close, chooseAppearance };
})();
