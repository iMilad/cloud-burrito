// Settings, policy editor and audit log panels for the watchlist page.
(function () {
  "use strict";
  const core = window.CloudBurritoCore;
  const { $, el, clear, focusableWithin, focusSafely, isTauri, tauriInvoke, displayPreferenceKey, displayPreferences } = core;
  const hooks = { onSettingsLoaded: () => {}, onCredentialsChanged: () => {}, focusFallback: "#settings-btn" };

  const failedDisplaySaves = new Set();

  function saveDisplayChoice(key, value) {
    try { localStorage.setItem(key, value); failedDisplaySaves.delete(key); }
    catch (_) { failedDisplaySaves.add(key); }
    $("#display-preferences-status").textContent = failedDisplaySaves.size
      ? "This display choice is session only because local storage is unavailable."
      : "Display preferences apply immediately and are saved on this device.";
  }

  for (const [id, key] of [["settings-show-tips", "showTips"], ["settings-expand-result-details", "expandResultDetails"]]) {
    const input = $("#" + id);
    input.checked = displayPreferences[key];
    input.addEventListener("change", () => {
      displayPreferences[key] = input.checked;
      document.documentElement.dataset.showTips = String(displayPreferences.showTips);
      if (key === "expandResultDetails") {
        core.markResultDetailsPreferenceChanged();
        document.querySelectorAll(".result-details").forEach(details => {
          details.open = input.checked;
          details.parentElement.parentElement._resultDetailsOpen = input.checked;
          details.parentElement.parentElement._resultDetailsPreferenceRevision = core.resultDetailsPreferenceRevision();
        });
      }
      saveDisplayChoice(displayPreferenceKey, JSON.stringify(displayPreferences));
    });
  }

  // One modal surface owns keyboard focus. Closed panels stay inert throughout
  // their visual transition; the scrim has no delayed hide that can race reopen.
  let activePanel = null;
  let panelReturnFocus = null;
  const panelBackground = new Map();

  function closeActivePanel(options = {}) {
    const close = { "settings-panel": closeSettingsPanel, "audit-panel": closeAuditPanel };
    if (activePanel) close[activePanel.id]?.(options);
  }

  function showPanel(panel) {
    if (!panel) return;
    if (activePanel === panel) return;
    const trigger = activePanel?.contains(document.activeElement) ? panelReturnFocus : document.activeElement;
    closeActivePanel({ restoreFocus: false });
    activePanel = panel;
    panelReturnFocus = trigger;
    panel.inert = false;
    panel.classList.add("open");
    panel.setAttribute("aria-hidden", "false");
    for (const sibling of document.body.children) {
      if (sibling === panel || sibling.id === "scrim" || sibling.classList.contains("sr-only") || sibling.tagName === "SCRIPT") continue;
      panelBackground.set(sibling, sibling.inert);
      sibling.inert = true;
    }
    $("#scrim").hidden = false;
    $("#scrim").classList.add("open");
    focusSafely(focusableWithin(panel)[0] || panel);
  }

  function hidePanel(panel, options = {}) {
    if (!panel || activePanel !== panel) return;
    const trigger = panelReturnFocus;
    activePanel = null;
    panelReturnFocus = null;
    panel.classList.remove("open");
    panel.setAttribute("aria-hidden", "true");
    panel.inert = true;
    for (const [node, inert] of panelBackground) node.inert = inert;
    panelBackground.clear();
    $("#scrim").classList.remove("open");
    $("#scrim").hidden = true;
    if (options.restoreFocus !== false && !focusSafely(trigger)) focusSafely($(hooks.focusFallback));
  }

  // ===== Settings (Tauri only) =====
  let cachedSettings = null;
  let settingsStorageReady = !isTauri;
  let settingsLoadId = 0;
  let settingsDraftRevision = 0;
  let settingsDraftDirty = false;
  let settingsReadFailed = false;
  let settingsDefaults = null;
  let auditRetentionMetadata = null;
  // Browser-only demonstration regions. Desktop choices come from Rust.
  let allowedRegions = isTauri ? [] : ["eu-west-1", "us-east-1"];
  const SETTINGS_FIELDS = {
    aws_config_path: "settings-aws-config-path", sso_session_name: "settings-sso-session",
    default_profile: "settings-default-profile", default_region: "settings-default-region", theme: "settings-theme", audit_retention: "settings-audit-retention",
  };

  const SETTINGS_REQUIRED_KEYS = Object.keys(SETTINGS_FIELDS).filter(key => key !== "audit_retention");

  function settingsRetention(value) {
    if (value === undefined) return "preserve";
    if (!["preserve", "bounded"].includes(value)) throw new Error("Audit retention setting is invalid");
    return value;
  }

  function showSettingsFieldErrors(errors = {}) {
    for (const [key, id] of Object.entries(SETTINGS_FIELDS)) {
      const message = typeof errors[key] === "string" ? errors[key].slice(0, 400) : "";
      $("#" + id).setAttribute("aria-invalid", String(!!message));
      $("#" + id + "-error").textContent = message;
      $("#" + id + "-error").hidden = !message;
    }
  }

  function consumeSettingsMetadata(metadata) {
    if (!metadata || !metadata.defaults || typeof metadata.defaults !== "object"
        || SETTINGS_REQUIRED_KEYS.some(key => typeof metadata.defaults[key] !== "string")
        || !["dark", "light"].includes(metadata.defaults.theme)
        || !Array.isArray(metadata.allowed_regions) || !metadata.allowed_regions.length
        || metadata.allowed_regions.length > 128
        || metadata.allowed_regions.some(region => typeof region !== "string" || !/^[a-z0-9-]{1,64}$/.test(region))
        || !metadata.field_errors || typeof metadata.field_errors !== "object" || Array.isArray(metadata.field_errors)) {
      throw new Error("Settings metadata unavailable");
    }
    settingsDefaults = { ...metadata.defaults, audit_retention: settingsRetention(metadata.defaults.audit_retention) };
    allowedRegions = [...new Set(metadata.allowed_regions)];
    for (const key of ["aws_config_path", "sso_session_name", "default_profile"]) {
      $("#" + SETTINGS_FIELDS[key]).placeholder = settingsDefaults[key] || "(none)";
    }
    populateSettingsRegionSelect($("#settings-default-region").value);
    showSettingsFieldErrors(metadata.field_errors);
  }

  function populateSettingsRegionSelect(value) {
    const select = $("#settings-default-region");
    resetSelect(select);
    addOption(select, "", `Default (${settingsDefaults.default_region})`);
    allowedRegions.forEach(region => addOption(select, region, region));
    if (value && !allowedRegions.includes(value)) addOption(select, value, `${value} — unsupported`);
    select.value = value || "";
  }

  function updateThemePreview() {
    const theme = $("#settings-theme").value;
    if (!["dark", "light"].includes(theme)) return;
    document.documentElement.dataset.theme = theme;
  }

  function markSettingsDirty() {
    settingsDraftRevision++;
    settingsDraftDirty = true;
    if (!settingsSavePending && !settingsReadFailed) $("#settings-status").textContent = "Current edits are not saved.";
    updateThemePreview();
  }

  function showStorageLoadWarning(store, retry, recover) {
    const id = `${store}-storage-warning`;
    let warning = $("#" + id);
    if (!warning) {
      warning = el("div", { id, class: "storage-load-warning", role: "status" },
        el("span", {}, `${store === "settings" ? "Settings" : "Watchlist"} could not be loaded. Saved data has not been replaced.`),
        el("button", { class: "btn btn-ghost small", type: "button", onclick: retry }, "Retry load"),
        el("button", { class: "btn btn-ghost small", type: "button", onclick: recover },
          store === "settings" ? "Replace settings with defaults" : "Reset layout"));
      $("#watchlist-main").before(warning);
    }
  }

  function fillSettingsForm(s) {
    fillIfNonDefault("settings-aws-config-path", s.aws_config_path, "aws_config_path");
    fillIfNonDefault("settings-sso-session", s.sso_session_name, "sso_session_name");
    fillIfNonDefault("settings-default-profile", s.default_profile, "default_profile");
    populateSettingsRegionSelect(s.default_region === settingsDefaults.default_region ? "" : s.default_region);
    $("#settings-theme").querySelectorAll("option[data-unsupported]").forEach(option => option.remove());
    if (!["dark", "light"].includes(s.theme)) {
      addOption($("#settings-theme"), s.theme, `${s.theme} — unsupported`, { unsupported: "true" });
    }
    $("#settings-theme").value = s.theme;
    $("#settings-audit-retention").value = settingsRetention(s.audit_retention);
    settingsDraftDirty = false;
    updateThemePreview();
  }

  async function retrySettingsLoad() {
    const draft = settingsDraftRevision;
    const s = await loadSettings();
    if (s && draft === settingsDraftRevision && !settingsDraftDirty) fillSettingsForm(s);
  }

  async function loadSettings() {
    if (!isTauri || settingsSavePending) return null;
    const request = ++settingsLoadId;
    settingsStorageReady = false;
    $("#settings-save").disabled = true;
    try {
      const loaded = await tauriInvoke("settings_get");
      if (request !== settingsLoadId) return null;
      consumeSettingsMetadata(loaded?._settings);
      if (!loaded || typeof loaded !== "object" || Array.isArray(loaded)
          || loaded.ok === false || loaded._storage?.status === "failed"
          || SETTINGS_REQUIRED_KEYS.some(key => typeof loaded[key] !== "string")) throw new Error("Settings unavailable");
      loaded.audit_retention = settingsRetention(loaded.audit_retention);
      cachedSettings = loaded;
      auditRetentionMetadata = loaded._audit_retention || null;
      settingsStorageReady = true;
      $("#settings-storage-warning")?.remove();
      $("#settings-save").disabled = settingsSavePending;
      if (settingsReadFailed) $("#settings-status").textContent = settingsDraftDirty ? "Current edits are not saved." : "";
      settingsReadFailed = false;
      if (!settingsDraftDirty) fillSettingsForm(loaded);
      hooks.onSettingsLoaded(cachedSettings);
      return cachedSettings;
    } catch (e) {
      if (request !== settingsLoadId) return null;
      settingsStorageReady = false;
      settingsReadFailed = true;
      $("#settings-save").disabled = true;
      $("#settings-status").textContent = "Settings could not be loaded. Retry loading or explicitly replace them with defaults.";
      showStorageLoadWarning("settings", retrySettingsLoad, () => saveSettings({ preventDefault() {} }, { recover: true }));
      console.warn("Settings could not be loaded.");
      return null;
    }
  }

  function fillIfNonDefault(inputId, value, key) {
    const node = $("#" + inputId);
    if (!node) return;
    node.value = value && value !== settingsDefaults[key] ? value : "";
  }

  function openSettingsPanel() {
    const panel = $("#settings-panel");
    showPanel(panel);
    refreshAuditHistory();
    // Re-fetch external edits, while preserving the user's unsaved draft.
    const draft = settingsDraftRevision;
    loadSettings().then((s) => {
      if (!s || draft !== settingsDraftRevision) return;
      if (!settingsDraftDirty) {
        fillSettingsForm(s);
        $("#settings-status").textContent = "";
      }
      loadPolicy();
    });
  }
  function closeSettingsPanel(options = {}) {
    hidePanel($("#settings-panel"), options);
  }

  let settingsSavePending = false;
  async function saveSettings(e, options = {}) {
    e.preventDefault();
    if (!isTauri || settingsSavePending || auditPreservePending) return;
    if (!settingsStorageReady && !options.recover) return;
    // Send empty strings for blank inputs — the backend treats empty as
    // "fall back to default" so the user doesn't need to retype defaults.
    const params = options.recover ? {
      aws_config_path: "", sso_session_name: "", default_profile: "", default_region: "", theme: "", audit_retention: "preserve",
    } : {
      aws_config_path:   $("#settings-aws-config-path").value.trim(),
      sso_session_name:  $("#settings-sso-session").value.trim(),
      default_profile:   $("#settings-default-profile").value.trim(),
      default_region:    $("#settings-default-region").value.trim(),
      theme:             $("#settings-theme").value,
      audit_retention:    $("#settings-audit-retention").value,
    };
    const status = $("#settings-status");
    status.textContent = "Saving…";
    const configuration = core.configurationGeneration();
    const draft = settingsDraftRevision;
    const previous = cachedSettings;
    ++settingsLoadId; // An older read cannot replace a newly accepted save.
    settingsSavePending = true;
    $("#settings-save").disabled = true;
    try {
      const saved = await tauriInvoke("settings_set", { params });
      if (configuration !== core.configurationGeneration()) return;
      consumeSettingsMetadata(saved?._settings);
      if (SETTINGS_REQUIRED_KEYS.some(key => typeof saved[key] !== "string")
          || !["dark", "light"].includes(saved.theme)) throw new Error("Settings save response was incomplete. Reload to verify.");
      ++settingsLoadId;
      saved.audit_retention = settingsRetention(saved.audit_retention);
      cachedSettings = saved;
      auditRetentionMetadata = saved._audit_retention || null;
      refreshAuditHistory();
      settingsStorageReady = true;
      settingsReadFailed = false;
      $("#settings-storage-warning")?.remove();
      if (draft === settingsDraftRevision) {
        if (options.recover) fillSettingsForm(saved);
        settingsDraftDirty = false;
        status.textContent = "Saved.";
      } else {
        status.textContent = "Previous values saved. Current edits are not saved.";
      }
      updateThemePreview();
      hooks.onSettingsLoaded(cachedSettings);
      // Preferences affect the next default choice, not an already verified
      // context. Only the credential source changes its identity contract.
      const credentialsChanged = !previous || ["aws_config_path", "sso_session_name"].some(key => previous[key] !== saved[key]);
      if (!credentialsChanged) return;
      // Credential changes invalidate every old configuration even if the
      // user selected another account while saving. Reverify the current
      // picker selection below; never restore the submitted draft's account.
      core.bumpConfiguration();
      hooks.onCredentialsChanged();
    } catch (err) {
      if (configuration !== core.configurationGeneration()) return;
      if (err.settings) {
        try { consumeSettingsMetadata(err.settings); } catch (_) { /* Keep the last accepted catalogue. */ }
      }
      status.textContent = "Save failed: " + err;
      if (err.error_type === "AuditPreserveRequired") refreshAuditHistory();
    } finally {
      settingsSavePending = false;
      $("#settings-save").disabled = !settingsStorageReady;
    }
  }

  let policyDraftRevision = 0;
  let policyDraftDirty = false;
  let policyLoadId = 0;
  let policyLoadPending = false;
  let policySavePending = false;
  let policyStorageReady = false;

  function updatePolicyControls() {
    $("#policy-save").disabled = policySavePending || !policyStorageReady;
    const reload = $("#policy-reload");
    reload.disabled = policySavePending || policyLoadPending;
    reload.textContent = policyDraftDirty ? "Discard edits & reload" : "Reload & validate";
    reload.title = policyDraftDirty ? "Replace the current draft with the saved policy" : "Read and validate the saved policy";
  }

  function setPolicyStatus(message, valid = null) {
    const status = $("#policy-status");
    status.textContent = message;
    status.className = valid === true ? "small policy-ok" : valid === false ? "small policy-bad" : "small";
  }

  function applyPolicyStatus(s, { replaceDraft, saved = false } = {}) {
    if (!s || s.ok === false || typeof s.raw !== "string" || typeof s.valid !== "boolean") {
      throw new Error("Policy response was incomplete. Reload to verify.");
    }
    const editor = $("#policy-editor");
    const path = $("#policy-path");
    if (path) path.textContent = s.path || "";
    if (replaceDraft) {
      editor.value = s.raw;
      editor.scrollTop = 0;
      editor.scrollLeft = 0;
      renderPolicyHighlight();
      policyDraftDirty = false;
    }
    const subject = saved ? "Saved policy" : "Saved policy on disk";
    const suffix = replaceDraft ? "" : " Current edits are not saved or validated.";
    if (s.valid) {
      const n = Array.isArray(s.actions) ? s.actions.length : 0;
      setPolicyStatus(`${subject} is valid — ${n} action(s) allowed.${suffix}`, replaceDraft ? true : null);
    } else {
      setPolicyStatus(`${subject} is invalid: ${s.error || "invalid policy"}.${suffix}`, false);
    }
  }

  async function loadPolicy({ discardDraft = false } = {}) {
    if (!isTauri || policySavePending || policyLoadPending) return;
    // Opening Settings never discards a policy draft. Only the explicitly
    // labelled reload action replaces edits that existed when it was clicked.
    if (policyDraftDirty && !discardDraft) return;
    const request = ++policyLoadId;
    const draft = policyDraftRevision;
    policyLoadPending = true;
    updatePolicyControls();
    setPolicyStatus("Loading saved policy…");
    try {
      const loaded = await tauriInvoke("policy_get");
      if (request !== policyLoadId) return;
      applyPolicyStatus(loaded, { replaceDraft: draft === policyDraftRevision });
      policyStorageReady = true;
    } catch (e) {
      if (request !== policyLoadId) return;
      setPolicyStatus("Policy could not be loaded. Current edits are preserved. " + e, false);
    } finally {
      if (request === policyLoadId) {
        policyLoadPending = false;
        updatePolicyControls();
      }
    }
  }

  async function savePolicy() {
    if (!isTauri || policySavePending || !policyStorageReady) return;
    const text = $("#policy-editor").value;
    const draft = policyDraftRevision;
    ++policyLoadId; // An earlier read cannot replace an accepted save or its status.
    policyLoadPending = false;
    policySavePending = true;
    updatePolicyControls();
    setPolicyStatus("Saving policy…");
    try {
      const saved = await tauriInvoke("policy_set", { params: { text } });
      // A legacy rejection is not proof that anything was written to disk.
      if (saved?.valid === false) throw new Error("Policy was rejected. Review the draft and try again.");
      if (saved?.raw !== text) throw new Error("Policy save response did not match the submitted draft. Reload to verify.");
      applyPolicyStatus(saved, { replaceDraft: draft === policyDraftRevision, saved: true });
    } catch (e) {
      setPolicyStatus("Policy save failed. Current edits are not saved. " + e, false);
    } finally {
      policySavePending = false;
      updatePolicyControls();
    }
  }

  function markPolicyDirty() {
    ++policyDraftRevision;
    policyDraftDirty = true;
    setPolicyStatus(policySavePending
      ? "Saving the submitted policy. Current edits are not saved or validated."
      : "Current policy edits are not saved or validated.");
    updatePolicyControls();
    renderPolicyHighlight(true);
  }

  function escapeHtml(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function hlValue(s) {
    // `s` is raw (un-escaped) and carries NO injected markup yet. Escape, then
    // wrap quoted strings BEFORE Allow/Deny so no later pass ever re-scans the
    // quotes inside an injected class="..." attribute.
    return escapeHtml(s)
      .replace(/("[^"]*"|'[^']*')/g, '<span class="pe-string">$1</span>')
      .replace(/\bAllow\b/g, '<span class="pe-allow">Allow</span>')
      .replace(/\bDeny\b/g, '<span class="pe-deny">Deny</span>');
  }
  function highlightYamlLine(line) {
    let code = line, comment = "";
    const h = line.indexOf("#");
    if (h !== -1) { code = line.slice(0, h); comment = line.slice(h); }
    // Parse structurally on the RAW text, then escape + wrap each piece so a
    // regex never runs over already-injected HTML (which corrupts class="...").
    const m = code.match(/^(\s*)(- )?(?:([A-Za-z_][\w-]*)(:\s*))?(.*)$/);
    let html;
    if (m) {
      const [, indent, dash, key, colon, rest] = m;
      html = indent;
      if (dash) html += '<span class="pe-dash">- </span>';
      if (key) html += '<span class="pe-key">' + escapeHtml(key) + "</span>" + escapeHtml(colon);
      html += hlValue(rest);
    } else {
      html = hlValue(code);
    }
    if (comment) html += '<span class="pe-comment">' + escapeHtml(comment) + "</span>";
    return html;
  }
  function highlightYaml(text) {
    return text.split("\n").map(highlightYamlLine).join("\n");
  }
  function syncPolicyScroll() {
    const ed = $("#policy-editor"), hl = $("#policy-highlight");
    if (ed && hl) { hl.scrollTop = ed.scrollTop; hl.scrollLeft = ed.scrollLeft; }
  }
  function renderPolicyHighlight(syncScroll = true) {
    const ed = $("#policy-editor"), hl = $("#policy-highlight");
    if (!ed || !hl) return;
    // Reset the overlay's scroll first so stale state from a previous policy
    // can't briefly mis-align the highlight before we re-sync below.
    hl.scrollTop = 0;
    hl.scrollLeft = 0;
    hl.innerHTML = highlightYaml(ed.value) + "\n"; // trailing \n keeps last line aligned at EOF
    if (syncScroll) syncPolicyScroll();
  }

  // ===== Audit log panel (Tauri only) =====
  let auditTimer = null;
  let auditReadPending = false;
  let auditGeneration = 0;
  let auditCursor = null;
  let auditEntries = [];
  let auditHistoryPending = false;
  let auditPreservePending = false;
  let auditHistory = null;

  function auditVisible() {
    return $("#audit-panel").getAttribute("aria-hidden") === "false" && !document.hidden;
  }
  function scheduleAudit(delay = 2000) {
    clearTimeout(auditTimer);
    auditTimer = null;
    if (auditVisible()) auditTimer = setTimeout(refreshAudit, delay);
  }
  function openAuditPanel() {
    showPanel($("#audit-panel"));
    refreshAuditHistory();
    refreshAudit();
  }
  function closeAuditPanel(options = {}) {
    hidePanel($("#audit-panel"), options);
    ++auditGeneration;
    clearTimeout(auditTimer);
    auditTimer = null;
  }
  function auditStorageText(value) {
    return typeof value === "string" ? value.slice(0, 2048) : "Unavailable";
  }
  function auditBytes(value) {
    return Number.isSafeInteger(value) && value >= 0 ? `${(value / 1048576).toFixed(2)} MiB` : "unavailable";
  }
  function renderAuditHistory(error = false) {
    document.querySelectorAll("[data-audit-history-status]").forEach(host => {
      clear(host);
      if (auditRetentionMetadata?.configured_mode === "bounded" && auditRetentionMetadata?.effective_mode === "preserve") {
        host.appendChild(el("p", { class: "audit-read-warning" }, "Bounded retention is saved, but preserve mode is active until the oversized existing history is preserved."));
      }
      if (auditHistory) {
        const mode = auditHistory.effective_mode || auditHistory.mode;
        host.append(el("p", {}, `Active retention: ${mode === "bounded" ? "Bounded — five files × 10 MiB (50 MiB total)" : "Preserve — no automatic expiry"}.`),
          el("p", { class: "mono" }, `Location: ${auditStorageText(auditHistory.location)}`),
          el("p", {}, `Active file: ${auditBytes(auditHistory.active_bytes)} · Current history: ${auditBytes(auditHistory.total_bytes)}.`),
          el("p", { class: "muted" }, auditStorageText(auditHistory.expiry)));
        if (auditHistory.preserve_required === true) host.appendChild(el("p", { class: "audit-read-warning" },
          "Existing history exceeds a rotation file limit. Preserve current history before saving bounded retention."));
      }
      if (error) host.appendChild(el("p", { class: "audit-read-warning", role: "status" },
        "Audit storage status could not be read. The last confirmed status is retained."));
      else if (!auditHistory) host.appendChild(el("p", { class: "muted" }, "Loading audit storage status…"));
    });
  }
  async function refreshAuditHistory() {
    if (!isTauri || auditHistoryPending) return;
    auditHistoryPending = true;
    renderAuditHistory();
    try {
      const result = await tauriInvoke("audit_history", { params: { action: "status" } });
      if (!result || result.ok === false || typeof result.location !== "string"
          || !["preserve", "bounded"].includes(result.effective_mode || result.mode)) throw new Error("Audit status unavailable");
      auditHistory = result;
      if (auditRetentionMetadata) auditRetentionMetadata = { ...auditRetentionMetadata, effective_mode: result.effective_mode || result.mode, preserve_required: result.preserve_required === true };
      renderAuditHistory();
    } catch (_) { renderAuditHistory(true); }
    finally { auditHistoryPending = false; }
  }
  async function preserveAuditHistory() {
    if (!isTauri || auditPreservePending || settingsSavePending) return;
    auditPreservePending = true;
    $("#settings-save").disabled = true;
    document.querySelectorAll("[data-audit-preserve]").forEach(button => { button.disabled = true; });
    const message = text => document.querySelectorAll("[data-audit-preserve-status]").forEach(host => { host.textContent = text; });
    message("Preserving current audit files…");
    try {
      const result = await tauriInvoke("audit_history", { params: { action: "preserve" } });
      if (!result || result.ok === false || !Number.isSafeInteger(result.preserved_files) || result.preserved_files < 0
          || result.preserved_files > 5 || result.preserved_files > 0 && typeof result.preserved_location !== "string") throw new Error("Preservation failed");
      message(result.preserved_files ? `Preserved ${result.preserved_files} files (${auditBytes(result.preserved_bytes)}) in ${auditStorageText(result.preserved_location)}. Preserved copies never expire automatically.` : "No current audit files needed preservation.");
      ++auditGeneration;
      auditCursor = null;
      await refreshAuditHistory();
      if (auditVisible()) refreshAudit();
    } catch (_) { message("Audit preservation did not complete. Existing files may remain in their original or preservation location; review storage before retrying bounded retention."); }
    finally {
      auditPreservePending = false;
      $("#settings-save").disabled = !settingsStorageReady || settingsSavePending;
      document.querySelectorAll("[data-audit-preserve]").forEach(button => { button.disabled = false; });
    }
  }

  // Render a single audit-log row. Each row's "detail" column carries the
  // most useful bits for that kind so a user can read the log top-to-bottom
  // and reconstruct what the app did from core startup to now.
  function auditRowFor(entry) {
    const ts = entry.ts ? new Date(entry.ts * 1000).toLocaleTimeString("en-GB", { hour12: false }) : "";
    const field = (key) => typeof entry[key] === "string" ? entry[key].slice(0, 160) : "";
    let kindLabel = field("kind") || "?";
    let kindClass = "audit-kind-" + kindLabel.replace(/[^a-z-]/gi, "");
    let primary = "";
    let detail = "";
    let extra = "";
    switch (entry.kind) {
      case "request": {
        const outcomes = { started: "Started", succeeded: "Succeeded", failed: "Failed", denied: "Denied", cancelled: "Cancelled (result discarded)" };
        kindLabel = "Request";
        primary = field("command");
        detail = Object.hasOwn(outcomes, entry.event) ? outcomes[entry.event] : "Unknown outcome";
        extra = [field("request_id"), field("account_id"), field("region"), field("error_type")].filter(Boolean).join(" · ");
        if (entry.event === "denied" || entry.event === "failed") kindClass += " audit-fail";
        break;
      }
      case "lifecycle":
        primary = field("event");
        detail = [field("account_id"), field("region"), field("error_type")].filter(Boolean).join(" · ");
        break;
      case "rpc":
        primary = field("method");
        detail = entry.ok === false
          ? `FAILED${field("error_type") ? " · " + field("error_type") : ""}`
          : "ok";
        extra = entry.duration_ms != null ? `${entry.duration_ms} ms` : "";
        if (entry.ok === false) kindClass += " audit-fail";
        break;
      case "aws":
        kindLabel = "Preflight";
        primary = `${field("service") || "?"}.${field("operation") || "?"}`;
        detail = "Permission check allowed; execution not confirmed";
        extra = [field("account_id"), field("region")].filter(Boolean).join(" / ");
        break;
      case "aws-blocked":
        kindLabel = "Preflight";
        primary = `${field("service") || "?"}.${field("operation") || "?"}`;
        detail = "Permission check denied";
        kindClass += " audit-fail";
        break;
      case "widget": {
        primary = field("widget");
        const cleanupLabels = {
          stopped: "Query cleanup: AWS confirmed stopped",
          not_confirmed: "Query cleanup: remote stop not confirmed",
          denied: "Query cleanup: stop denied; remote status unknown",
          failed: "Query cleanup: stop request failed; remote status unknown",
          not_attempted: "Query cleanup: stop not attempted",
        };
        const cleanup = field("cleanup_status");
        detail = entry.event === "cache_hit" ? "Result reused from memory"
          : entry.event === "query_cleanup" && Object.hasOwn(cleanupLabels, cleanup)
            ? cleanupLabels[cleanup] : field("error_type") || "Widget diagnostic";
        const failed = entry.failed_count ?? entry.failed;
        const failedCount = Number.isSafeInteger(failed) && failed >= 0 ? `${failed} failed` : "";
        extra = [field("request_id"), failedCount].filter(Boolean).join(" · ");
        if (cleanup === "denied" || cleanup === "failed" || failed > 0) kindClass += " audit-fail";
        break;
      }
      default:
        primary = "Unrecognized audit event";
    }
    return el("tr", { class: kindClass },
      el("td", { class: "mono muted" }, ts),
      el("td", { class: "mono" }, kindLabel),
      el("td", { class: "mono" }, primary),
      el("td", { class: "mono muted" }, detail),
      el("td", { class: "mono muted" }, extra),
    );
  }

  async function refreshAudit() {
    clearTimeout(auditTimer);
    auditTimer = null;
    if (!isTauri || !auditVisible() || auditReadPending) return;
    const generation = auditGeneration;
    const requestedCursor = auditCursor;
    auditReadPending = true;
    const wrap = $("#audit-table-wrap");
    let warning = $("#audit-read-warning");
    if (!warning) {
      warning = el("p", { id: "audit-read-warning", class: "audit-read-warning small", role: "status", hidden: true });
      wrap.before(warning);
    }
    let nextDelay = 2000;
    try {
      const res = await tauriInvoke("audit_tail", { params: { limit: 300, ...(requestedCursor ? { cursor: requestedCursor } : {}) } });
      if (generation !== auditGeneration || !auditVisible()) return;
      if (!res || res.ok === false || !Array.isArray(res.entries) || res.entries.length > 1000
          || res.cursor != null && (typeof res.cursor !== "string" || !/^[\x20-\x7e]{1,256}$/.test(res.cursor))) throw new Error("Audit page unavailable");
      warning.hidden = true;
      const incoming = res.entries.filter(entry => entry && typeof entry === "object" && !Array.isArray(entry));
      // Cursorless older bridges return a full snapshot, not an append page.
      auditEntries = (res.reset === true || !requestedCursor ? incoming : auditEntries.concat(incoming)).slice(-300);
      auditCursor = typeof res.cursor === "string" ? res.cursor : null;
      const notes = [];
      if (res.reset === true) notes.push("The audit file changed; the displayed page was replaced.");
      if (res.limited === true) notes.push("Older entries were omitted by the bounded reader.");
      if (Number.isSafeInteger(res.skipped) && res.skipped > 0) notes.push(`${res.skipped} invalid or oversized records were skipped.`);
      if (res.partial_tail === true) notes.push("Waiting for an incomplete final record.");
      if (res.has_more === true) { notes.push("More recent entries are being read."); nextDelay = 100; }
      $("#audit-page-status").textContent = notes.join(" ");
      renderAuditEntries(wrap);
    } catch (_) {
      if (generation !== auditGeneration || !auditVisible()) return;
      warning.textContent = "Audit history could not be read. Previously displayed entries are unchanged.";
      warning.hidden = false;
    } finally {
      auditReadPending = false;
      scheduleAudit(nextDelay);
    }
  }

  function renderAuditEntries(wrap) {
    clear(wrap);
    const entries = auditEntries;
    if (entries.length === 0) {
      wrap.appendChild(el("div", { class: "muted small" }, "No entries yet."));
      return;
    }
    const counts = entries.reduce((acc, entry) => {
      const kind = typeof entry.kind === "string" ? entry.kind.slice(0, 80) : "unknown";
      acc.set(kind, (acc.get(kind) || 0) + 1);
      return acc;
    }, new Map());
    wrap.appendChild(el("div", { class: "muted small", style: "margin-bottom:8px;" },
      Array.from(counts.entries()).sort().map(([kind, count]) => `${kind}=${count}`).join(" · ")));
    wrap.appendChild(el("table", { class: "audit-table" },
      el("thead", {}, el("tr", {}, ...["Time", "Kind", "Primary", "Detail", "Extra"].map(name => el("th", {}, name)))),
      el("tbody", {}, ...entries.slice().reverse().map(auditRowFor))));
  }

  // ===== End of audit panel =====

  function resetSelect(sel) {
    while (sel.firstChild) sel.removeChild(sel.firstChild);
  }
  function addOption(sel, value, label, dataset) {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    if (dataset) Object.assign(opt.dataset, dataset);
    sel.appendChild(opt);
    return opt;
  }

  $("#scrim").addEventListener("click", () => closeActivePanel());
  if (isTauri) {
    $("#settings-btn").addEventListener("click", openSettingsPanel);
    $("#audit-btn").addEventListener("click", openAuditPanel);
    $("#settings-panel-close").addEventListener("click", closeSettingsPanel);
    $("#settings-cancel").addEventListener("click", closeSettingsPanel);
    $("#settings-form").addEventListener("submit", saveSettings);
    $("#settings-form").addEventListener("input", markSettingsDirty);
    $("#policy-save").addEventListener("click", savePolicy);
    $("#policy-reload").addEventListener("click", () => loadPolicy({ discardDraft: true }));
    $("#policy-editor").addEventListener("input", markPolicyDirty);
    $("#policy-editor").addEventListener("scroll", syncPolicyScroll);
    $("#audit-panel-close").addEventListener("click", closeAuditPanel);
    document.querySelectorAll("[data-audit-preserve]").forEach(button => button.addEventListener("click", preserveAuditHistory));
    document.addEventListener("visibilitychange", () => {
      clearTimeout(auditTimer);
      auditTimer = null;
      if (!document.hidden && auditVisible()) refreshAudit();
    });
    updatePolicyControls();
  } else {
    $("#settings-btn").hidden = true;
    $("#audit-btn").hidden = true;
  }
  // Escape closes the panel; Tab stays inside it (same rule as before).
  document.addEventListener("keydown", event => {
    if (event.defaultPrevented || !activePanel) return;
    if (event.key === "Tab") {
      const controls = focusableWithin(activePanel);
      const first = controls[0] || activePanel, last = controls[controls.length - 1] || activePanel;
      if (!activePanel.contains(document.activeElement) || (event.shiftKey && document.activeElement === first)
          || (!event.shiftKey && document.activeElement === last)) {
        event.preventDefault();
        focusSafely(event.shiftKey ? last : first);
      }
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeActivePanel();
    }
  }, true);
  document.addEventListener("focusin", event => {
    if (activePanel && !activePanel.contains(event.target)) focusSafely(focusableWithin(activePanel)[0] || activePanel);
  });
  window.CloudBurritoPanels = { configure: partial => Object.assign(hooks, partial), loadSettings, openSettingsPanel,
    openAuditPanel, closeActivePanel, activePanel: () => activePanel };
})();
