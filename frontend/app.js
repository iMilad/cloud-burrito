// Cloud Burrito — frontend interactions
// Vanilla JS, no build step. Runs in any modern browser.

(function () {
  "use strict";

  const $ = (sel, root = document) => root.querySelector(sel);
  const el = (tag, attrs = {}, ...children) => {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === "class") node.className = v;
      else if (k === "dataset") Object.assign(node.dataset, v);
      else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
      else if (v !== undefined && v !== null && v !== false) node.setAttribute(k, v);
    }
    for (const c of children.flat()) {
      if (c == null || c === false) continue;
      node.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
    }
    return node;
  };
  const clear = (node) => { while (node.firstChild) node.removeChild(node.firstChild); };

  // ===== Display naming rule =====
  // Backend/widget payloads may use stable machine keys such as
  // `latest_version`, `execution-id`, or `callerARN`. UI labels must be human
  // readable: words separated, title-cased, and common cloud acronyms preserved.
  const DISPLAY_NAME_OVERRIDES = {
    arn: "ARN",
    aws: "AWS",
    cfn: "CFN",
    id: "ID",
    iam: "IAM",
    json: "JSON",
    sso: "SSO",
    ts: "Time",
    ui: "UI",
    url: "URL",
    yaml: "YAML",
  };
  const CAMEL_WORD_SPLIT = /([a-z0-9])([A-Z])/g;

  function displayName(value) {
    const raw = String(value ?? "").trim();
    if (!raw) return "";
    return raw
      .replace(CAMEL_WORD_SPLIT, "$1 $2")
      .replace(/[_./:-]+/g, " ")
      .split(/\s+/)
      .filter(Boolean)
      .map((part) => {
        const lower = part.toLowerCase();
        if (DISPLAY_NAME_OVERRIDES[lower]) return DISPLAY_NAME_OVERRIDES[lower];
        if (/^[A-Z0-9]{2,}$/.test(part)) return part;
        return lower.charAt(0).toUpperCase() + lower.slice(1);
      })
      .join(" ");
  }

  function tableHeaderLabel(column) {
    return displayName(column);
  }

  // Logical IDs are usually long PascalCase strings with a final CDK hash.
  // Add non-copying line-break opportunities at their semantic boundaries so
  // fixed-width tables wrap whole name segments before falling back mid-word.
  function breakableIdentifier(value) {
    const raw = String(value ?? "");
    const fragment = document.createDocumentFragment();
    const hash = raw.match(/[A-F0-9]{8}$/);
    const hashStart = hash && raw.length > 8 ? raw.length - 8 : -1;
    let segmentStart = 0;

    for (let i = 1; i < raw.length; i++) {
      const previous = raw[i - 1];
      const current = raw[i];
      const next = raw[i + 1] || "";
      const atHash = i === hashStart;
      const insideHash = hashStart >= 0 && i > hashStart;
      const camelBoundary = !insideHash && /[a-z0-9]/.test(previous) && /[A-Z]/.test(current);
      const acronymBoundary = !insideHash && /[A-Z]/.test(previous) && /[A-Z]/.test(current) && /[a-z]/.test(next);
      if (!atHash && !camelBoundary && !acronymBoundary) continue;
      fragment.appendChild(document.createTextNode(raw.slice(segmentStart, i)));
      fragment.appendChild(document.createElement("wbr"));
      segmentStart = i;
    }
    fragment.appendChild(document.createTextNode(raw.slice(segmentStart)));
    return fragment;
  }

  // ===== Backend handshake (Tauri ↔ Rust core) =====
  const isTauri = typeof window.__TAURI__ !== "undefined";
  const tauriInvoke = isTauri ? async (command, args) => {
    let result;
    try {
      result = await window.__TAURI__.core.invoke(command, args);
    } catch (_) {
      // Bridge failures can carry private native diagnostics. Resource data
      // remains in successful response models; unexpected errors stay fixed.
      throw new Error("Desktop request failed. Try again.");
    }
    observeDiagnostics(result);
    if (["settings_set", "dashboard_set", "policy_set"].includes(command) && result?.ok === false) {
      const errorType = ["StorageWriteFailed", "StorageSuperseded"].includes(result.error_type) ? result.error_type : "InvalidRequest";
      const error = new Error(errorType === "StorageWriteFailed"
        ? "Changes could not be saved. Try again."
        : errorType === "StorageSuperseded" ? "Save was superseded by a newer change. Review and retry if needed."
        : "Changes were rejected. Review the input and try again.");
      error.error_type = errorType;
      if (command === "settings_set") error.settings = result._settings;
      throw error;
    }
    return result;
  } : null;

  function observeDiagnostics(result) {
    if (result?._diagnostics?.audit_write_failed !== true || $("#audit-write-warning")) return;
    // A lost entry cannot be restored by a later successful write. Keep this
    // warning for the session, even if older responses arrive afterward.
    const warning = el("div", { id: "audit-write-warning", class: "diagnostics-warning", role: "status" },
      el("span", {}, "Some audit entries could not be saved. Inspection is still available."),
      el("button", { class: "btn btn-ghost small", type: "button", onclick: openAuditPanel }, "Open audit log"));
    document.body.appendChild(warning);
  }

  // When the page is opened directly in a browser (no Tauri shell), append a
  // small "browser mode" tag next to the brand name. In Tauri mode the tag
  // stays absent.
  function ensureBrowserModeTag() {
    const brand = $(".brand");
    if (!brand || brand.querySelector(".brand-tag")) return;
    brand.appendChild(el("span", { class: "brand-tag" }, "browser mode"));
  }

  async function pingCore() {
    const indicator = $("#core-status");
    const label = indicator.querySelector(".core-label");
    if (!isTauri) {
      indicator.dataset.state = "offline";
      label.textContent = "browser mode";
      ensureBrowserModeTag();
      return;
    }
    try {
      const result = await tauriInvoke("ping");
      indicator.dataset.state = "online";
      label.textContent = "core v" + (result.version || "?");
    } catch (e) {
      indicator.dataset.state = "offline";
      label.textContent = "core offline";
      console.warn("Backend ping failed.");
    }
  }

  // ===== Per-tile config (gear panel state) =====
  // Each tile's config lives on the .grid-stack-item via a JSON-serialised
  // `data-config` attribute. dashboard.json round-trips it verbatim. Shape:
  //   { context:     {mode: "inherit"} |
  //                  {mode: "pinned", profile, account_id, region},
  //     header_color: null | "blue" | "green" | "amber" | "pink" | "purple" | "red",
  //     inputs:       { <name>: <value>, ... } }
  // The backend is opaque about this — frontend owns the shape.
  const ALLOWED_HEADER_COLORS = ["blue", "green", "amber", "pink", "purple", "red"];
  const INHERIT_CONTEXT = { mode: "inherit", profile: null, account_id: null, region: null };
  const PIPELINE_PINS_KEY = "pinned_pipelines";

  function emptyConfig() {
    return { context: { ...INHERIT_CONTEXT }, header_color: null, inputs: {} };
  }

  function tileItemFor(el) {
    return el && el.closest && el.closest(".grid-stack-item");
  }

  function normalizeContext(context, legacyOverride) {
    const src = context && typeof context === "object" ? context : legacyOverride;
    if (src && typeof src === "object") {
      const mode = src.mode || (src.profile && src.account_id && src.region ? "pinned" : "inherit");
      if (mode === "pinned" && src.profile && src.account_id && src.region) {
        return {
          mode: "pinned",
          profile: String(src.profile),
          account_id: String(src.account_id),
          region: String(src.region),
        };
      }
    }
    return { ...INHERIT_CONTEXT };
  }

  function normalizeConfig(obj) {
    const raw = obj && typeof obj === "object" ? obj : {};
    const cfg = emptyConfig();
    cfg.context = normalizeContext(raw.context, raw.account_override);
    cfg.header_color = ALLOWED_HEADER_COLORS.includes(raw.header_color) ? raw.header_color : null;
    cfg.inputs = raw.inputs && typeof raw.inputs === "object" ? { ...raw.inputs } : {};
    const pins = normalizePipelinePins(cfg.inputs[PIPELINE_PINS_KEY]);
    if (pins.length) cfg.inputs[PIPELINE_PINS_KEY] = pins;
    else delete cfg.inputs[PIPELINE_PINS_KEY];
    return cfg;
  }

  function normalizePipelinePins(rawPins) {
    if (!Array.isArray(rawPins)) return [];
    const seen = new Set();
    const pins = [];
    rawPins.forEach((raw) => {
      if (!raw || typeof raw !== "object") return;
      const pipelineName = String(raw.pipeline_name || raw.pipelineName || "").trim();
      const profile = String(raw.profile || "").trim();
      const accountId = String(raw.account_id || raw.accountId || "").trim();
      const region = String(raw.region || "").trim();
      if (!pipelineName || !profile || !accountId || !region) return;
      const key = pipelinePinKey({ pipeline_name: pipelineName, profile, account_id: accountId, region });
      if (seen.has(key)) return;
      seen.add(key);
      pins.push({
        id: String(raw.id || key),
        pipeline_name: pipelineName,
        profile,
        account_id: accountId,
        region,
      });
    });
    return pins.slice(0, 50);
  }

  function pipelinePinKey(pin) {
    return [
      pin.profile || "",
      pin.account_id || "",
      pin.region || "",
      pin.pipeline_name || "",
    ].join("|");
  }

  function tileConfig(tileEl) {
    const item = tileItemFor(tileEl);
    if (!item) return emptyConfig();
    if (item._config) return item._config;
    let parsed = emptyConfig();
    if (item.dataset.config) {
      try {
        const obj = JSON.parse(item.dataset.config);
        parsed = normalizeConfig(obj);
      } catch (_) {}
    }
    item._config = parsed;
    return parsed;
  }

  function writeTileConfig(item, cfg) {
    item._config = normalizeConfig(cfg);
    item.dataset.config = JSON.stringify(item._config);
  }

  function widgetTypeForItem(item) {
    const widget = item && item.querySelector && item.querySelector(".widget");
    if (widget && widget.dataset.widget) return widget.dataset.widget;
    const explicit = item && item.getAttribute && item.getAttribute("data-widget-type");
    if (explicit) return explicit;
    const id = item && item.getAttribute && item.getAttribute("gs-id");
    return id ? id.split(":")[0] : "";
  }

  function widgetDisplayName(widgetType) {
    const known = prebuiltWidgets.find(w => w.gsId === widgetType);
    return known ? known.name : displayName(widgetType);
  }

  function tileById(id) {
    return Array.from(document.querySelectorAll(".grid-stack-item"))
      .find(item => item.getAttribute("gs-id") === id) || null;
  }

  function makeTileId(widgetType) {
    const random = Math.random().toString(36).slice(2, 8);
    return `${widgetType}:${Date.now().toString(36)}-${random}`;
  }

  function applyTileHeaderColor(item) {
    const widget = item.querySelector(".widget");
    if (!widget) return;
    const cfg = tileConfig(widget);
    if (cfg.header_color && ALLOWED_HEADER_COLORS.includes(cfg.header_color)) {
      widget.dataset.headerColor = cfg.header_color;
    } else {
      delete widget.dataset.headerColor;
    }
  }

  function applyAllTileColors() {
    document.querySelectorAll(".grid-stack-item").forEach(applyTileHeaderColor);
  }

  function contextForTile(tileEl) {
    const item = tileItemFor(tileEl);
    const cfg = item ? tileConfig(item) : emptyConfig();
    return cfg.context && cfg.context.mode === "pinned"
      ? cfg.context
      : { ...INHERIT_CONTEXT };
  }

  function contextPayloadForTile(tileEl) {
    return contextForTile(tileEl);
  }

  function effectivePinnedContextForTile(tileEl) {
    const tileCtx = contextForTile(tileEl);
    if (tileCtx.mode === "pinned") return tileCtx;
    if (topbarState.profile && topbarState.accountId && topbarState.region) {
      return {
        mode: "pinned",
        profile: topbarState.profile,
        account_id: topbarState.accountId,
        region: topbarState.region,
      };
    }
    return null;
  }

  function contextOverrideFromElement(node) {
    let cur = node;
    while (cur) {
      if (cur._widgetContextOverride) return cur._widgetContextOverride;
      cur = cur.parentNode;
    }
    return null;
  }

  function contextLabelForTile(item) {
    const ctx = contextForTile(item);
    if (ctx.mode === "pinned") {
      return `Pinned · ${ctx.profile} · ${ctx.account_id} · ${ctx.region}`;
    }
    const profile = topbarState.profile || "(none)";
    const account = topbarState.accountId || "(none)";
    const region = topbarState.region || "(none)";
    return `Default · ${profile} · ${account} · ${region}`;
  }

  function ensureContextChip(widget) {
    if (!widget) return null;
    const header = widget.querySelector(".widget-header");
    if (!header) return null;
    let chip = header.querySelector(".widget-context");
    if (!chip) {
      chip = el("span", { class: "widget-context" });
      const actions = header.querySelector(".widget-actions");
      if (actions) header.insertBefore(chip, actions);
      else header.appendChild(chip);
    }
    return chip;
  }

  function updateWidgetContextChip(item) {
    const widget = item && item.querySelector && item.querySelector(".widget");
    const chip = ensureContextChip(widget);
    if (!chip) return;
    const ctx = contextForTile(item);
    const label = contextLabelForTile(item);
    chip.textContent = label;
    chip.title = label;
    chip.dataset.mode = ctx.mode;
  }

  function updateAllWidgetContextChips() {
    document.querySelectorAll(".grid-stack-item").forEach(updateWidgetContextChip);
  }

  // A request belongs to a particular mounted surface, its ancestor lifetime,
  // and (unless explicitly pinned) one selection attempt. Equality of account
  // labels alone cannot distinguish A -> B -> A. Discarding a result here is
  // logical cancellation; it does not stop a remote query or subprocess.
  let nextOwnedRequestId = 0;
  let configurationGeneration = 0;

  function invalidateRequests(node, preserveResult = false) {
    if (!node) return;
    node._requestGeneration = (node._requestGeneration || 0) + 1;
    delete node._resultContext;
    if (!preserveResult) {
      delete node._resultView;
      if (node._resultHost) {
        delete node._resultHost._resultView;
        if (node._resultHost !== node) clear(node._resultHost);
      }
    }
  }

  function requestFailureLabel(result) {
    if (result?.partial || result?.status === "partial") return "Partial";
    if (result?.truncated || result?.coverage?.completeness === "limited") return "Limited";
    const outcome = result?._request?.outcome;
    if (outcome === "denied" || result?.render === "permission_denied") return "Denied";
    if (outcome === "cancelled") return "Cancelled";
    if (outcome === "failed" || result?.ok === false || result?.error
        || (result?.render === "raw_json" && result.data?.error)) return "Failed";
    return "";
  }

  function beginOwnedRequest(owner, contextOverride) {
    if (!owner) throw new Error("A request owner is required.");
    invalidateRequests(owner, true);
    const id = `ui-${++nextOwnedRequestId}`;
    const ancestors = [];
    for (let node = owner; node; node = node.parentNode) {
      ancestors.push([node, node._requestGeneration || 0]);
    }
    const independent = !!owner.closest?.(".pipeline-pin-card")
      || contextForTile(owner).mode === "pinned";
    const selection = currentSelectionId;
    const configuration = configurationGeneration;
    const context = { ...(contextOverride || contextOverrideFromElement(owner) || contextPayloadForTile(owner)) };
    const expected = context.mode === "pinned" ? context : {
      profile: topbarState.profile, account_id: topbarState.accountId, region: topbarState.region,
    };
    const request = {
      id, owner, context, freshnessContext: JSON.stringify([configuration, independent ? "pinned" : selection, expected]),
      allowed: (!isTauri || settingsStorageReady) && (independent || (discoveryReady && lastSetAccountResult?.ok === true)),
      current() {
        return owner.isConnected && owner._ownedRequest === request
          && configuration === configurationGeneration
          && (independent || selection === currentSelectionId)
          && ancestors.every(([node, generation]) => (node._requestGeneration || 0) === generation
            && (!node._evidenceSourceCurrent || node._evidenceSourceCurrent()));
      },
      accept(result) {
        if (!request.current()) return false;
        const meta = result && result._request;
        if (!meta || meta.id !== id) throw new Error("Response request identity did not match.");
        const failed = result.render === "permission_denied"
          || (result.render === "raw_json" && !!result.data?.error)
          || (!result.render && (result.ok === false || !!result.error));
        if (!failed && ["context_id", "provider_revision", "settings_revision"].some(key => typeof meta[key] !== "string" || !meta[key])) {
          throw new Error("Response verified context was missing.");
        }
        if (meta.context_id !== null && ["profile", "account_id", "region"].some(key => meta[key] !== expected[key])) {
          throw new Error("Response AWS context did not match.");
        }
        owner._resultContext = { ...meta };
        return true;
      },
    };
    owner._ownedRequest = request;
    return request;
  }

  async function fetchWidgetData(widgetName, inputs, owner, contextOverride, request) {
    if (!isTauri) return Promise.resolve(null);
    if (!request) throw new Error("An owned widget request is required.");
    if (!request.current()) return Promise.reject(new Error("Request owner is no longer current."));
    beginResultRequest(request, widgetName, inputs || {});
    if (!request.allowed) return Promise.reject(new Error("Select and verify an AWS account first."));
    if (widgetName === "aws-cli") {
      const available = await checkCliAvailability();
      if (!request.current()) throw new Error("Request owner is no longer current.");
      if (!available) throw new Error("AWS CLI is unavailable. Use Retry CLI check in this widget.");
    }
    return tauriInvoke("widget_fetch", {
      params: { widget: widgetName, inputs: inputs || {}, context: request.context, request_id: request.id },
    });
  }

  function resultHostForRequest(request) {
    const owner = request.owner;
    if (owner.matches?.(".pipeline-pin-card")) return $(".pipeline-pin-result", owner) || owner;
    if (owner.matches?.("input")) {
      if (!owner._resultHost?.isConnected) {
        owner._resultHost = el("span", { class: "result-input-status small" });
        owner.after(owner._resultHost);
      }
      return owner._resultHost;
    }
    return owner;
  }

  function beginResultRequest(request, widgetName, inputs) {
    const host = resultHostForRequest(request);
    request.owner._resultHost = host;
    request.resultHost = host;
    request.resultKey = JSON.stringify([request.freshnessContext, widgetName, inputs]);
    host._activeResultRequest = request;
    const previous = host._resultView;
    if (previous?.key !== request.resultKey) {
      delete host._resultView;
      clear(host);
    }
    if (host._resultView) {
      clear(host);
      host.append(...host._resultView.nodes);
      host.hidden = false;
      host._resultView.displayState = "loading";
      delete host._resultView.displaySpec;
      delete host._resultView.displayMessage;
    }
    host.hidden = false;
    paintResultStatus(host, "loading", null, host._resultView);
  }

  function validCoverage(coverage, depth = 0) {
    if (coverage === undefined) return true;
    if (!coverage || typeof coverage !== "object" || Array.isArray(coverage) || depth > 3
        || !["complete", "limited", "unknown"].includes(coverage.completeness)
        || !(coverage.has_more === null || typeof coverage.has_more === "boolean")) return false;
    for (const key of ["counts", "limits"]) {
      const entries = coverage[key];
      if (!entries || typeof entries !== "object" || Array.isArray(entries) || Object.keys(entries).length > 24) return false;
      if (Object.entries(entries).some(([name, value]) => !/^[a-z][a-z0-9_]{0,63}$/.test(name)
          || !(key === "limits" && value === null) && !(Number.isSafeInteger(value) && value >= 0))) return false;
    }
    if (!Array.isArray(coverage.reasons) || coverage.reasons.length > 32 || coverage.reasons.some(reason => !reason
        || typeof reason.code !== "string" || !/^[a-z][a-z0-9_]{0,63}$/.test(reason.code)
        || typeof reason.message !== "string" || reason.message.length > 500)) return false;
    return coverage.sections === undefined || (coverage.sections && typeof coverage.sections === "object"
      && !Array.isArray(coverage.sections) && Object.keys(coverage.sections).length <= 12
      && Object.entries(coverage.sections).every(([name, section]) => /^[a-z][a-z0-9_]{0,63}$/.test(name) && validCoverage(section, depth + 1)));
  }

  function resultHasEvidence(spec) {
    if (!spec || typeof spec !== "object") return false;
    if (["rows", "resources", "events", "actions", "matches", "versions", "functions", "groups", "streams", "pipelines"].some(key => Array.isArray(spec[key]) && spec[key].length)) return true;
    return spec.render === "raw_json" && spec.data !== undefined && spec.data !== null
      && !spec.data?.error && (typeof spec.data !== "object" || Object.keys(spec.data).length > 0);
  }

  function resultFailure(spec) {
    return !!(spec?.error || spec?.data?.error || spec?.ok === false || spec?.status === "failed" || spec?.render === "permission_denied"
      || ["failed", "denied", "cancelled"].includes(spec?._request?.outcome));
  }

  function resultState(spec) {
    if (spec?.partial === true || spec?.status === "partial" || (resultFailure(spec) && resultHasEvidence(spec))) return "partial";
    if (spec?._request?.outcome === "cancelled" || ["Superseded", "Cancelled", "QueryCancelled"].includes(spec?.error_type || spec?.data?.error_type)) return "cancelled";
    if (spec?.render === "permission_denied" || spec?._request?.outcome === "denied" || spec?.error_type === "PolicyDenied") return "denied";
    if (spec?.error_type === "CredentialsExpired" || spec?.needs_sso_login) return "expired";
    if (resultFailure(spec)) return "failed";
    if (spec?.truncated || spec?.coverage?.completeness === "limited") return "limited";
    return resultHasEvidence(spec) ? "success" : "empty";
  }

  function appendCoverage(status, coverage, label = "Coverage") {
    if (!coverage) { status.appendChild(el("div", {}, `${label}: unknown.`)); return; }
    const counts = Object.entries(coverage.counts).map(([key, value]) => `${displayName(key)}: ${value}`);
    const limits = Object.entries(coverage.limits).map(([key, value]) => `${displayName(key)} limit: ${value === null ? "unknown" : value}`);
    const more = coverage.has_more === true ? "Continuation or omitted results reported." : coverage.has_more === false ? "No continuation reported." : "More results: unknown.";
    status.appendChild(el("div", {}, `${label}: ${coverage.completeness}. ${[...counts, ...limits, more].join(" · ")}`));
    coverage.reasons.forEach(reason => status.appendChild(el("div", {}, reason.message)));
    Object.entries(coverage.sections || {}).forEach(([name, section]) => appendCoverage(status, section, displayName(name)));
  }

  function paintResultStatus(host, state, spec, view, message) {
    host.querySelectorAll(":scope > .result-status").forEach(node => node.remove());
    const labels = { loading: "Loading", success: "Updated", empty: "Empty response", limited: "Limited result", partial: "Partial result", stale: "Stale evidence", denied: "Denied", expired: "Credentials expired", failed: "Failed", cancelled: "Cancelled" };
    const status = el("div", { class: "result-status small", role: "status", "data-state": state },
      el("strong", {}, labels[state] || "Result"));
    if (view?.receivedAt) status.appendChild(el("time", { datetime: view.receivedAt, class: "result-received" }, `Received ${new Date(view.receivedAt).toLocaleString()}`));
    const context = view?.context || (spec?._request?.context_id ? spec._request : null);
    if (context) status.appendChild(el("div", { class: "result-context" }, `Profile: ${context.profile} · Verified account: ${context.account_id} · Region: ${context.region}`));
    if (state === "loading" && view) status.appendChild(el("div", {}, "Showing the previous result while refreshing."));
    if (state === "stale") {
      status.appendChild(el("div", {}, "Refresh did not complete. Retained evidence keeps its original timestamp."));
      status.appendChild(el("div", {}, `Refresh outcome: ${labels[resultState(spec || { ok: false })]}.`));
    }
    if (message) status.appendChild(el("div", { class: "result-failure" }, message));
    if (view && ["loading", "stale"].includes(state)) {
      status.appendChild(el("div", {}, `Retained result: ${labels[view.state] || "Updated"}.`));
      appendCoverage(status, view.coverage, "Retained coverage");
    }
    if (spec && state !== "loading") appendCoverage(status, spec.coverage, state === "stale" ? "Refresh coverage" : "Coverage");
    const cleanup = spec?.cleanup || spec?.data?.cleanup;
    if (cleanup) {
      const confirmed = cleanup.status === "stopped" && cleanup.remote_stop_confirmed === true;
      const labels = { stopped: "Remote stop was not confirmed.", not_confirmed: "Remote stop was not confirmed.", denied: "Remote stop was denied.", failed: "Remote stop failed.", not_attempted: "Remote stop was not attempted." };
      status.appendChild(el("div", { class: "result-cleanup" }, confirmed ? "Remote query stop confirmed." : labels[cleanup.status] || "Remote query status is unknown."));
      if (!confirmed && cleanup.remote_queries_may_still_run === true) status.appendChild(el("div", {}, "Remote queries may still be running."));
    } else if (state === "cancelled" || resultState(spec) === "cancelled") status.appendChild(el("div", { class: "result-cleanup" }, "The request is no longer awaited. This does not confirm that remote work stopped."));
    host.prepend(status);
    updateEvidenceLinks(host);
  }

  // Handoffs are reviewed backend facts, not names from which the UI guesses
  // another resource. The source request and its verified context stay attached
  // to every nested view; an inherited tile still expires with its selection.
  const HANDOFF_SOURCES = new Set([
    "pipeline_build_execution", "pipeline_stack_configuration", "codebuild_logs", "cfn_log_group",
    "cfn_ownership", "lambda_logging_config", "lambda_default_convention", "relationship_unknown",
    "context_mismatch", "unsupported_action", "identifier_unavailable", "ownership_ambiguous",
    "ownership_failed", "ownership_not_found", "ownership_unsupported", "ownership_not_attempted", "ownership_denied",
  ]);

  function evidenceOrigin(host, spec) {
    const request = host._activeResultRequest;
    const meta = spec?._request;
    if (!request || !meta || ["context_id", "provider_revision", "settings_revision", "profile", "account_id", "region"]
      .some(key => typeof meta[key] !== "string" || !meta[key])) return null;
    const context = { mode: "pinned", profile: meta.profile, account_id: meta.account_id, region: meta.region };
    return { context, current: () => request.current() && request.owner._resultContext?.id === meta.id };
  }

  function reviewedHandoff(value, kind) {
    if (!value || !["available", "manual", "unavailable"].includes(value.status)
        || !HANDOFF_SOURCES.has(value.source) || typeof value.reason !== "string" || value.reason.length > 500
        || !value.inputs || typeof value.inputs !== "object" || Array.isArray(value.inputs)) return null;
    if (value.status === "unavailable") return { ...value, widget: null, inputs: {} };
    const fields = {
      "codebuild-log": ["build_id"], "cfn-stack-detail": ["stack_name"],
      "resource-lookup": [], "log-tail": ["mode", "log_group", "log_stream"],
    }[value.widget];
    const routes = { build: ["codebuild-log"], stack: ["cfn-stack-detail", "resource-lookup"], logs: ["log-tail"] };
    if (!fields || !routes[kind]?.includes(value.widget) || Object.keys(value.inputs).some(key => !fields.includes(key))) return null;
    const text = name => typeof value.inputs[name] === "string" && value.inputs[name].length > 0
      && value.inputs[name].length <= 2048 && !/[\u0000-\u001f\u007f]/u.test(value.inputs[name]);
    if (value.widget === "codebuild-log" && !text("build_id")) return null;
    if (value.widget === "cfn-stack-detail" && !text("stack_name")) return null;
    if (value.widget === "log-tail" && (!text("log_group") || !["streams", "events"].includes(value.inputs.mode)
        || (value.inputs.mode === "events" && !text("log_stream"))
        || (value.inputs.mode === "streams" && Object.hasOwn(value.inputs, "log_stream")))) return null;
    return { ...value, inputs: { ...value.inputs } };
  }

  function updateEvidenceLinks(host) {
    if (!host) return;
    host.querySelectorAll(".evidence-handoff").forEach(card => {
      if (!card._evidenceOrigin) return;
      const current = card._evidenceOrigin.current();
      card.querySelectorAll(":scope > .evidence-open").forEach(button => { button.disabled = !current; });
      if (!current) card.querySelectorAll("button, input, select").forEach(control => { control.disabled = true; });
      const stale = card.querySelector(":scope > .evidence-stale");
      if (stale) stale.hidden = current;
    });
    host.querySelectorAll(".evidence-navigation").forEach(controls => {
      const current = controls._evidenceCurrent?.() === true;
      controls.querySelectorAll("button, select").forEach(control => { control.disabled = !current; });
      const stale = controls.querySelector(".evidence-stale");
      if (stale) stale.hidden = current;
    });
  }

  function appendEvidenceHandoffs(container, handoffs, sourceHost, sourceSpec) {
    if (!handoffs || typeof handoffs !== "object" || Array.isArray(handoffs)) return;
    const origin = evidenceOrigin(sourceHost, sourceSpec);
    for (const kind of ["build", "stack", "logs"]) {
      if (!Object.hasOwn(handoffs, kind)) continue;
      const target = reviewedHandoff(handoffs[kind], kind);
      const card = el("div", { class: "evidence-handoff", "data-kind": kind,
        "data-source": target?.source || "invalid" },
        el("strong", { class: "small" }, { build: "Build execution", stack: "Stack relationship", logs: "Log evidence" }[kind]),
        el("div", { class: "small evidence-reason" }, target?.reason || "Linked evidence is unavailable: the response did not establish a reviewed target."));
      if (!target?.widget || !origin) { container.appendChild(card); continue; }
      card._evidenceOrigin = origin;
      card.appendChild(el("div", { class: "small muted evidence-context" },
        `Source context: ${origin.context.profile} · ${origin.context.account_id} · ${origin.context.region}`));
      const label = target.widget === "codebuild-log" ? "View log"
        : target.widget === "resource-lookup" ? "Find a stack manually"
        : target.widget === "cfn-stack-detail" ? target.status === "manual" ? "Inspect stack candidate"
          : target.source === "pipeline_stack_configuration" ? "Inspect configured stack" : "Open matched stack"
        : target.source === "lambda_default_convention" ? "Try conventional log group"
        : target.inputs.mode === "events" ? "Open log events" : "Browse log streams";
      const button = el("button", { class: "exec-btn small evidence-open", type: "button", "aria-expanded": "false" }, label);
      const stale = el("div", { class: "small evidence-stale", hidden: true }, "Refresh the source before opening linked evidence.");
      const nested = el("div", { class: `evidence-result${target.widget === "codebuild-log" ? " exec-log" : ""}`, hidden: true });
      // This override is ephemeral. It never changes a tile's saved inherit/pin setting.
      nested._widgetContextOverride = origin.context;
      nested._evidenceSourceCurrent = origin.current;
      button.addEventListener("click", () => {
        if (!origin.current() || !button.isConnected) { updateEvidenceLinks(card.parentNode); return; }
        if (button.getAttribute("aria-expanded") === "true") {
          invalidateRequests(nested);
          clear(nested);
          nested.hidden = true;
          button.setAttribute("aria-expanded", "false");
          button.textContent = label;
          return;
        }
        button.setAttribute("aria-expanded", "true");
        button.textContent = target.widget === "codebuild-log" ? "Hide log" : "Close linked evidence";
        nested.hidden = false;
        if (target.status === "manual") nested.appendChild(el("div", { class: "small evidence-choice" },
          target.widget === "resource-lookup" ? "Manual investigation in the source context. A search match does not establish a relationship to this build."
            : "User-selected candidate. This does not establish ownership or a relationship to the source."));
        const content = el("div", { class: "evidence-content" });
        nested.appendChild(content);
        if (target.widget === "resource-lookup") return renderEvidenceLookup(content, origin);
        if (target.widget === "log-tail" && target.inputs.mode === "streams") return loadEvidenceStreams(content, target.inputs, origin);
        return fetchWidgetInto(content, target.widget, target.inputs, origin.context);
      });
      card.append(button, stale, nested);
      container.appendChild(card);
    }
    updateEvidenceLinks(container);
  }

  function renderEvidenceLookup(host, origin) {
    const input = el("input", { class: "lookup-input", type: "search", "aria-label": "Resource query in source context",
      placeholder: "Resource name, ARN or partial id", maxlength: "2048" });
    const search = el("button", { class: "exec-btn", type: "submit" }, "Search resources");
    const form = el("form", { class: "evidence-lookup-form" }, input, search);
    const results = el("div", { class: "lookup-results evidence-lookup-results" });
    form.addEventListener("input", () => { invalidateRequests(results); clear(results); });
    form.addEventListener("submit", event => {
      event.preventDefault();
      if (!origin.current() || !form.isConnected) return;
      const query = input.value.trim();
      if (!query) { clear(results); results.appendChild(el("div", { class: "small" }, "Enter a resource query first.")); input.focus(); return; }
      return fetchWidgetInto(results, "resource-lookup", { query }, origin.context);
    });
    const stackInput = el("input", { type: "text", class: "evidence-stack-input", "aria-label": "Stack name or ARN for manual inspection",
      placeholder: "Stack name or ARN", maxlength: "2048" });
    const stackOutput = el("div", { class: "evidence-manual-stack" });
    const stackForm = el("form", { class: "evidence-lookup-form" }, stackInput,
      el("button", { class: "exec-btn", type: "submit" }, "Inspect chosen stack"));
    stackForm.addEventListener("input", () => { invalidateRequests(stackOutput); clear(stackOutput); });
    stackForm.addEventListener("submit", event => {
      event.preventDefault();
      if (!origin.current() || !stackForm.isConnected) return;
      const stackName = stackInput.value.trim();
      if (!stackName) { stackInput.focus(); return; }
      return fetchWidgetInto(stackOutput, "cfn-stack-detail", { stack_name: stackName }, origin.context);
    });
    host.append(form, results,
      el("div", { class: "small evidence-choice" }, "Or inspect a stack you choose. Your selection does not establish a relationship to the source build."),
      stackForm, stackOutput);
    input.focus();
  }

  async function loadEvidenceStreams(host, inputs, origin) {
    if (!origin.current()) return;
    const request = beginOwnedRequest(host, origin.context);
    try {
      const result = await fetchWidgetData("log-tail", { ...inputs, max_streams: 50 }, host, origin.context, request);
      if (!request.accept(result)) return;
      renderArrayResult(host, result, "streams", streams => {
        clear(host);
        host.appendChild(el("div", { class: "small evidence-log-group" }, `Log group: ${inputs.log_group}`));
        if (!streams.length) { host.appendChild(el("div", { class: "small" }, "No streams were returned for this log group.")); return; }
        const select = el("select", { "aria-label": "Linked log stream", class: "evidence-stream-select" });
        streams.forEach(stream => { if (typeof stream.name === "string" && stream.name) select.appendChild(el("option", { value: stream.name }, stream.name)); });
        const view = el("button", { class: "exec-btn", type: "button" }, "View linked log");
        const output = el("div", { class: "evidence-log-output log-stream" });
        select.addEventListener("change", () => { invalidateRequests(output); clear(output); });
        view.addEventListener("click", () => {
          if (!origin.current() || !request.current() || !view.isConnected || !select.value) return;
          return fetchWidgetInto(output, "log-tail", { mode: "events", log_group: inputs.log_group, log_stream: select.value, limit: 500 }, origin.context);
        });
        host.append(el("div", { class: "evidence-stream-controls" }, select, view), output);
      });
    } catch (error) { if (request.current()) failResult(host, request, String(error)); }
  }

  function failResult(host, request, message, spec) {
    if (!request?.current()) return;
    const previous = host._resultView?.key === request.resultKey ? host._resultView : null;
    clear(host);
    if (previous) host.append(...previous.nodes);
    else host.appendChild(el("div", { class: "muted small" }, message));
    host.hidden = false;
    if (previous) {
      previous.displayState = "stale";
      previous.displaySpec = { coverage: spec?.coverage, error_type: spec?.error_type, _request: spec?._request, cleanup: spec?.cleanup || spec?.data?.cleanup, ok: false };
      previous.displayMessage = message;
    }
    paintResultStatus(host, previous ? "stale" : resultState(spec || { ok: false }), spec, previous, message);
  }

  function renderWithResultState(host, spec, render) {
    const request = host._activeResultRequest;
    if (!request || !spec?._request || host._renderingResult) return render();
    if (!request.current() || spec._request.id !== request.id) return;
    if (!validCoverage(spec.coverage)) return failResult(host, request, "Response coverage metadata was invalid.");
    if (!validResultShape(spec)) return failResult(host, request, "Response data shape was invalid.");
    const previous = host._resultView?.key === request.resultKey ? host._resultView : null;
    if (resultFailure(spec) && !resultHasEvidence(spec) && previous) {
      return failResult(host, request, spec.error || spec.data?.error || spec.reason || "The refresh did not complete.", spec);
    }
    host._renderingResult = true;
    try { render(); } finally { host._renderingResult = false; }
    const state = resultState(spec);
    const usable = !resultFailure(spec) || resultHasEvidence(spec);
    const view = { key: request.resultKey, receivedAt: new Date().toISOString(), context: spec._request, state, coverage: spec.coverage,
      nodes: Array.from(host.childNodes).filter(node => !node.classList?.contains("result-status")) };
    if (usable) host._resultView = view;
    else delete host._resultView;
    host.hidden = false;
    paintResultStatus(host, state, spec, usable ? view : null);
  }

  function validResultShape(spec) {
    const required = { table: ["rows", "columns"], errors_chart: ["rows"], log_stream: ["events"],
      reverse_lookup: ["matches"], execution_detail: ["actions"], stack_detail: ["resources", "events"],
      codeartifact_version_history: ["versions"], raw_json: [], permission_denied: [] };
    if (spec.render && !Object.hasOwn(required, spec.render)) return false;
    const keys = spec.render ? required[spec.render] : ["functions", "groups", "streams", "pipelines"].filter(key => Object.hasOwn(spec, key));
    if (!spec.render && !keys.length && !resultFailure(spec)) return false;
    return keys.every(key => Array.isArray(spec[key]) && (key === "columns"
      ? spec[key].every(value => typeof value === "string")
      : spec[key].every(value => value && typeof value === "object" && !Array.isArray(value))));
  }

  function renderArrayResult(host, result, key, render) {
    if (!resultFailure(result) && !Array.isArray(result[key])) return failResult(host, host._activeResultRequest, "Response data shape was invalid.");
    if (result.render) return dispatchRender(host, result);
    return renderWithResultState(host, result, () => {
      if (!Array.isArray(result[key])) {
        clear(host);
        host.appendChild(el("div", { class: "muted small" }, result.error || "The request did not complete."));
        return;
      }
      render(result[key]);
      if (!result[key].length && resultFailure(result)) {
        clear(host);
        host.appendChild(el("div", { class: "muted small" }, "Results are unavailable or incomplete."));
      }
    });
  }

  // Local filtering and row selection change presentation, not receipt time.
  function restoreResultStatus(host) {
    const view = host._resultView;
    if (!view || host._renderingResult) return;
    view.nodes = Array.from(host.childNodes).filter(node => !node.classList?.contains("result-status"));
    paintResultStatus(host, view.displayState || view.state, view.displaySpec || { coverage: view.coverage }, view, view.displayMessage);
  }

  function reuseResultBrowser(body) {
    if (body._reloadResult && body.firstElementChild && body._browserGeneration === (body._requestGeneration || 0)) {
      body._reloadResult();
      return true;
    }
    return false;
  }

  function sectionEmptyMessage(spec, section, emptyMessage) {
    const coverage = spec.coverage?.sections?.[section];
    const denied = coverage?.reasons.some(reason => reason.code === "policy_denied");
    const failed = coverage?.reasons.some(reason => reason.code === "request_failed");
    if (denied) return `${displayName(section)} were denied; no data is available for this section.`;
    if (failed || (!coverage && resultFailure(spec))) return `${displayName(section)} could not be loaded; this is not an empty result.`;
    return emptyMessage;
  }

  async function fetchWidgetInto(host, widgetName, inputs, contextOverride) {
    const request = beginOwnedRequest(host, contextOverride);
    try {
      const result = await fetchWidgetData(widgetName, inputs, host, contextOverride, request);
      if (request.accept(result)) dispatchRender(host, result);
    } catch (error) {
      if (request.current()) renderError(host, String(error));
    }
  }

  function clearWidgetResults(widget) {
    // Keep explicit pin cards mounted; their identity and request lifetime do
    // not belong to the topbar. Form controls remain available for retry.
    widget.querySelectorAll(".widget-data, .lookup-results, .log-tail-body, .cw-logs-body, .cfn-stacks-body, .errors-body").forEach(host => {
      if (host.closest(".pipeline-pin-card")) return;
      if (host.classList.contains("widget-data")) {
        host._reloadAfterSelection = host._reloadAfterSelection || !host.hidden;
      }
      invalidateRequests(host);
      clear(host);
      if (host.classList.contains("widget-data")) host.hidden = true;
    });
    widget.querySelectorAll(".pipeline-error, .aws-cli-error, .logs-insights-error, .codeartifact-packages-error").forEach(node => { node.textContent = ""; });
    widget.querySelectorAll("datalist, .pipeline-name-select").forEach(node => { invalidateRequests(node); clear(node); });
    widget.querySelectorAll(".li-group").forEach(node => { invalidateRequests(node); delete node.dataset.loaded; });
    widget.querySelectorAll(".pipeline-name-search").forEach(node => {
      invalidateRequests(node);
      node._pipelineNames = [];
      node.value = "";
      node.disabled = true;
      node.placeholder = "(verify an account to list pipelines)";
    });
    const subtitle = $(".widget-sub", widget);
    if (subtitle) subtitle.textContent = "";
  }

  function clearInheritedResults() {
    closePipelineCombo();
    document.querySelectorAll(".widget").forEach(widget => {
      if (contextForTile(widget).mode !== "pinned") clearWidgetResults(widget);
    });
  }

  function wireRequestInputs(form, owner, errorEl) {
    if (form._requestInputsWired) return;
    form._requestInputsWired = true;
    form.addEventListener("input", () => {
      owner._reloadAfterSelection = false;
      invalidateRequests(owner);
      clear(owner);
      owner.hidden = true;
      if (errorEl) errorEl.textContent = "";
    });
  }

  // ===== Settings (Tauri only) =====
  let cachedSettings = null;
  let settingsStorageReady = !isTauri;
  let settingsLoadId = 0;
  let settingsDraftRevision = 0;
  let settingsDraftDirty = false;
  let settingsReadFailed = false;
  let settingsDefaults = null;
  let desktopPickersStarted = false;
  let bootComplete = false;
  // Browser-only demonstration regions. Desktop choices come from Rust.
  let allowedRegions = isTauri ? [] : ["eu-west-1", "us-east-1"];
  const SETTINGS_FIELDS = {
    aws_config_path: "settings-aws-config-path", sso_session_name: "settings-sso-session",
    default_profile: "settings-default-profile", default_region: "settings-default-region", theme: "settings-theme",
  };

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
        || Object.keys(SETTINGS_FIELDS).some(key => typeof metadata.defaults[key] !== "string")
        || !["dark", "light"].includes(metadata.defaults.theme)
        || !Array.isArray(metadata.allowed_regions) || !metadata.allowed_regions.length
        || metadata.allowed_regions.length > 128
        || metadata.allowed_regions.some(region => typeof region !== "string" || !/^[a-z0-9-]{1,64}$/.test(region))
        || !metadata.field_errors || typeof metadata.field_errors !== "object" || Array.isArray(metadata.field_errors)) {
      throw new Error("Settings metadata unavailable");
    }
    settingsDefaults = { ...metadata.defaults };
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
    $("#appearance-unsaved").hidden = !isTauri || !cachedSettings || theme === cachedSettings.theme;
    $("#theme-toggle").setAttribute("aria-label", `Preview ${theme === "dark" ? "light" : "dark"} theme`);
  }

  function markSettingsDirty() {
    settingsDraftRevision++;
    settingsDraftDirty = true;
    if (!settingsSavePending && !settingsReadFailed) $("#settings-status").textContent = "Current edits are not saved.";
    updateThemePreview();
  }

  function startDesktopPickers() {
    if (!isTauri || !bootComplete || !settingsStorageReady) return;
    if (desktopPickersStarted) {
      if (connectionState === "settings_failed") initTopbarPickers();
      return;
    }
    desktopPickersStarted = true;
    initTopbarPickers();
    startAuthStatusPolling();
  }

  function showStorageLoadWarning(store, retry, recover) {
    const id = `${store}-storage-warning`;
    let warning = $("#" + id);
    if (!warning) {
      warning = el("div", { id, class: "storage-load-warning", role: "status" },
        el("span", {}, `${store === "settings" ? "Settings" : "Dashboard"} could not be loaded. Saved data has not been replaced.`),
        el("button", { class: "btn btn-ghost small", type: "button", onclick: retry }, "Retry load"),
        el("button", { class: "btn btn-ghost small", type: "button", onclick: recover },
          store === "settings" ? "Replace settings with defaults" : "Reset layout"));
      $(".dashboard").before(warning);
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
          || Object.keys(SETTINGS_FIELDS).some(key => typeof loaded[key] !== "string")) throw new Error("Settings unavailable");
      cachedSettings = loaded;
      settingsStorageReady = true;
      $("#settings-storage-warning")?.remove();
      $("#settings-save").disabled = settingsSavePending;
      $("#theme-toggle").disabled = false;
      if (settingsReadFailed) $("#settings-status").textContent = settingsDraftDirty ? "Current edits are not saved." : "";
      settingsReadFailed = false;
      if (!settingsDraftDirty) fillSettingsForm(loaded);
      startDesktopPickers();
      return cachedSettings;
    } catch (e) {
      if (request !== settingsLoadId) return null;
      settingsStorageReady = false;
      settingsReadFailed = true;
      $("#settings-save").disabled = true;
      $("#theme-toggle").disabled = true;
      $("#settings-status").textContent = "Settings could not be loaded. Retry loading or explicitly replace them with defaults.";
      clearDiscoveredConnection("settings_failed");
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
    panel.classList.add("open");
    panel.setAttribute("aria-hidden", "false");
    $("#scrim").classList.add("open");
    $("#scrim").hidden = false;
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
  function closeSettingsPanel() {
    const panel = $("#settings-panel");
    panel.classList.remove("open");
    panel.setAttribute("aria-hidden", "true");
    $("#scrim").classList.remove("open");
    setTimeout(() => { $("#scrim").hidden = true; }, 220);
  }

  let settingsSavePending = false;
  async function saveSettings(e, options = {}) {
    e.preventDefault();
    if (!isTauri || settingsSavePending) return;
    if (!settingsStorageReady && !options.recover) return;
    // Send empty strings for blank inputs — the backend treats empty as
    // "fall back to default" so the user doesn't need to retype defaults.
    const params = options.recover ? {
      aws_config_path: "", sso_session_name: "", default_profile: "", default_region: "", theme: "",
    } : {
      aws_config_path:   $("#settings-aws-config-path").value.trim(),
      sso_session_name:  $("#settings-sso-session").value.trim(),
      default_profile:   $("#settings-default-profile").value.trim(),
      default_region:    $("#settings-default-region").value.trim(),
      theme:             $("#settings-theme").value,
    };
    const status = $("#settings-status");
    status.textContent = "Saving…";
    const configuration = configurationGeneration;
    const draft = settingsDraftRevision;
    const previous = cachedSettings;
    ++settingsLoadId; // An older read cannot replace a newly accepted save.
    settingsSavePending = true;
    $("#settings-save").disabled = true;
    try {
      const saved = await tauriInvoke("settings_set", { params });
      if (configuration !== configurationGeneration) return;
      consumeSettingsMetadata(saved?._settings);
      if (Object.keys(SETTINGS_FIELDS).some(key => typeof saved[key] !== "string")
          || !["dark", "light"].includes(saved.theme)) throw new Error("Settings save response was incomplete. Reload to verify.");
      ++settingsLoadId;
      cachedSettings = saved;
      settingsStorageReady = true;
      settingsReadFailed = false;
      $("#settings-storage-warning")?.remove();
      $("#theme-toggle").disabled = false;
      if (draft === settingsDraftRevision) {
        if (options.recover) fillSettingsForm(saved);
        settingsDraftDirty = false;
        status.textContent = "Saved.";
      } else {
        status.textContent = "Previous values saved. Current edits are not saved.";
      }
      updateThemePreview();
      startDesktopPickers();
      // Preferences affect the next default choice, not an already verified
      // context. Only the credential source changes its identity contract.
      const credentialsChanged = !previous || ["aws_config_path", "sso_session_name"].some(key => previous[key] !== saved[key]);
      if (!credentialsChanged) return;
      // Credential changes invalidate every old configuration even if the
      // user selected another account while saving. Reverify the current
      // picker selection below; never restore the submitted draft's account.
      ++configurationGeneration;
      ++currentSelectionId;
      lastSetAccountResult = null;
      lastAuthStatus = null;
      document.querySelectorAll(".widget").forEach(widget => {
        invalidateRequests(tileItemFor(widget));
        clearWidgetResults(widget);
        widget.querySelectorAll(".pipeline-pin-card").forEach(card => {
          invalidateRequests(card);
          delete card.dataset.loaded;
          const result = $(".pipeline-pin-result", card);
          if (result) { clear(result); result.textContent = "Refresh to verify the updated configuration."; }
          const stamp = $(".pipeline-pin-updated", card);
          if (stamp) stamp.textContent = "";
          const badge = $(".pipeline-pin-status", card);
          if (badge) { badge.className = "badge badge-neutral pipeline-pin-status"; badge.textContent = "Not refreshed"; }
        });
      });
      setPillInFlight("auth: configuration changed — verify account …");
      // Re-apply to the Pipeline Runs config inputs so the user doesn't see
      // stale placeholders the next time they look at it.
      prefillPipelineConfig(cachedSettings);
      refreshProfilesInBackground();
    } catch (err) {
      if (configuration !== configurationGeneration) return;
      if (err.settings) {
        try { consumeSettingsMetadata(err.settings); } catch (_) { /* Keep the last accepted catalogue. */ }
      }
      status.textContent = "Save failed: " + err;
    } finally {
      settingsSavePending = false;
      $("#settings-save").disabled = !settingsStorageReady;
    }
  }

  function applyPolicyStatus(s) {
    if (!s) return;
    const editor = $("#policy-editor");
    const status = $("#policy-status");
    const path = $("#policy-path");
    if (path) path.textContent = s.path || "";
    if (typeof s.raw === "string") {
      editor.value = s.raw;
      editor.scrollTop = 0;
      editor.scrollLeft = 0;
      renderPolicyHighlight();
    }
    if (s.valid) {
      const n = Array.isArray(s.actions) ? s.actions.length : 0;
      status.textContent = "✓ valid — " + n + " action(s) allowed";
      status.className = "small policy-ok";
    } else {
      status.textContent = "✗ " + (s.error || "invalid policy");
      status.className = "small policy-bad";
    }
  }

  async function loadPolicy() {
    if (!isTauri) return;
    try {
      applyPolicyStatus(await tauriInvoke("policy_get"));
    } catch (e) {
      $("#policy-status").textContent = "policy_get failed: " + e;
    }
  }

  async function savePolicy() {
    if (!isTauri) return;
    const text = $("#policy-editor").value;
    $("#policy-status").textContent = "Saving…";
    try {
      applyPolicyStatus(await tauriInvoke("policy_set", { params: { text } }));
    } catch (e) {
      $("#policy-status").textContent = "Save failed: " + e;
    }
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

  // The pipeline-runs tile no longer carries inline profile/account/region
  // inputs — those live in the per-widget gear panel now. Kept as a no-op so
  // callers (settings save, boot) don't have to know about the simplification.
  function prefillPipelineConfig(_s) { /* intentional no-op */ }

  // ===== Audit log panel (Tauri only) =====
  let auditTimer = null;

  function openAuditPanel() {
    const panel = $("#audit-panel");
    panel.classList.add("open");
    panel.setAttribute("aria-hidden", "false");
    $("#scrim").classList.add("open");
    $("#scrim").hidden = false;
    refreshAudit();
    if (auditTimer) clearInterval(auditTimer);
    // 2s gives a usable "live feed" feel without thrashing the core.
    auditTimer = setInterval(refreshAudit, 2000);
  }
  function closeAuditPanel() {
    const panel = $("#audit-panel");
    panel.classList.remove("open");
    panel.setAttribute("aria-hidden", "true");
    $("#scrim").classList.remove("open");
    setTimeout(() => { $("#scrim").hidden = true; }, 220);
    if (auditTimer) { clearInterval(auditTimer); auditTimer = null; }
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
        detail = entry.event === "query_cleanup" && Object.hasOwn(cleanupLabels, cleanup)
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
    if (!isTauri) return;
    const wrap = $("#audit-table-wrap");
    let warning = $("#audit-read-warning");
    if (!warning) {
      warning = el("p", { id: "audit-read-warning", class: "audit-read-warning small", role: "status", hidden: true });
      wrap.before(warning);
    }
    const showFailure = () => {
      warning.textContent = "Audit history could not be read. Previously displayed entries are unchanged.";
      warning.hidden = false;
    };
    let res;
    try {
      res = await tauriInvoke("audit_tail", { params: { limit: 300 } });
    } catch (e) {
      console.warn("Audit history could not be read.");
      showFailure();
      return;
    }
    if (!res || res.ok === false || !Array.isArray(res.entries)) {
      showFailure();
      return;
    }
    warning.hidden = true;
    clear(wrap);
    const entries = res.entries.filter(entry => entry && typeof entry === "object" && !Array.isArray(entry));
    if (entries.length === 0) {
      wrap.appendChild(el("div", { class: "muted small" }, "No entries yet."));
      return;
    }
    // Count summary at the top so the user gets an instant read.
    const counts = entries.reduce((acc, e) => {
      const kind = typeof e.kind === "string" ? e.kind : "unknown";
      acc.set(kind, (acc.get(kind) || 0) + 1);
      return acc;
    }, new Map());
    const summary = Array.from(counts.entries())
      .sort()
      .map(([k, n]) => `${k}=${n}`)
      .join(" · ");
    wrap.appendChild(el("div", { class: "muted small", style: "margin-bottom:8px;" }, summary));
    const table = el("table", { class: "audit-table" },
      el("thead", {}, el("tr", {},
        el("th", {}, "Time"),
        el("th", {}, "Kind"),
        el("th", {}, "Primary"),
        el("th", {}, "Detail"),
        el("th", {}, "Extra"),
      )),
      el("tbody", {},
        ...entries.slice().reverse().map(auditRowFor),
      ),
    );
    wrap.appendChild(table);
  }

  // ===== Searchable top bar pickers (backed by selects populated from ~/.aws/config) =====
  const topbarState = { profile: null, accountId: null, role: null, region: null };
  let topbarPickerList = null;
  let topbarPickerInput = null;
  let topbarPickerActiveIndex = -1;
  let topbarPickerGlobalsWired = false;

  function pickerInputForSelect(select) {
    if (!select || !select.id) return null;
    return document.querySelector(`.picker-search[data-select-id="${select.id}"]`);
  }

  function syncTopbarPicker(select) {
    const input = pickerInputForSelect(select);
    if (!input || !select) return;
    const option = select.options[select.selectedIndex] || select.options[0];
    const label = option ? option.textContent : "";
    input.disabled = select.disabled;
    input.dataset.committedLabel = label;
    input.title = label;
    if (topbarPickerInput !== input) input.value = label;
  }

  function matchingTopbarOptions(select, query) {
    const tokens = String(query || "").trim().toLowerCase().split(/\s+/).filter(Boolean);
    return Array.from(select.options)
      .map((option, index) => ({ option, index }))
      .filter(({ option }) => {
        if (!option.value || option.disabled) return false;
        const haystack = [option.textContent, option.value, ...Object.values(option.dataset)]
          .join(" ").toLowerCase();
        return tokens.every(token => haystack.includes(token));
      });
  }

  function ensureTopbarPickerList() {
    if (topbarPickerList) return topbarPickerList;
    topbarPickerList = el("ul", {
      id: "topbar-picker-list",
      class: "combo-list topbar-picker-list",
      role: "listbox",
      hidden: "",
    });
    document.body.appendChild(topbarPickerList);
    return topbarPickerList;
  }

  function positionTopbarPickerList(input) {
    if (!topbarPickerList || !input) return;
    const rect = input.getBoundingClientRect();
    const accountPicker = input.dataset.selectId === "account-select";
    const width = Math.min(window.innerWidth - 16, Math.max(rect.width, accountPicker ? 320 : 180));
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    const below = window.innerHeight - rect.bottom;
    const above = rect.top;
    topbarPickerList.style.left = `${left}px`;
    topbarPickerList.style.width = `${width}px`;
    if (below < 180 && above > below) {
      topbarPickerList.style.bottom = `${window.innerHeight - rect.top + 2}px`;
      topbarPickerList.style.top = "auto";
      topbarPickerList.style.maxHeight = `${Math.min(320, Math.max(80, above - 8))}px`;
    } else {
      topbarPickerList.style.top = `${rect.bottom + 2}px`;
      topbarPickerList.style.bottom = "auto";
      topbarPickerList.style.maxHeight = `${Math.min(320, Math.max(80, below - 8))}px`;
    }
  }

  function setTopbarPickerActive(index) {
    if (!topbarPickerList || !topbarPickerInput) return;
    const items = Array.from(topbarPickerList.querySelectorAll(".combo-item"));
    if (items.length === 0) {
      topbarPickerActiveIndex = -1;
      topbarPickerInput.removeAttribute("aria-activedescendant");
      return;
    }
    topbarPickerActiveIndex = Math.max(0, Math.min(index, items.length - 1));
    items.forEach((item, itemIndex) => {
      const active = itemIndex === topbarPickerActiveIndex;
      item.classList.toggle("active", active);
      item.setAttribute("aria-selected", String(active));
    });
    const active = items[topbarPickerActiveIndex];
    topbarPickerInput.setAttribute("aria-activedescendant", active.id);
    active.scrollIntoView({ block: "nearest" });
  }

  function renderTopbarPicker(input, query) {
    if (!input || input.disabled) return;
    const select = document.getElementById(input.dataset.selectId);
    if (!select) return;
    const list = ensureTopbarPickerList();
    const matches = matchingTopbarOptions(select, query);
    clear(list);
    matches.forEach(({ option, index }, visibleIndex) => {
      const item = el("li", {
        id: `topbar-picker-option-${visibleIndex}`,
        class: "combo-item",
        role: "option",
        dataset: { optionIndex: String(index) },
      }, option.textContent);
      item.addEventListener("mousedown", (event) => {
        event.preventDefault();
        chooseTopbarPickerOption(index);
      });
      item.addEventListener("click", () => chooseTopbarPickerOption(index));
      list.appendChild(item);
    });
    if (matches.length === 0) {
      list.appendChild(el("li", { class: "combo-empty" }, "No matches"));
    }
    const status = $("#topbar-picker-status");
    if (status) status.textContent = matches.length === 0
      ? "No matching options"
      : `${matches.length} matching option${matches.length === 1 ? "" : "s"}`;
    topbarPickerInput = input;
    topbarPickerActiveIndex = -1;
    positionTopbarPickerList(input);
    list.hidden = false;
    input.setAttribute("aria-expanded", "true");
    const selectedMatch = matches.findIndex(match => match.index === select.selectedIndex);
    setTopbarPickerActive(selectedMatch >= 0 ? selectedMatch : 0);
  }

  function closeTopbarPicker(restoreValue) {
    const input = topbarPickerInput;
    if (topbarPickerList) topbarPickerList.hidden = true;
    topbarPickerInput = null;
    topbarPickerActiveIndex = -1;
    if (!input) return;
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
    if (restoreValue) {
      const select = document.getElementById(input.dataset.selectId);
      syncTopbarPicker(select);
    }
  }

  function chooseTopbarPickerOption(optionIndex) {
    if (!topbarPickerInput) return;
    const input = topbarPickerInput;
    const select = document.getElementById(input.dataset.selectId);
    if (!select || !select.options[optionIndex]) return;
    select.selectedIndex = optionIndex;
    closeTopbarPicker(false);
    syncTopbarPicker(select);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function moveTopbarPickerActive(delta) {
    if (!topbarPickerList || topbarPickerList.hidden) {
      renderTopbarPicker(topbarPickerInput, "");
      return;
    }
    const count = topbarPickerList.querySelectorAll(".combo-item").length;
    if (count === 0) return;
    const next = topbarPickerActiveIndex < 0
      ? (delta > 0 ? 0 : count - 1)
      : (topbarPickerActiveIndex + delta + count) % count;
    setTopbarPickerActive(next);
  }

  function wireTopbarSearchablePicker(input) {
    if (!input || input.dataset.wired === "1") return;
    const select = document.getElementById(input.dataset.selectId);
    if (!select) return;
    input.dataset.wired = "1";
    select.addEventListener("change", () => syncTopbarPicker(select));
    input.addEventListener("focus", () => {
      input.value = "";
      renderTopbarPicker(input, "");
    });
    input.addEventListener("click", () => {
      if (!topbarPickerList || topbarPickerList.hidden || topbarPickerInput !== input) {
        input.value = "";
        renderTopbarPicker(input, "");
      }
    });
    input.addEventListener("input", () => renderTopbarPicker(input, input.value));
    input.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        if (topbarPickerInput !== input) {
          topbarPickerInput = input;
          renderTopbarPicker(input, "");
        }
        moveTopbarPickerActive(event.key === "ArrowDown" ? 1 : -1);
      } else if (event.key === "Home" || event.key === "End") {
        if (!topbarPickerList || topbarPickerList.hidden || topbarPickerInput !== input) return;
        event.preventDefault();
        const count = topbarPickerList.querySelectorAll(".combo-item").length;
        setTopbarPickerActive(event.key === "Home" ? 0 : count - 1);
      } else if (event.key === "Enter") {
        if (!topbarPickerList || topbarPickerList.hidden || topbarPickerInput !== input) {
          event.preventDefault();
          renderTopbarPicker(input, "");
          return;
        }
        const items = Array.from(topbarPickerList.querySelectorAll(".combo-item"));
        const active = items[topbarPickerActiveIndex];
        if (active) {
          event.preventDefault();
          chooseTopbarPickerOption(Number(active.dataset.optionIndex));
        }
      } else if (event.key === "Escape") {
        event.stopPropagation();
        closeTopbarPicker(true);
      } else if (event.key === "Tab") {
        closeTopbarPicker(true);
      }
    });
    input.addEventListener("blur", () => setTimeout(() => {
      if (topbarPickerInput === input) closeTopbarPicker(true);
    }, 120));
    syncTopbarPicker(select);
  }

  function wireTopbarSearchablePickers() {
    document.querySelectorAll(".picker-search[data-select-id]").forEach(wireTopbarSearchablePicker);
    if (topbarPickerGlobalsWired) return;
    topbarPickerGlobalsWired = true;
    document.addEventListener("mousedown", (event) => {
      if (!topbarPickerInput || (topbarPickerList && topbarPickerList.contains(event.target))) return;
      if (topbarPickerInput.contains(event.target)) return;
      closeTopbarPicker(true);
    });
    window.addEventListener("resize", () => closeTopbarPicker(true));
  }

  function resetSelect(sel) {
    const input = pickerInputForSelect(sel);
    if (input && topbarPickerInput === input) closeTopbarPicker(true);
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

  function populateRegionSelect(current) {
    const sel = $("#region-select");
    if (!sel) return;
    resetSelect(sel);
    allowedRegions.forEach(r => {
      const opt = addOption(sel, r, r);
      if (r === current) opt.selected = true;
    });
    if (current && !allowedRegions.includes(current)) addOption(sel, current, `${current} — unsupported`);
    if (!current) addOption(sel, "", "(choose a supported region)");
    sel.value = current || "";
    sel.disabled = !allowedRegions.length;
    syncTopbarPicker(sel);
  }

  function populateAccountSelect(profiles) {
    const sel = $("#account-select");
    if (!sel) return;
    resetSelect(sel);
    if (!profiles || profiles.length === 0) {
      addOption(sel, "", "(no configured profiles)");
      sel.disabled = true;
      syncTopbarPicker(sel);
      return;
    }
    sel.disabled = !profiles.some(profile => profile.eligibility === "supported_sso");
    profiles.forEach(p => {
      const acct = p.account_id ? ` · ${p.account_id}` : "";
      const opt = addOption(sel, p.name, `${p.name}${acct}${p.eligibility === "supported_sso" ? "" : " · unsupported"}`, {
        accountId: p.account_id || "",
        role: p.role_name || "",
        region: p.region || "",
        ssoSession: p.sso_session || "",
        eligibility: p.eligibility,
      });
      opt.disabled = p.eligibility !== "supported_sso";
    });
    syncTopbarPicker(sel);
  }

  // Monotonic counter — every applyTopbarSelection call gets a unique id and
  // checks it after the await; if a newer call started in the meantime, this
  // one bails out instead of overwriting state. Prevents two rapid picks from
  // trampling each other and clears the "stuck UI" race.
  let currentSelectionId = 0;
  // Captured aws_set_account result so the auth pill can render the
  // needs_sso_login signal accurately even before the next authStatus poll.
  let lastSetAccountResult = null;
  let discoveryReady = false;
  let freshProfiles = [];
  let freshProfilesConfiguration = -1;
  let connectionState = "discovering";
  const CONNECTION_STATES = {
    discovering: ["Discovering profiles", "Reading the configured AWS profile file. No account is verified yet."],
    missing_config: ["AWS config file not found", "Choose your existing AWS config file in Settings, then retry discovery."],
    unreadable_config: ["AWS config file cannot be read", "Check that the configured file is accessible, or choose another file in Settings, then retry."],
    malformed_config: ["AWS config needs correction", "Correct the existing AWS config file or choose another file in Settings, then retry."],
    no_profiles: ["No profiles found", "Use an existing SSO profile in the configured AWS file, then retry discovery."],
    unsupported: ["Profile cannot be used", "This beta supports explicit SSO profiles. Review the profile details below or choose a supported SSO profile."],
    unsupported_region: ["Region is not supported", "Choose a supported region in the topbar or correct the saved default in Settings."],
    verifying: ["Verifying AWS identity", "Checking the selected profile, account and region before loading resources."],
    verified: ["Connected to AWS", "The account and region below are verified for the selected profile."],
    expired: ["SSO login needs renewal", "Complete your existing SSO login outside this app (aws sso login for the selected profile), then retry connection."],
    mismatch: ["AWS identity did not match", "The returned account did not match the selection. Check the SSO profile/account configuration, then retry."],
    failed: ["Connection could not be verified", "Review the selected SSO profile and Identity details. Correct the dependency, then retry connection."],
    settings_failed: ["Settings could not be loaded", "Open Settings to retry loading or explicitly recover the saved settings before connecting."],
  };

  function setConnectionState(state, verified) {
    if (!isTauri) return;
    connectionState = CONNECTION_STATES[state] ? state : "failed";
    const [title, message] = CONNECTION_STATES[connectionState];
    const strip = $("#connection-status");
    strip.hidden = false;
    strip.dataset.state = connectionState;
    $("#connection-title").textContent = title;
    $("#connection-message").textContent = message;
    const profile = $("#account-select").value || topbarState.profile || "(none)";
    const region = $("#region-select").value || topbarState.region || "(none)";
    $("#connection-identity").textContent = verified
      ? `Profile: ${verified.profile} · Verified account: ${verified.account_id} · Region: ${verified.region}`
      : `Selected profile: ${profile} · Region: ${region} · Account is not verified`;
    $("#connection-retry").disabled = ["discovering", "verifying"].includes(connectionState);
    $("#connection-details").disabled = !lastAuthStatus && !lastSetAccountResult;
  }

  function connectionFailureState(info) {
    if (info?.needs_sso_login || ["CredentialsExpired", "SsoTokenExpired", "SsoLoginRequired"].includes(info?.error_type)) return "expired";
    if (["IdentityMismatch", "AccountMismatch"].includes(info?.error_type)) return "mismatch";
    if (["UnsupportedProfile", "SsoSessionConflict"].includes(info?.error_type)) return "unsupported";
    if (info?.error_type === "UnsupportedRegion") return "unsupported_region";
    return "failed";
  }

  function clearDiscoveredConnection(state) {
    discoveryReady = false;
    freshProfiles = [];
    freshProfilesConfiguration = -1;
    ++currentSelectionId;
    ++currentAuthStatusId;
    lastSetAccountResult = null;
    lastAuthStatus = { has_context: false, logged_in: false, connection_state: "unselected" };
    topbarState.accountId = null;
    clearInheritedResults();
    populateAccountSelect([]);
    updateAllWidgetContextChips();
    const pill = $("#auth-status");
    pill.hidden = false;
    pill.dataset.state = "offline";
    $(".core-label", pill).textContent = "auth: no verified account";
    setConnectionState(state);
    if ($("#identity-panel").classList.contains("open")) renderIdentityPanel(lastAuthStatus);
  }

  async function retryConnection() {
    if (!settingsStorageReady) {
      const loaded = await loadSettings();
      if (!loaded) return;
      if (connectionState === "discovering") return;
    }
    return refreshProfilesInBackground();
  }
  $("#connection-settings").addEventListener("click", openSettingsPanel);
  $("#connection-retry").addEventListener("click", retryConnection);
  $("#connection-details").addEventListener("click", openIdentityPanel);

  function setPillInFlight(label) {
    const pill = $("#auth-status");
    if (!pill) return;
    pill.hidden = false;
    pill.dataset.state = "checking";
    const lbl = pill.querySelector(".core-label");
    if (lbl) lbl.textContent = label;
  }

  async function applyTopbarSelection(opts) {
    if (!isTauri || !settingsStorageReady || !discoveryReady) return;
    const accSel = $("#account-select");
    const opt = accSel.options[accSel.selectedIndex];
    if (!opt || !opt.value || opt.disabled || opt.dataset.eligibility !== "supported_sso") return;
    const myId = ++currentSelectionId;
    lastSetAccountResult = null;
    clearInheritedResults();
    lastAuthStatus = { has_context: false, logged_in: false, connection_state: "verifying" };
    if ($("#identity-panel")?.classList.contains("open")) renderIdentityPanel(lastAuthStatus);

    const fromProfile = opts && opts.fromProfile;
    const profileRegion = opt.dataset.region || "";
    const regSel = $("#region-select");
    if (fromProfile && profileRegion) populateRegionSelect(profileRegion);
    const region = regSel.value;
    const profile = opt.value;
    const accountId = opt.dataset.accountId;
    const ssoSession = opt.dataset.ssoSession || "";
    if (!accountId) {
      setConnectionState("unsupported");
      return;
    }
    topbarState.profile = profile;
    topbarState.accountId = accountId;
    topbarState.region = region;
    if (!allowedRegions.includes(region)) {
      lastSetAccountResult = { ok: false, error: "Choose a supported region before verifying the account." };
      updateAllWidgetContextChips();
      const pill = $("#auth-status");
      pill.hidden = false;
      pill.dataset.state = "offline";
      $(".core-label", pill).textContent = "auth: unsupported region — choose a supported region";
      setConnectionState("unsupported_region");
      return;
    }

    updateAllWidgetContextChips();
    document.querySelectorAll('.widget[data-widget="pipeline-runs"]').forEach((widget) => {
      const tile = widget.closest(".grid-stack-item");
      if (contextForTile(tile).mode === "pinned") return;
      const runsRows = $(".pipeline-runs-rows", widget);
      if (runsRows) { clear(runsRows); runsRows.hidden = true; }
    });

    // Immediate visible feedback — the pill stops showing stale state the
    // moment the user picks. Without this the SSO call (1-3s) feels like a
    // dead click.
    setPillInFlight(`auth: verifying ${profile} · ${region} …`);
    setConnectionState("verifying");

    let res;
    try {
      res = await tauriInvoke("aws_set_account", {
        params: {
          profile,
          account_id: accountId,
          region,
          sso_session_name: ssoSession || null,
          request_id: `selection-${myId}`,
        },
      });
    } catch (e) {
      if (myId !== currentSelectionId) return; // newer selection won — discard
      console.warn("Account verification request failed.");
      lastSetAccountResult = { ok: false, error: "Account verification could not be completed. Retry connection." };
      setConnectionState("failed");
      writeLastSelection(profile, region);
      refreshAuthStatus();
      return;
    }
    if (myId !== currentSelectionId) return;  // newer selection won — discard
    if (!res?._request || res._request.id !== `selection-${myId}`
        || (res.ok && ["context_id", "provider_revision", "settings_revision"].some(key => typeof res._request[key] !== "string" || !res._request[key]))
        || (res._request.context_id !== null && (res._request.profile !== profile
          || res._request.account_id !== accountId || res._request.region !== region))) {
      res = { ok: false, error_type: "IdentityMismatch", error: "Response AWS context did not match." };
    }

    lastSetAccountResult = res;
    if (res.ok) setConnectionState("verified", res._request);
    else setConnectionState(connectionFailureState(res));
    if (!res.ok) {
      console.warn("Account verification failed.");
    }
    // A deliberate default account/region switch reloads every inherited widget.
    // Pinned tiles are skipped because they intentionally ignore the topbar.
    writeLastSelection(profile, region);
    refreshAuthStatus();
    updateAllWidgetContextChips();
    // Refresh the pipeline dropdown FIRST (a single ListPipelines call, so the
    // combobox reflects the account just picked), THEN reload all unpinned
    // widgets (so pipeline-runs picks up the new selection).
    if (res && res.ok) {
      loadPipelineListsForInheritedWidgets().then(() => {
        if (myId === currentSelectionId && lastSetAccountResult?.ok) refreshAllUnpinnedWidgets();
      });
    }
  }

  function describeProfilesEmpty(info) {
    // Build a short, useful explanation for the topbar dropdown when no
    // profiles came back from aws.listProfiles.
    if (!info) return "(profile lookup failed)";
    const path = info.resolved_path || info.config_path || "the configured AWS file";
    if (info.file_exists === false) return `(no file at ${path})`;
    if (info.error) return `(parse error: ${info.error})`;
    return `(0 profiles in ${path})`;
  }

  // Profile choices belong to the current successful discovery. Persisted
  // old listings never authorize a new selection or populate pinned choices.
  const LAST_SELECTION_KEY = "acc.last.v1";

  function readProfilesCache() {
    return discoveryReady && freshProfilesConfiguration === configurationGeneration
      ? { profiles: freshProfiles } : null;
  }
  function writeProfilesCache(info) {
    freshProfiles = info.profiles;
    freshProfilesConfiguration = configurationGeneration;
  }
  function readLastSelection() {
    try {
      const raw = localStorage.getItem(LAST_SELECTION_KEY);
      return raw ? JSON.parse(raw) : null;
    } catch (_) { return null; }
  }
  function writeLastSelection(profile, region) {
    try {
      localStorage.setItem(LAST_SELECTION_KEY, JSON.stringify({ profile, region }));
    } catch (_) {}
  }
  function profilesEqual(a, b) {
    if (a.length !== b.length) return false;
    const key = (p) => `${p.name}|${p.account_id}|${p.role_name}|${p.region}|${p.sso_session}`;
    return a.every((p, i) => key(p) === key(b[i]));
  }

  function selectAccountByName(name, opts) {
    const accSel = $("#account-select");
    if (!accSel) return false;
    const match = Array.from(accSel.options).find(o => o.value === name && !o.disabled);
    if (!match) return false;
    accSel.selectedIndex = match.index;
    syncTopbarPicker(accSel);
    applyTopbarSelection(opts || { fromProfile: true });
    return true;
  }

  let currentProfilesRequestId = 0;
  async function refreshProfilesInBackground() {
    if (!isTauri || !settingsStorageReady) return;
    const accSel = $("#account-select");
    const previousValue = accSel.value || topbarState.profile;
    const requestId = ++currentProfilesRequestId;
    const configuration = configurationGeneration;
    const selection = currentSelectionId;
    discoveryReady = false;
    accSel.disabled = true;
    syncTopbarPicker(accSel);
    setPillInFlight("auth: discovering profiles …");
    setConnectionState("discovering");
    $("#connection-profiles").hidden = true;
    $("#connection-coverage").hidden = true;
    let info;
    try {
      info = await tauriInvoke("aws_list_profiles");
      if (requestId !== currentProfilesRequestId || configuration !== configurationGeneration
          || selection !== currentSelectionId) return;
      if (!["ready", "missing_config", "unreadable_config", "malformed_config", "no_profiles"].includes(info?.discovery_state)
          || !Array.isArray(info.profiles) || info.profiles.length > 500) throw new Error("Discovery response unavailable");
      if (info.discovery_state !== "ready") {
        clearDiscoveredConnection(info.discovery_state);
        return;
      }
      if (!info.profiles.length || info.profiles.some(profile => typeof profile.name !== "string" || !profile.name
          || !["supported_sso", "unsupported_credentials", "invalid_sso"].includes(profile.eligibility))) {
        throw new Error("Discovery response unavailable");
      }
    } catch (e) {
      if (requestId !== currentProfilesRequestId || configuration !== configurationGeneration || selection !== currentSelectionId) return;
      console.warn("Profile list could not be refreshed.");
      clearDiscoveredConnection("failed");
      return;
    }
    const profiles = info.profiles;
    discoveryReady = true;
    writeProfilesCache(info);
    const unsupported = profiles.filter(profile => profile.eligibility !== "supported_sso");
    const list = $("#connection-profiles");
    clear(list);
    unsupported.slice(0, 8).forEach(profile => list.appendChild(el("li", {}, `${profile.name}: ${profile.eligibility === "unsupported_credentials"
      ? "Unsupported credential provider. Use an existing SSO profile."
      : "SSO profile configuration is incomplete or conflicts with Settings."}`)));
    if (unsupported.length > 8) list.appendChild(el("li", {}, `${unsupported.length - 8} more unsupported profiles.`));
    list.hidden = unsupported.length === 0;
    if (info.partial === true) {
      $("#connection-coverage").textContent = `Profile discovery is limited: ${profiles.length} profiles shown. Other profiles were omitted.`;
      $("#connection-coverage").hidden = false;
    }
    populateAccountSelect(profiles);
    const last = readLastSelection();
    const want = previousValue || last?.profile || cachedSettings.default_profile || "";
    const wanted = profiles.find(profile => profile.name === want);
    if (wanted?.eligibility !== "supported_sso" && (wanted || !profiles.some(profile => profile.eligibility === "supported_sso"))) {
      ++currentSelectionId;
      lastSetAccountResult = { ok: false, error_type: "UnsupportedProfile", error: "Choose a supported SSO profile." };
      lastAuthStatus = { has_context: false, logged_in: false, connection_state: "unselected" };
      clearInheritedResults();
      topbarState.accountId = null;
      accSel.value = wanted?.name || profiles[0].name;
      topbarState.profile = accSel.value;
      topbarState.region = $("#region-select").value;
      updateAllWidgetContextChips();
      syncTopbarPicker(accSel);
      $("#auth-status").dataset.state = "offline";
      $(".core-label", $("#auth-status")).textContent = "auth: unsupported profile";
      setConnectionState("unsupported");
      return;
    }
    selectAccountByName(wanted?.name || profiles.find(profile => profile.eligibility === "supported_sso").name, { fromProfile: false });
  }

  function applyBrowserRegionSelection() {
    const regSel = $("#region-select");
    if (!regSel) return;
    const region = allowedRegions.includes(regSel.value)
      ? regSel.value
      : allowedRegions[0];
    topbarState.region = region;
    writeLastSelection("", region);
    updateAllWidgetContextChips();
  }

  async function initTopbarPickers() {
    wireTopbarSearchablePickers();
    if (isTauri && !settingsStorageReady) return;
    if (!isTauri) {
      const sel = $("#account-select");
      resetSelect(sel);
      addOption(sel, "", "(browser mode — no profiles)");
      sel.disabled = true;
      syncTopbarPicker(sel);
      const regSel = $("#region-select");
      const last = readLastSelection();
      const initialRegion = last && allowedRegions.includes(last.region)
        ? last.region
        : allowedRegions[0];
      populateRegionSelect(initialRegion);
      if (regSel.dataset.contextWired !== "1") {
        regSel.dataset.contextWired = "1";
        regSel.addEventListener("change", applyBrowserRegionSelection);
      }
      applyBrowserRegionSelection();
      return;
    }
    const accSel = $("#account-select");
    const regSel = $("#region-select");
    if (accSel.dataset.contextWired !== "1") {
      accSel.dataset.contextWired = "1";
      accSel.addEventListener("change", () => applyTopbarSelection({ fromProfile: true }));
    }
    if (regSel.dataset.contextWired !== "1") {
      regSel.dataset.contextWired = "1";
      regSel.addEventListener("change", () => {
        writeLastSelection(accSel.value, regSel.value);
        applyTopbarSelection({ fromProfile: false });
      });
    }
    // Only fresh discovery may populate profile choices or verify a context.
    const last = readLastSelection();
    const initialRegion = allowedRegions.includes(cachedSettings.default_region)
      ? (last && last.region) || cachedSettings.default_region : cachedSettings.default_region;
    populateRegionSelect(initialRegion);
    return refreshProfilesInBackground();
  }

  // ===== Auth status pill (Tauri only) =====
  let authStatusTimer = null;

  function formatRemaining(expiresAt) {
    if (!expiresAt) return null;
    const t = Date.parse(expiresAt);
    if (Number.isNaN(t)) return null;
    const diff = t - Date.now();
    if (diff <= 0) return "expired";
    const mins = Math.round(diff / 60000);
    if (mins < 60) return `${mins}m left`;
    const hours = Math.floor(mins / 60);
    const remMins = mins - hours * 60;
    return remMins === 0 ? `${hours}h left` : `${hours}h ${remMins}m left`;
  }

  // Cached aws_auth_status result so the Identity panel can render without
  // re-issuing an RPC if it was just refreshed.
  let lastAuthStatus = null;
  let currentAuthStatusId = 0;

  async function refreshAuthStatus() {
    if (!isTauri || !settingsStorageReady || !discoveryReady) return;
    const pill = $("#auth-status");
    if (!pill) return;
    pill.hidden = false;
    const label = pill.querySelector(".core-label");
    const selectionId = currentSelectionId;
    const requestId = ++currentAuthStatusId;
    const discovery = currentProfilesRequestId;
    let info;
    try {
      info = await tauriInvoke("aws_auth_status");
    } catch (e) {
      if (selectionId !== currentSelectionId || requestId !== currentAuthStatusId || discovery !== currentProfilesRequestId) return;
      pill.dataset.state = "offline";
      label.textContent = "auth: rpc error";
      pill.title = "Identity status could not be refreshed. Retry connection.";
      setConnectionState("failed");
      return;
    }
    if (selectionId !== currentSelectionId || requestId !== currentAuthStatusId || discovery !== currentProfilesRequestId || !discoveryReady
        || info.error_type === "Superseded") return;
    const meta = info._request;
    if (info.has_context && (!meta || ["context_id", "provider_revision", "settings_revision"].some(key => typeof meta[key] !== "string" || !meta[key]))) return;
    if (meta?.context_id && (meta.profile !== topbarState.profile
        || meta.account_id !== topbarState.accountId || meta.region !== topbarState.region)) return;
    if (!info.has_context && lastSetAccountResult?.ok) {
      lastSetAccountResult = { ok: false, needs_sso_login: !!info.needs_sso_login,
        error_type: info.error_type, error: info.error || "AWS context is no longer verified." };
      clearInheritedResults();
    }
    // If aws_set_account just failed, prefer that immediate result while there
    // is no active context. It carries the concrete policy/credential reason.
    if (lastSetAccountResult && !lastSetAccountResult.ok && !info.has_context) {
      info = {
        ...info,
        needs_sso_login: !!lastSetAccountResult.needs_sso_login,
        error_type: lastSetAccountResult.error_type || info.error_type,
        error: lastSetAccountResult.error || info.error,
      };
    }
    lastAuthStatus = info;
    if (info.logged_in && info.has_context) setConnectionState("verified", meta);
    else if (info.connection_state === "verifying") setConnectionState("verifying");
    else if (lastSetAccountResult || info.needs_sso_login) setConnectionState(connectionFailureState(info));
    const remaining = formatRemaining(info.expires_at);
    if (info.connection_state === "verifying") {
      pill.dataset.state = "checking";
      label.textContent = "auth: verifying account …";
    } else if (info.logged_in || info.has_context) {
      pill.dataset.state = info.logged_in ? "online" : "checking";
      const parts = [info.logged_in ? "auth: ok" : "auth: active"];
      if (info.account_id) parts.push(info.account_id);
      if (remaining) parts.push(remaining);
      label.textContent = parts.join(" · ");
    } else if (info.needs_sso_login) {
      pill.dataset.state = "offline";
      label.textContent = "auth: expired — run aws sso login";
    } else if (!info.has_context && !lastSetAccountResult) {
      pill.dataset.state = "unknown";
      label.textContent = "auth: pick account";
    } else {
      pill.dataset.state = "offline";
      // Last set-account had an error other than expired SSO.
      label.textContent = "auth: error — click for details";
    }
    pill.title = "Click for full identity details";
    // Repaint the Identity panel if it happens to be open.
    if ($("#identity-panel")?.classList.contains("open")) renderIdentityPanel(info);
  }

  function startAuthStatusPolling() {
    if (!isTauri) return;
    refreshAuthStatus();
    if (authStatusTimer) clearInterval(authStatusTimer);
    authStatusTimer = setInterval(refreshAuthStatus, 60_000);
    const pill = $("#auth-status");
    if (pill) pill.addEventListener("click", openIdentityPanel);
  }

  // ===== Identity panel =====
  function openIdentityPanel() {
    const panel = $("#identity-panel");
    if (!panel) return;
    panel.classList.add("open");
    panel.setAttribute("aria-hidden", "false");
    $("#scrim").classList.add("open");
    $("#scrim").hidden = false;
    // Refresh from the backend so the panel always reflects current state.
    // Only an accepted, current poll may render identity data. A discarded
    // response must not revive a previously cached account through a callback.
    return refreshAuthStatus();
  }
  function closeIdentityPanel() {
    const panel = $("#identity-panel");
    if (!panel) return;
    panel.classList.remove("open");
    panel.setAttribute("aria-hidden", "true");
    $("#scrim").classList.remove("open");
    setTimeout(() => { $("#scrim").hidden = true; }, 220);
  }

  function renderIdentityPanel(info) {
    const wrap = $("#identity-body");
    if (!wrap) return;
    clear(wrap);
    const stateBadge = info.logged_in
      ? el("span", { class: "pill pill-green" }, "active")
      : info.needs_sso_login
        ? el("span", { class: "pill pill-amber" }, "sso expired")
        : info.has_context
          ? el("span", { class: "pill pill-amber" }, "error")
          : el("span", { class: "pill pill-grey" }, "no context");
    wrap.appendChild(el("div", { class: "identity-state" }, stateBadge));

    if (info.error) {
      wrap.appendChild(el("div", { class: "identity-error" }, info.error));
    }

    const rows = [
      ["Profile", info.profile],
      ["Account", info.account_id],
      ["Region", info.region],
      ["SSO session", info.sso_session],
      ["Caller ARN", info.caller_arn || "(resolved on next refresh)"],
      ["Verified credentials expire", info.expires_at || "(unknown)"],
      ["Last set-account succeeded at",
        info.set_account_at ? new Date(info.set_account_at * 1000).toLocaleString() : "(never)"],
      ["Application operation checks",
        info.read_only_guard_active === false
          ? "DISABLED (this should never happen)"
          : "ACTIVE — requested operations must be supported and allowed by policy"],
    ];
    const dl = el("dl", { class: "identity-kv" });
    rows.forEach(([k, v]) => {
      dl.appendChild(el("dt", {}, k));
      dl.appendChild(el("dd", { class: "mono" }, v || "(empty)"));
    });
    wrap.appendChild(dl);

    const actions = el("div", { class: "identity-actions" });
    actions.appendChild(el("button", {
      class: "btn btn-ghost",
      onclick: () => refreshAuthStatus(),
    }, "Refresh"));
    if (!info.logged_in && info.profile) {
      actions.appendChild(el("button", {
        class: "btn btn-primary",
        onclick: () => applyTopbarSelection({ fromProfile: false }),
      }, "Retry set-account"));
    }
    wrap.appendChild(actions);
  }

  // Manual refresh: clicking the Account label re-runs the profile lookup.
  document.addEventListener("DOMContentLoaded", () => {
    const label = document.querySelector('label[for="account-picker-search"]');
    if (label) {
      label.style.cursor = "pointer";
      label.title = "Click to reload profiles from ~/.aws/config";
      label.addEventListener("click", () => initTopbarPickers());
    }
  });

  // ===== Theme toggle =====
  $("#theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    if (!isTauri) { document.documentElement.dataset.theme = next; return; }
    if (!settingsStorageReady) return;
    $("#settings-theme").value = next;
    markSettingsDirty();
  });
  $("#appearance-settings").addEventListener("click", openSettingsPanel);

  // Cmd-K focuses the global search; Esc closes the side panel.
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      $("#global-search").focus();
    }
    if (e.key === "Escape") {
      closeSidePanel();
      closeSettingsPanel();
      closeAuditPanel();
      closeIdentityPanel();
      closeWidgetConfigPanel();
    }
  });

  function widgetsOfType(type) {
    return Array.from(document.querySelectorAll(`.widget[data-widget="${type}"]`));
  }

  function withWidgets(type, target, fn) {
    let widget = null;
    if (target && target.matches && target.matches(".widget")) {
      widget = target;
    }
    if (!widget && target && target.closest) {
      widget = target.closest(".widget");
    }
    if (!widget && target && target.querySelector) {
      widget = target.querySelector(".widget");
    }
    if (widget && widget.dataset && widget.dataset.widget === type) {
      return fn(widget);
    }
    widgetsOfType(type).forEach(fn);
    return undefined;
  }

  // ===== Pipeline runs widget =====
  function renderPipelineRuns(target) {
    if (!isTauri) {
      return renderPipelineRunsMockOnly(target);
    }
    return withWidgets("pipeline-runs", target, renderPipelineRunsOne);
  }

  function pipelinePinsForWidget(widget) {
    const cfg = tileConfig(widget);
    return normalizePipelinePins(cfg.inputs && cfg.inputs[PIPELINE_PINS_KEY]);
  }

  function writePipelinePins(widget, pins) {
    const item = tileItemFor(widget);
    if (!item) return [];
    const cfg = tileConfig(item);
    const clean = normalizePipelinePins(pins);
    const inputs = { ...(cfg.inputs || {}) };
    if (clean.length) inputs[PIPELINE_PINS_KEY] = clean;
    else delete inputs[PIPELINE_PINS_KEY];
    writeTileConfig(item, { ...cfg, inputs });
    scheduleSaveLayout();
    return clean;
  }

  function pipelinePinContext(pin) {
    return {
      mode: "pinned",
      profile: pin.profile,
      account_id: pin.account_id,
      region: pin.region,
    };
  }

  function selectedPipelineName(form) {
    const sel = $(".pipeline-name-select", form);
    const input = $(".pipeline-name-search", form);
    return ((sel && sel.value) || (input && input.value) || "").trim();
  }

  function addSelectedPipelineOption(form, name) {
    const sel = $(".pipeline-name-select", form);
    if (!sel || !name) return;
    const has = Array.from(sel.options).some(o => o.value === name);
    if (!has) {
      const opt = document.createElement("option");
      opt.value = name;
      opt.textContent = name;
      sel.appendChild(opt);
    }
    sel.value = name;
  }

  function pinMetaLabel(pin) {
    return `${pin.profile} · ${pin.account_id} · ${pin.region}`;
  }

  // ----- Environment "lights" -----
  // Pipeline and profile names usually embed their stage (my-svc-dev,
  // team-api-qa). Default mode lights only the stage token, so long pipeline
  // names don't become one giant chip. wholeWord mode lights the entire
  // whitespace-delimited word (team-api-dev) — right for short profile names in
  // the meta line. Tokens only match between -_. separators, so "devops"
  // stays plain.
  const ENV_TOKEN_RE = /(^|[-_.])(prod|prd|production|qa|uat|stg|stage|staging|test|tst|dev|development|sandbox|sbx)(?=$|[-_.])/i;
  const ENV_TOKEN_RE_G = new RegExp(ENV_TOKEN_RE.source, "gi");

  function envLightClass(token) {
    const t = token.toLowerCase();
    if (t === "prod" || t === "prd" || t === "production") return "env-prod";
    if (t === "dev" || t === "development" || t === "sandbox" || t === "sbx") return "env-dev";
    return "env-qa";
  }

  function envLightNodes(name, opts) {
    const nodes = [];
    if (opts && opts.wholeWord) {
      for (const part of String(name).split(/(\s+)/)) {
        if (!part) continue;
        const m = ENV_TOKEN_RE.exec(part);
        if (m) nodes.push(el("span", { class: "env-light " + envLightClass(m[2]) }, part));
        else nodes.push(document.createTextNode(part));
      }
    } else {
      const s = String(name);
      let last = 0;
      ENV_TOKEN_RE_G.lastIndex = 0;
      let m;
      while ((m = ENV_TOKEN_RE_G.exec(s)) !== null) {
        const start = m.index + m[1].length;
        if (start > last) nodes.push(document.createTextNode(s.slice(last, start)));
        nodes.push(el("span", { class: "env-light " + envLightClass(m[2]) }, m[2]));
        last = start + m[2].length;
      }
      if (last < s.length) nodes.push(document.createTextNode(s.slice(last)));
    }
    if (nodes.length === 0) nodes.push(document.createTextNode(String(name)));
    return nodes;
  }

  // ----- Live / Pinned tabs -----
  function activePipelineTab(widget) {
    const body = $(".pipeline-runs-body", widget);
    return (body && body.dataset.tab) || "live";
  }

  function setPipelineTab(widget, tab) {
    const body = $(".pipeline-runs-body", widget);
    if (!body) return;
    body.dataset.tab = tab;
    body.querySelectorAll(".pipeline-tab").forEach((btn) => {
      const active = btn.dataset.tab === tab;
      btn.classList.toggle("active", active);
      btn.setAttribute("aria-selected", String(active));
    });
    const live = $(".pipeline-pane-live", body);
    const pinned = $(".pipeline-pane-pinned", body);
    if (live) live.hidden = tab !== "live";
    if (pinned) pinned.hidden = tab !== "pinned";
  }

  async function loadSelectedPipelineRuns(widget, form) {
    const errorEl = $(".pipeline-error", widget);
    const rowsHost = $(".pipeline-runs-rows", widget);
    const pipelineName = selectedPipelineName(form);
    if (!pipelineName) {
      if (errorEl) errorEl.textContent = "Pick a pipeline first.";
      return;
    }
    addSelectedPipelineOption(form, pipelineName);
    const fetchContext = contextPayloadForTile(form);
    clear(rowsHost);
    const request = beginOwnedRequest(rowsHost, fetchContext);
    if (errorEl) errorEl.textContent = "Loading...";
    try {
      const result = await fetchWidgetData(
        "pipeline-runs",
        { pipeline_name: pipelineName },
        form,
        fetchContext, request,
      );
      if (!request.accept(result)) return;
      if (rowsHost) {
        rowsHost._widgetContextOverride = fetchContext;
        renderTable(rowsHost, result, {
          expand: (row, cell) => {
            cell._widgetContextOverride = fetchContext;
            return fetchWidgetInto(cell, "pipeline-execution-detail",
              { pipeline_name: pipelineName, execution_id: row.execution_id }, fetchContext);
          },
        });
        rowsHost.hidden = false;
      }
      if (errorEl) errorEl.textContent = "";
    } catch (err) {
      if (!request.current()) return;
      if (errorEl) errorEl.textContent = `Error: ${err}`;
      failResult(rowsHost, request, String(err));
    }
  }

  function pinSelectedPipeline(widget, form) {
    const errorEl = $(".pipeline-error", widget);
    const pipelineName = selectedPipelineName(form);
    if (!pipelineName) {
      if (errorEl) errorEl.textContent = "Pick a pipeline first.";
      return;
    }
    const ctx = effectivePinnedContextForTile(form);
    if (!ctx) {
      if (errorEl) errorEl.textContent = "Pick an account and region first.";
      return;
    }
    const pin = {
      pipeline_name: pipelineName,
      profile: ctx.profile,
      account_id: ctx.account_id,
      region: ctx.region,
    };
    pin.id = pipelinePinKey(pin);
    const pins = pipelinePinsForWidget(widget);
    if (!pins.some(p => pipelinePinKey(p) === pin.id)) {
      writePipelinePins(widget, [...pins, pin]);
    }
    addSelectedPipelineOption(form, pipelineName);
    renderPipelinePinList(widget);
    refreshPipelinePinByKey(widget, pin.id);
    if (errorEl) errorEl.textContent = "Pinned — see the Pinned tab.";
  }

  // Pin cards render collapsed so a reload shows a compact, scannable list.
  // Expansion state is session-only, keyed by pin id, so toggles survive the
  // list reconciliation that pinning/refreshing triggers.
  const pipelinePinExpandState = new Map();

  function renderPipelinePinList(widget) {
    const list = $(".pipeline-pin-list", widget);
    if (!list) return;
    const pins = pipelinePinsForWidget(widget);
    const existing = new Map(Array.from(list.children).map(card => [card.dataset.pinId, card]));
    existing.forEach((card, key) => {
      if (!pins.some(pin => pipelinePinKey(pin) === key)) card.remove();
    });
    const emptyEl = $(".pipeline-pin-empty", widget);
    if (emptyEl) emptyEl.hidden = pins.length > 0;
    const countEl = $(".pipeline-pin-count", widget);
    if (countEl) {
      countEl.textContent = String(pins.length);
      countEl.hidden = pins.length === 0;
    }
    // Reorders persist by reading the card order back out of the DOM, so a
    // drop only moves the node — fetched results and expansion state survive.
    const commitPinOrder = () => {
      const byKey = new Map(pipelinePinsForWidget(widget).map(p => [pipelinePinKey(p), p]));
      const order = Array.from(list.children).map(c => c.dataset.pinId);
      writePipelinePins(widget, order.map(k => byKey.get(k)).filter(Boolean));
    };
    pins.forEach((pin) => {
      const key = pipelinePinKey(pin);
      pin.id = key;
      const reused = existing.get(key);
      if (reused) { list.appendChild(reused); return; }
      const handle = el("button", { class: "icon-btn pipeline-pin-handle", type: "button", title: "Drag to reorder" }, "≡");
      const toggle = el("button", { class: "icon-btn pipeline-pin-toggle", type: "button" }, "▸");
      const status = el("span", { class: "badge badge-neutral pipeline-pin-status" }, "Pinned");
      const updated = el("span", { class: "muted small pipeline-pin-updated" });
      const resultHost = el("div", { class: "pipeline-pin-result muted small" }, "Not refreshed");
      const head = el("div", { class: "pipeline-pin-head" },
        handle,
        toggle,
        el("div", { class: "pipeline-pin-main" },
          el("div", { class: "pipeline-pin-name", title: pin.pipeline_name }, envLightNodes(pin.pipeline_name)),
          el("div", { class: "pipeline-pin-meta", title: pinMetaLabel(pin) }, envLightNodes(pinMetaLabel(pin), { wholeWord: true })),
        ),
        status,
        updated,
        el("button", { class: "icon-btn pipeline-pin-refresh", type: "button", title: "Refresh pinned pipeline" }, "↻"),
        el("button", { class: "icon-btn pipeline-pin-remove", type: "button", title: "Remove pinned pipeline" }, "✕"),
      );
      const card = el("section", { class: "pipeline-pin-card", "data-pin-id": key }, head, resultHost);
      const applyExpanded = (want) => {
        card.classList.toggle("expanded", want);
        resultHost.hidden = !want;
        toggle.textContent = want ? "▾" : "▸";
        toggle.title = want ? "Collapse pinned pipeline" : "Expand pinned pipeline";
        toggle.setAttribute("aria-expanded", String(want));
      };
      const setExpanded = (want) => {
        pipelinePinExpandState.set(key, want);
        applyExpanded(want);
        // First expand of a card that has never fetched loads it on demand.
        if (want && card.dataset.loaded !== "1") refreshPipelinePinCard(widget, pin, card);
      };
      applyExpanded(pipelinePinExpandState.get(key) === true);
      toggle.addEventListener("click", () => setExpanded(resultHost.hidden));
      head.addEventListener("click", (e) => {
        if (e.target.closest("button")) return;
        setExpanded(resultHost.hidden);
      });
      $(".pipeline-pin-refresh", card).addEventListener("click", () => refreshPipelinePinCard(widget, pin, card));
      $(".pipeline-pin-remove", card).addEventListener("click", () => {
        const kept = pipelinePinsForWidget(widget).filter(p => pipelinePinKey(p) !== key);
        writePipelinePins(widget, kept);
        pipelinePinExpandState.delete(key);
        renderPipelinePinList(widget);
      });
      // Mouse-based reorder wired at the document level. HTML5 drag events
      // never fire inside the Tauri webview (native drag is reserved for
      // file drop), and WKWebView's pointer capture doesn't reliably route
      // mouse moves either — plain document mousemove/mouseup always works.
      handle.addEventListener("mousedown", (e) => {
        if (e.button !== 0) return;
        e.preventDefault(); // stops text selection from spilling across widgets
        card.classList.add("dragging");
        let moved = false;
        const onMove = (ev) => {
          ev.preventDefault();
          const over = document.elementFromPoint(ev.clientX, ev.clientY);
          const target = over && over.closest ? over.closest(".pipeline-pin-card") : null;
          if (!target || target === card || target.parentElement !== list) return;
          const r = target.getBoundingClientRect();
          list.insertBefore(card, ev.clientY < r.top + r.height / 2 ? target : target.nextSibling);
          moved = true;
        };
        const onUp = () => {
          document.removeEventListener("mousemove", onMove);
          card.classList.remove("dragging");
          if (moved) commitPinOrder();
        };
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp, { once: true });
      });
      list.appendChild(card);
    });
  }

  function refreshPipelinePinByKey(widget, key) {
    const pins = pipelinePinsForWidget(widget);
    const pin = pins.find(p => pipelinePinKey(p) === key);
    const card = pin && Array.from(widget.querySelectorAll(".pipeline-pin-card"))
      .find(item => item.dataset.pinId === key);
    if (pin && card) return refreshPipelinePinCard(widget, pin, card);
    return Promise.resolve();
  }

  async function refreshPipelinePinCard(widget, pin, card) {
    card.dataset.loaded = "1"; // marks an in-flight/completed fetch so expand doesn't refetch
    const statusEl = $(".pipeline-pin-status", card);
    const updatedEl = $(".pipeline-pin-updated", card);
    const resultHost = $(".pipeline-pin-result", card);
    const ctx = pipelinePinContext(pin);
    if (statusEl) {
      statusEl.className = "badge badge-progress pipeline-pin-status";
      statusEl.textContent = "Loading";
    }
    if (updatedEl) updatedEl.textContent = "";
    if (resultHost) {
      resultHost.className = "pipeline-pin-result muted small";
      resultHost.textContent = "Loading...";
      resultHost._widgetContextOverride = ctx;
    }
    const request = beginOwnedRequest(card, ctx);
    try {
      const result = await fetchWidgetData(
        "pipeline-runs",
        { pipeline_name: pin.pipeline_name },
        widget,
        ctx, request,
      );
      if (!request.accept(result)) return;
      if (result && result.render === "table" && (!resultFailure(result) || resultHasEvidence(result))) {
        const rows = Array.isArray(result.rows) ? result.rows : [];
        const latest = rows[0] || null;
        if (statusEl) {
          const rawStatus = requestFailureLabel(result) || (latest && latest.status ? latest.status : "No runs");
          statusEl.className = "badge " + (latest ? statusToBadge(rawStatus) : "badge-neutral") + " pipeline-pin-status";
          statusEl.textContent = requestFailureLabel(result) || (latest ? formatStatusLabel(rawStatus) : "No runs");
        }
        if (updatedEl && latest && latest.last_updated) {
          updatedEl.textContent = formatDisplayValue("last_updated", latest.last_updated).text;
          updatedEl.title = latest.last_updated;
        }
        if (resultHost) {
          resultHost.className = "pipeline-pin-result";
          resultHost._widgetContextOverride = ctx;
          renderTable(resultHost, result, {
            expand: (row, cell) => {
              cell._widgetContextOverride = ctx;
              return fetchWidgetInto(cell, "pipeline-execution-detail",
                { pipeline_name: pin.pipeline_name, execution_id: row.execution_id }, ctx);
            },
          });
        }
        return;
      }
      if (statusEl) {
        statusEl.className = "badge badge-error pipeline-pin-status";
        statusEl.textContent = requestFailureLabel(result) || "Error";
      }
      if (resultHost) {
        resultHost.className = "pipeline-pin-result";
        resultHost._widgetContextOverride = ctx;
        dispatchRender(resultHost, result);
      }
    } catch (err) {
      if (!request.current()) return;
      if (statusEl) {
        statusEl.className = "badge badge-error pipeline-pin-status";
        statusEl.textContent = "Error";
      }
      if (resultHost) {
        resultHost.className = "pipeline-pin-result muted small";
        failResult(resultHost, request, String(err));
      }
    }
  }

  function refreshPipelinePins(widget) {
    renderPipelinePinList(widget);
    const pins = pipelinePinsForWidget(widget);
    return Promise.all(pins.map(pin => refreshPipelinePinByKey(widget, pipelinePinKey(pin))));
  }

  function renderPipelineRunsOne(widget) {
    const form = $(".pipeline-config", widget);
    if (!form) return;
    wireRequestInputs(form, $(".pipeline-runs-rows", widget), $(".pipeline-error", widget));
    wirePipelineCombo(form);
    updateWidgetContextChip(widget.closest(".grid-stack-item"));
    renderPipelinePinList(widget);

    if (form.dataset.wired === "1") {
      setPipelineTab(widget, activePipelineTab(widget));
      return;
    }
    form.dataset.wired = "1";

    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      loadSelectedPipelineRuns(widget, form);
    });
    widget.querySelectorAll(".pipeline-tab").forEach((btn) => {
      btn.addEventListener("click", () => setPipelineTab(widget, btn.dataset.tab));
    });
    $(".pipeline-load-btn", widget)?.addEventListener("click", () => {
      setPipelineTab(widget, "live");
      loadSelectedPipelineRuns(widget, form);
    });
    $(".pipeline-pin-btn", widget)?.addEventListener("click", () => pinSelectedPipeline(widget, form));
    // A tile that already has pins opens on the Pinned tab; otherwise Live.
    setPipelineTab(widget, pipelinePinsForWidget(widget).length > 0 ? "pinned" : "live");
  }

  // ----- Searchable, regex-aware pipeline picker (combobox) -----
  // The dropdown list is appended to document.body and positioned fixed because
  // .widget is overflow:hidden and would clip an in-tile dropdown. The active
  // input owns its own pipeline-name array, so duplicate Pipeline Runs tiles can
  // point at different accounts/regions.
  let comboListEl = null;
  let comboOwnerInput = null;
  let comboActiveIndex = -1;
  const COMBO_MAX = 200;

  function pipelineComboMatches(input, query) {
    const names = input._pipelineNames || [];
    const q = query.trim();
    if (!q) return { items: names.slice(0, COMBO_MAX), regexOk: true };
    let items;
    let regexOk;
    try {
      const re = new RegExp(q, "i");
      items = names.filter(n => re.test(n));
      regexOk = true;
    } catch (_e) {
      const lq = q.toLowerCase();
      items = names.filter(n => n.toLowerCase().includes(lq));
      regexOk = false;
    }
    return { items: items.slice(0, COMBO_MAX), regexOk };
  }

  function positionComboList(input) {
    if (!comboListEl) return;
    const r = input.getBoundingClientRect();
    comboListEl.style.left = r.left + "px";
    comboListEl.style.width = r.width + "px";
    const below = window.innerHeight - r.bottom;
    const above = r.top;
    if (below < 180 && above > below) {
      comboListEl.style.bottom = (window.innerHeight - r.top + 2) + "px";
      comboListEl.style.top = "auto";
      comboListEl.style.maxHeight = Math.min(260, Math.max(0, above - 8)) + "px";
    } else {
      comboListEl.style.top = (r.bottom + 2) + "px";
      comboListEl.style.bottom = "auto";
      comboListEl.style.maxHeight = Math.min(260, Math.max(0, below - 8)) + "px";
    }
  }

  function ensureComboListEl() {
    if (comboListEl) return comboListEl;
    comboListEl = document.createElement("ul");
    comboListEl.className = "combo-list";
    comboListEl.setAttribute("role", "listbox");
    comboListEl.hidden = true;
    document.body.appendChild(comboListEl);
    return comboListEl;
  }

  function renderPipelineComboList(input) {
    if (!input || input.disabled) return;
    ensureComboListEl();
    comboOwnerInput = input;
    const q = input.value.trim();
    const { items, regexOk } = pipelineComboMatches(input, input.value);
    while (comboListEl.firstChild) comboListEl.removeChild(comboListEl.firstChild);
    if (items.length === 0) {
      const li = document.createElement("li");
      li.className = "combo-empty";
      li.textContent = "no matches";
      comboListEl.appendChild(li);
    } else {
      for (const name of items) {
        const li = document.createElement("li");
        li.className = "combo-item";
        li.setAttribute("role", "option");
        li.dataset.name = name;
        envLightNodes(name).forEach((n) => li.appendChild(n));
        li.addEventListener("mousedown", (e) => {
          e.preventDefault();
          if (!li.isConnected || !input.isConnected || comboOwnerInput !== input || input.disabled) return;
          choosePipeline(input, name);
        });
        comboListEl.appendChild(li);
      }
    }
    input.classList.toggle("combo-regex-bad", !regexOk && q !== "");
    comboActiveIndex = -1;
    positionComboList(input);
    comboListEl.hidden = false;
    input.setAttribute("aria-expanded", "true");
  }

  function closePipelineCombo() {
    if (comboListEl) comboListEl.hidden = true;
    if (comboOwnerInput) comboOwnerInput.setAttribute("aria-expanded", "false");
    comboOwnerInput = null;
    comboActiveIndex = -1;
  }

  function choosePipeline(input, name) {
    if (input) input.value = name;
    const form = input && input.closest("form");
    const sel = form && $(".pipeline-name-select", form);
    if (sel) {
      const has = Array.from(sel.options).some(o => o.value === name);
      if (!has) {
        const opt = document.createElement("option");
        opt.value = name;
        opt.textContent = name;
        sel.appendChild(opt);
      }
      sel.value = name;
    }
    closePipelineCombo();
    if (isTauri && form) form.dispatchEvent(new Event("submit", { cancelable: true }));
  }

  function moveComboActive(input, delta) {
    if (!input) return;
    if (!comboListEl || comboListEl.hidden || comboOwnerInput !== input) {
      renderPipelineComboList(input);
      return;
    }
    const items = Array.from(comboListEl.querySelectorAll(".combo-item"));
    if (items.length === 0) return;
    if (comboActiveIndex >= 0 && items[comboActiveIndex]) {
      items[comboActiveIndex].classList.remove("active");
    }
    comboActiveIndex = (comboActiveIndex + delta + items.length) % items.length;
    const active = items[comboActiveIndex];
    active.classList.add("active");
    active.scrollIntoView({ block: "nearest" });
  }

  function wirePipelineCombo(form) {
    const input = $(".pipeline-name-search", form);
    if (!input || input.dataset.wired === "1") return;
    input.dataset.wired = "1";
    input._pipelineNames = input._pipelineNames || [];
    ensureComboListEl();

    input.addEventListener("focus", () => renderPipelineComboList(input));
    input.addEventListener("input", () => renderPipelineComboList(input));
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        moveComboActive(input, 1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        moveComboActive(input, -1);
      } else if (e.key === "Enter") {
        if (comboListEl && !comboListEl.hidden && comboOwnerInput === input) {
          e.preventDefault();
          const items = Array.from(comboListEl.querySelectorAll(".combo-item"));
          const pick = (comboActiveIndex >= 0 && items[comboActiveIndex]) || items[0];
          if (pick) choosePipeline(input, pick.dataset.name);
        }
      } else if (e.key === "Escape") {
        closePipelineCombo();
      }
    });
    input.addEventListener("blur", () => setTimeout(() => {
      if (comboOwnerInput === input) closePipelineCombo();
    }, 120));

    document.addEventListener("scroll", (e) => {
      // Scrolling inside the dropdown itself must not close it — only
      // page/widget scrolls that would leave the fixed list floating.
      if (comboListEl && !comboListEl.hidden && !comboListEl.contains(e.target)) closePipelineCombo();
    }, true);
    window.addEventListener("resize", () => {
      if (comboListEl && !comboListEl.hidden) closePipelineCombo();
    });
    document.addEventListener("mousedown", (e) => {
      if (!comboListEl || comboListEl.hidden) return;
      if (comboOwnerInput && (comboOwnerInput.contains(e.target) || comboListEl.contains(e.target))) return;
      closePipelineCombo();
    });
  }

  async function loadPipelineList(target) {
    if (!isTauri) return;
    const widget = target && target.closest ? target.closest(".widget") : target;
    if (!widget) return;
    const sel = $(".pipeline-name-select", widget);
    const input = $(".pipeline-name-search", widget);
    const hint = $(".pipeline-name-hint", widget);
    if (!sel || !input) return;
    closePipelineCombo();
    const request = beginOwnedRequest(input);
    beginResultRequest(request, "pipeline-names", {});
    const host = request.resultHost;
    const retained = !!host._resultView;
    if (!retained) {
      clear(sel);
      input._pipelineNames = [];
      input.value = "";
      input.disabled = true;
      sel.disabled = true;
      input.placeholder = "Loading pipeline names…";
    }
    if (hint) {
      clear(hint);
      hint.appendChild(el("button", { type: "button", class: "exec-btn small pipeline-list-retry",
        onclick: () => loadPipelineList(widget) }, "Retry pipeline list"));
    }
    try {
      if (!request.allowed) throw new Error("Select and verify an AWS account first.");
      const res = await tauriInvoke("aws_list_pipelines", {
        params: { context: request.context, request_id: request.id },
      });
      if (!request.accept(res)) return;
      renderArrayResult(host, res, "pipelines", pipelines => {
        const selected = selectedPipelineName($(".pipeline-config", widget));
        const ordered = pipelines.slice().sort((a, b) => String(b.updated || "").localeCompare(String(a.updated || "")));
        clear(sel);
        ordered.forEach(pipeline => sel.appendChild(el("option", { value: pipeline.name }, pipeline.name)));
        input._pipelineNames = ordered.map(pipeline => pipeline.name);
        sel.disabled = input.disabled = ordered.length === 0;
        sel.selectedIndex = -1;
        input.value = "";
        if (selected && input._pipelineNames.includes(selected)) {
          addSelectedPipelineOption($(".pipeline-config", widget), selected);
          input.value = selected;
        }
        input.placeholder = ordered.length ? "search pipelines (regex)..." : resultFailure(res)
          ? "Pipeline names are unavailable or incomplete" : "No pipeline names returned";
        clear(host);
        host.appendChild(el("span", {}, `${ordered.length} pipeline names returned.`));
      });
      if (resultFailure(res) && !host._resultView) input.placeholder = "Pipeline names could not be loaded. Retry.";
    } catch (_) {
      if (!request.current()) return;
      failResult(host, request, "Pipeline names could not be loaded. Retry pipeline list.");
      if (!retained) input.placeholder = "Pipeline names could not be loaded. Retry.";
    }
  }

  function loadPipelineListsForInheritedWidgets() {
    if (!isTauri) return Promise.resolve();
    const jobs = widgetsOfType("pipeline-runs")
      .filter(widget => contextForTile(widget).mode !== "pinned")
      .map(widget => loadPipelineList(widget));
    return Promise.all(jobs);
  }

  function renderPipelineRunsMockOnly(target) {
    return withWidgets("pipeline-runs", target, (widget) => {
      const body = $(".pipeline-runs-body", widget);
      if (!body) return;
      clear(body);
      const list = el("div", { class: "run-list" });
      Mock.pipelineRuns.forEach((run) => {
        list.appendChild(
          el("div", { class: "run-row" },
            el("span", { class: "status-mark " + run.status, title: run.status }),
            el("div", {},
              el("div", { class: "run-id" }, run.id),
              el("div", { class: "run-time" }, `${run.trigger} · ${run.actor}`),
            ),
            el("span", { class: "badge " + statusToBadge(run.status) }, run.status),
            el("span", { class: "run-time" }, run.time),
          )
        );
      });
      body.appendChild(list);
    });
  }

  const ISO_DATE_TIME_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})$/;
  const DATE_VALUE_COLUMNS = new Set([
    "created",
    "created_at",
    "creation_time",
    "last_published",
    "last_updated",
    "published",
    "started",
    "started_at",
    "time",
    "updated",
    "updated_at",
  ]);
  const STATUS_VALUE_COLUMNS = new Set(["state", "status"]);

  function normalizedColumnKey(column) {
    return String(column ?? "").trim().toLowerCase().replace(/[\s.-]+/g, "_");
  }

  function parseIsoDateTime(value) {
    const raw = String(value ?? "");
    if (!ISO_DATE_TIME_RE.test(raw)) return null;
    const normalized = raw.replace(/\.(\d{3})\d+(Z|[+-]\d{2}:?\d{2})$/, ".$1$2");
    const date = new Date(normalized);
    if (Number.isNaN(date.getTime())) return null;
    return date;
  }

  function sameLocalDate(a, b) {
    return a.getFullYear() === b.getFullYear()
      && a.getMonth() === b.getMonth()
      && a.getDate() === b.getDate();
  }

  function formatLocalTime(date) {
    return new Intl.DateTimeFormat("en-GB", {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(date);
  }

  function formatDateTime(date) {
    const now = new Date();
    const yesterday = new Date(now);
    yesterday.setDate(now.getDate() - 1);
    const time = formatLocalTime(date);
    if (sameLocalDate(date, now)) return `Today, ${time}`;
    if (sameLocalDate(date, yesterday)) return `Yesterday, ${time}`;
    const opts = date.getFullYear() === now.getFullYear()
      ? { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit", hour12: false }
      : { day: "2-digit", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", hour12: false };
    return new Intl.DateTimeFormat("en-GB", opts).format(date);
  }

  function formatDisplayValue(column, value) {
    const raw = String(value ?? "");
    if (!raw) return { text: "" };
    if (raw.toLowerCase().startsWith("error:")) {
      return { text: "Error: " + raw.slice(raw.indexOf(":") + 1).trim() };
    }
    const key = normalizedColumnKey(column);
    const date = (DATE_VALUE_COLUMNS.has(key) || ISO_DATE_TIME_RE.test(raw))
      ? parseIsoDateTime(raw)
      : null;
    if (date) return { text: formatDateTime(date), className: "value-date" };
    if (STATUS_VALUE_COLUMNS.has(key)) return { text: displayName(raw) };
    if (raw === "true") return { text: "Yes" };
    if (raw === "false") return { text: "No" };
    return { text: raw };
  }

  function formatStatusLabel(status) {
    return formatDisplayValue("status", status).text;
  }

  function toggleRowDetail(tr, row, opts, colspan) {
    const next = tr.nextElementSibling;
    if (next && next.classList.contains("row-detail")) {
      next.remove();
      tr.classList.remove("expanded");
      tr.setAttribute("aria-expanded", "false");
      tr.removeAttribute("aria-controls");
      return;
    }
    const cell = el("td", { colspan: String(colspan) });
    cell.appendChild(el("div", { class: "muted small" }, "Loading…"));
    const detailId = `row-detail-${++toggleRowDetail.nextId}`;
    const detail = el("tr", { class: "row-detail", id: detailId }, cell);
    tr.after(detail);
    tr.classList.add("expanded");
    tr.setAttribute("aria-expanded", "true");
    tr.setAttribute("aria-controls", detailId);
    Promise.resolve(opts.expand(row, cell)).catch((e) => {
      if (!cell.isConnected) return;
      clear(cell);
      cell.appendChild(el("div", { class: "muted small" }, "Error: " + e));
    });
  }
  toggleRowDetail.nextId = 0;

  function renderTable(host, spec, opts) {
    if (spec.render !== "table" && spec.render) return dispatchRender(host, spec);
    return renderWithResultState(host, spec, () => renderTableContent(host, spec, opts));
  }

  function renderTableContent(host, spec, opts) {
    clear(host);
    if (spec.render !== "table") {
      host.appendChild(el("pre", { class: "raw-json" }, JSON.stringify(spec, null, 2)));
      return;
    }
    if (spec.error) {
      host.appendChild(el("div", { class: "muted small table-error" }, "Error: " + spec.error));
    }
    const columns = Array.isArray(spec.columns) ? spec.columns : [];
    const rows = Array.isArray(spec.rows) ? spec.rows : [];
    if (!rows.length && resultFailure(spec)) return;
    const expandable = !!(opts && typeof opts.expand === "function");
    const tbody = el("tbody", {},
      ...rows.map(row => {
        const tr = el("tr", {}, ...columns.map(c => {
          const raw = String(row[c] ?? "");
          const custom = opts && typeof opts.renderCell === "function"
            ? opts.renderCell(c, row)
            : null;
          if (custom) return el("td", {}, custom);
          const formatted = formatDisplayValue(c, raw);
          if (c === "status") {
            return el("td", {}, el("span", { class: "badge " + statusToBadge(raw) }, formatted.text));
          }
          const attrs = {};
          if (raw && raw !== formatted.text) attrs.title = raw;
          if (formatted.className) attrs.class = formatted.className;
          return el("td", attrs, formatted.text);
        }));
        // Whole-row haystack (all columns, original case) for the filter below.
        tr.dataset.search = columns.map(c => String(row[c] ?? "")).join(" ");
        if (expandable) {
          tr.classList.add("expandable");
          tr.tabIndex = 0;
          tr.setAttribute("aria-expanded", "false");
          tr.addEventListener("click", (event) => {
            if (event.target.closest && event.target.closest("button, a, input, select, textarea")) return;
            toggleRowDetail(tr, row, opts, columns.length);
          });
          tr.addEventListener("keydown", (event) => {
            if (event.target !== tr || (event.key !== "Enter" && event.key !== " ")) return;
            event.preventDefault();
            toggleRowDetail(tr, row, opts, columns.length);
          });
        }
        return tr;
      }),
    );
    const table = el("table", { class: "events-table" },
      el("thead", {}, el("tr", {}, ...columns.map(c => el("th", { title: c }, tableHeaderLabel(c))))),
      tbody,
    );

    // Regex filter — normally only for tables big enough that searching helps,
    // with an opt-in for lookup-oriented tables such as CloudFormation stacks.
    // Matches every column; invalid regex falls back to a case-insensitive
    // substring match (with a warning border), mirroring the pipeline picker.
    if (rows.length > 10 || (opts && opts.alwaysFilter)) {
      const filterPlaceholder = (opts && opts.filterPlaceholder) || "filter rows (regex)…";
      const filter = el("input", {
        class: "table-filter", type: "text", placeholder: filterPlaceholder,
        "aria-label": filterPlaceholder,
        autocomplete: "off", autocorrect: "off", spellcheck: "false",
      });
      const count = el("span", { class: "table-filter-count muted small" }, String(rows.length));
      const empty = el("div", {
        class: "table-filter-empty",
        role: "status",
        "aria-live": "polite",
        hidden: true,
      }, (opts && opts.filterEmptyText) || "No matching rows.");
      const trs = Array.from(tbody.children);
      const applyFilter = () => {
        const q = filter.value.trim();
        const { ok, test } = rowFilterMatcher(q);
        let shown = 0;
        for (const tr of trs) {
          const match = test(tr.dataset.search);
          tr.hidden = !match;
          // Keep an expanded detail row in lockstep with its data row.
          const d = tr.nextElementSibling;
          if (d && d.classList.contains("row-detail")) d.hidden = !match;
          if (match) shown++;
        }
        filter.classList.toggle("table-filter-bad", !ok);
        count.textContent = q ? `${shown} / ${rows.length}` : String(rows.length);
        const noMatches = q !== "" && shown === 0;
        empty.hidden = !noMatches;
        table.hidden = noMatches;
        // Filtering can sharply reduce this scroll container's height. Reset
        // its local scroll position so WebKit keeps the sticky search control
        // painted and the first remaining match is immediately visible.
        host.scrollTop = 0;
      };
      filter.addEventListener("input", applyFilter);
      host.appendChild(el("div", { class: "table-filter-bar" }, filter, count));
      host.appendChild(empty);
    }
    host.appendChild(table);
  }

  function rowFilterMatcher(query) {
    const q = String(query || "").trim();
    if (!q) return { ok: true, test: () => true };
    try {
      const re = new RegExp(q, "i");
      return { ok: true, test: (s) => re.test(String(s || "")) };
    } catch (_e) {
      const lq = q.toLowerCase();
      return { ok: false, test: (s) => String(s || "").toLowerCase().includes(lq) };
    }
  }

  function statusToBadge(status) {
    const exact = ({
      Succeeded:  "badge-success",
      Failed:     "badge-error",
      InProgress: "badge-progress",
      Stopped:    "badge-neutral",
    })[status];
    if (exact) return exact;
    const s = String(status || "").replace(CAMEL_WORD_SPLIT, "$1_$2").toUpperCase().replace(/[\s-]+/g, "_");
    if (s.includes("FAILED") || s.includes("ROLLBACK")) return "badge-error";
    if (s.includes("IN_PROGRESS")) return "badge-progress";
    if (s.includes("COMPLETE")) return "badge-success";
    return "badge-neutral";
  }

  function renderStackDetail(host, spec) {
    clear(host);
    const resources = (spec && Array.isArray(spec.resources)) ? spec.resources : [];
    const events = (spec && Array.isArray(spec.events)) ? spec.events : [];
    const instanceId = ++renderStackDetail.nextId;
    const resourcePanelId = `stack-resources-${instanceId}`;
    const eventPanelId = `stack-events-${instanceId}`;
    const resourceTabId = `stack-resources-tab-${instanceId}`;
    const eventTabId = `stack-events-tab-${instanceId}`;

    const shell = el("div", { class: "stack-detail-shell" });
    if (spec && spec.error) {
      shell.appendChild(el("div", { class: "muted small stack-detail-error" }, "Error: " + spec.error));
    }

    const resourceTab = el("button", {
      class: "stack-detail-tab active",
      type: "button",
      role: "tab",
      id: resourceTabId,
      "aria-controls": resourcePanelId,
      "aria-selected": "true",
    },
    el("span", {}, "Resources"),
    el("span", { class: "stack-detail-tab-count" }, String(resources.length)));
    const eventTab = el("button", {
      class: "stack-detail-tab",
      type: "button",
      role: "tab",
      id: eventTabId,
      "aria-controls": eventPanelId,
      "aria-selected": "false",
      tabindex: "-1",
    },
    el("span", {}, "Events"),
    el("span", { class: "stack-detail-tab-count" }, String(events.length)));
    const tabs = el("div", {
      class: "stack-detail-tabs",
      role: "tablist",
      "aria-label": "Stack detail view",
    }, resourceTab, eventTab);

    const resourcePanel = el("div", {
      class: "stack-detail-panel",
      id: resourcePanelId,
      role: "tabpanel",
      "aria-labelledby": resourceTabId,
    });

    if (resources.length === 0) {
      resourcePanel.appendChild(el("div", { class: "muted small stack-detail-empty" }, sectionEmptyMessage(spec, "resources", "No resources returned for this stack.")));
    } else {
      resourcePanel.appendChild(el("table", { class: "events-table stack-resources-table" },
        el("colgroup", {},
          el("col", { class: "stack-resource-logical-col" }),
          el("col", { class: "stack-resource-type-col" }),
          el("col", { class: "stack-resource-status-col" }),
          el("col", { class: "stack-resource-physical-col" })),
        el("thead", {}, el("tr", {},
          el("th", {}, tableHeaderLabel("logical_id")),
          el("th", {}, tableHeaderLabel("type")),
          el("th", {}, tableHeaderLabel("status")),
          el("th", {}, tableHeaderLabel("physical_id")))),
        el("tbody", {}, ...resources.flatMap((r) => {
          const row = el("tr", {},
            el("td", { class: "stack-resource-logical", title: r.logical_id || undefined }, breakableIdentifier(r.logical_id)),
            el("td", { class: "stack-resource-type", title: r.type || undefined }, r.type || ""),
            el("td", { class: "stack-resource-status" }, el("span", { class: "badge " + statusToBadge(r.status) }, formatStatusLabel(r.status))),
            el("td", { class: "stack-resource-physical", title: r.physical_id || undefined }, r.physical_id || ""));
          const detail = el("td", { colspan: "4" });
          appendEvidenceHandoffs(detail, r.handoffs, host, spec);
          return detail.childNodes.length ? [row, el("tr", { class: "stack-resource-handoff-row" }, detail)] : [row];
        })),
      ));
    }

    const eventPanel = el("div", {
      class: "stack-detail-panel",
      id: eventPanelId,
      role: "tabpanel",
      "aria-labelledby": eventTabId,
      hidden: "hidden",
    });
    if (events.length === 0) {
      eventPanel.appendChild(el("div", { class: "muted small stack-detail-empty" }, sectionEmptyMessage(spec, "events", "No recent stack events returned.")));
    } else {
      eventPanel.appendChild(el("table", { class: "events-table stack-events-table" },
        el("colgroup", {},
          el("col", { class: "stack-event-time-col" }),
          el("col", { class: "stack-event-logical-col" }),
          el("col", { class: "stack-event-status-col" }),
          el("col", { class: "stack-event-reason-col" })),
        el("thead", {}, el("tr", {},
          el("th", {}, tableHeaderLabel("time")),
          el("th", {}, tableHeaderLabel("logical_id")),
          el("th", {}, tableHeaderLabel("status")),
          el("th", {}, tableHeaderLabel("reason")))),
        el("tbody", {}, ...events.map((e) => el("tr", {},
          el("td",
            e.time ? { class: "value-date", title: e.time } : {},
            formatDisplayValue("time", e.time).text),
          el("td", { class: "stack-event-logical", title: e.logical_id || undefined }, breakableIdentifier(e.logical_id)),
          el("td", { class: "stack-event-status" }, el("span", { class: "badge " + statusToBadge(e.status) }, formatStatusLabel(e.status))),
          el("td", { class: "stack-event-reason", title: e.reason || undefined }, e.reason || ""),
        ))),
      ));
    }

    const tabEntries = [
      { button: resourceTab, panel: resourcePanel },
      { button: eventTab, panel: eventPanel },
    ];
    const activateTab = (activeButton) => {
      for (const entry of tabEntries) {
        const active = entry.button === activeButton;
        entry.button.classList.toggle("active", active);
        entry.button.setAttribute("aria-selected", String(active));
        entry.button.tabIndex = active ? 0 : -1;
        entry.panel.hidden = !active;
      }
    };
    resourceTab.addEventListener("click", () => activateTab(resourceTab));
    eventTab.addEventListener("click", () => activateTab(eventTab));
    tabs.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault();
      const nextTab = event.target === resourceTab ? eventTab : resourceTab;
      activateTab(nextTab);
      nextTab.focus();
    });

    shell.appendChild(tabs);
    shell.appendChild(resourcePanel);
    shell.appendChild(eventPanel);
    host.appendChild(shell);
  }
  renderStackDetail.nextId = 0;

  // Only console origins deliberately supported by this beta. Match the raw
  // authority as well as the parsed URL: URL() normalizes explicit :443,
  // whitespace and some confusing host spellings before exposing hostname.
  function supportedConsoleUrl(value) {
    if (typeof value !== "string" || value.length > 4096
        || /[\s\u0000-\u001f\u007f\\]/u.test(value)
        || /%(?:0[0-9a-f]|1[0-9a-f]|7f)/i.test(value)) return null;
    const authority = value.match(/^https:\/\/([^/?#]+)(?:[/?#]|$)/i)?.[1].toLowerCase();
    const hosts = ["console.aws.amazon.com", "eu-west-1.console.aws.amazon.com", "us-east-1.console.aws.amazon.com"];
    if (!hosts.includes(authority)) return null;
    try {
      const url = new URL(value);
      return url.protocol === "https:" && !url.username && !url.password && !url.port
        && hosts.includes(url.hostname) ? url.href : null;
    } catch (_) {
      return null;
    }
  }

  function renderExecutionDetail(host, spec) {
    clear(host);
    if (spec && spec.error) {
      host.appendChild(el("div", { class: "muted small" }, "Error: " + spec.error));
    }
    const actions = (spec && Array.isArray(spec.actions)) ? spec.actions : [];
    if (actions.length === 0) {
      host.appendChild(el("div", { class: "muted small" }, resultFailure(spec)
        ? "Action details are unavailable or incomplete." : "No action details returned for this execution."));
      return;
    }
    const list = el("div", { class: "exec-detail" });
    actions.forEach((a) => {
      const block = el("div", { class: "exec-action-block" + (a.error ? " has-error" : "") },
        el("div", { class: "exec-action-head" },
          el("span", { class: "exec-stage" }, a.stage || "—"),
          el("span", { class: "exec-action-name" }, a.action || ""),
          el("span", { class: "badge " + statusToBadge(a.status) }, formatStatusLabel(a.status)),
        ),
        el("div", { class: "exec-action-meta muted small" },
          [a.category, a.provider].filter(Boolean).join(" · ") +
          (a.last_updated ? " · " + formatDisplayValue("last_updated", a.last_updated).text : "")),
      );
      if (a.summary) block.appendChild(el("div", { class: "exec-summary small" }, a.summary));
      if (a.error) block.appendChild(el("div", { class: "exec-error small" }, a.error));
      const actionsRow = el("div", { class: "exec-action-links" });
      const consoleUrl = supportedConsoleUrl(a.external_url);
      if (consoleUrl) {
        actionsRow.appendChild(el("a", {
          class: "exec-link small", href: consoleUrl, target: "_blank", rel: "noopener noreferrer",
        }, "Open in AWS Console ↗"));
        const copyBtn = el("button", {
          class: "exec-btn exec-icon-btn small", type: "button",
          title: "Copy AWS Console link", "aria-label": "Copy AWS Console link",
        }, "⧉");
        copyBtn.addEventListener("click", () => copyToClipboard(consoleUrl, copyBtn));
        actionsRow.appendChild(copyBtn);
      } else if (a.external_url) {
        actionsRow.appendChild(el("span", { class: "muted small" }, "External link unavailable: unsupported console URL."));
      }
      if (actionsRow.childNodes.length) block.appendChild(actionsRow);
      appendEvidenceHandoffs(block, a.handoffs, host, spec);
      if (!a.handoffs) block.appendChild(el("div", { class: "muted small" }, "Linked targets are unknown for this action."));
      list.appendChild(block);
    });
    host.appendChild(list);
  }

  function copyToClipboard(text, btn) {
    const flash = () => {
      if (!btn) return;
      const prev = btn.textContent;
      btn.textContent = btn.classList.contains("exec-icon-btn") ? "✓" : "Copied";
      setTimeout(() => { btn.textContent = prev; }, 1200);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(flash).catch(() => fallbackCopy(text, flash));
    } else {
      fallbackCopy(text, flash);
    }
  }

  function fallbackCopy(text, done) {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); } catch (_e) { /* ignore */ }
    document.body.removeChild(ta);
    done();
  }

  // ===== Render dispatcher =====
  // Every widget_fetch response carries a `render` field that names its shape.
  // Drawing is keyed off that — no widget can inject arbitrary HTML; the
  // closed render-shape catalog is defined in src-tauri/src/widgets/mod.rs.
  function renderRawJson(host, spec) {
    clear(host);
    // Common backend error shape: {render: "raw_json", data: {error: "…"}}.
    // Render that as a one-line muted message instead of a JSON dump.
    const data = spec && spec.data;
    if (data && typeof data === "object" && typeof data.error === "string"
        && Object.keys(data).length <= 2) {
      host.appendChild(el("div", { class: "muted small" }, data.error));
      return;
    }
    host.appendChild(el("pre", { class: "raw-json" }, JSON.stringify(spec, null, 2)));
  }

  function renderError(host, message) {
    if (host._activeResultRequest) return failResult(host, host._activeResultRequest, message || "The request did not complete.");
    clear(host);
    host.appendChild(el("div", { class: "muted small" }, message || "No data returned."));
  }

  function renderPermissionDenied(host, spec) {
    clear(host);
    const action = (spec && spec.action) || "unknown:Action";
    const reason = (spec && spec.reason) || "This request is not allowed by the app or its policy.";
    host.appendChild(
      el("div", { class: "permission-denied" },
        el("div", { class: "permission-denied-icon" }, "🔒"),
        el("div", { class: "permission-denied-action" }, "Request blocked: " + action),
        el("div", { class: "permission-denied-reason muted small" }, reason)
      )
    );
  }

  function dispatchRender(host, spec) {
    return renderWithResultState(host, spec, () => {
      dispatchRenderContent(host, spec);
      appendEvidenceHandoffs(host, spec?.handoffs, host, spec);
    });
  }

  function dispatchRenderContent(host, spec) {
    if (!spec || typeof spec !== "object") {
      renderError(host, "No data returned.");
      return;
    }
    switch (spec.render) {
      case "table":           return renderTable(host, spec);
      case "errors_chart":    return renderErrorsLive(host, spec);
      case "log_stream":      return renderLogStreamLive(host, spec);
      case "reverse_lookup":  return renderReverseLookupLive(host, spec);
      case "execution_detail": return renderExecutionDetail(host, spec);
      case "stack_detail":    return renderStackDetail(host, spec);
      case "permission_denied": return renderPermissionDenied(host, spec);
      case "raw_json":
      default:
        renderRawJson(host, spec);
    }
  }

  function renderErrorsLive(host, spec) {
    renderErrorsBars(host, spec);
  }

  function renderErrorsBars(host, spec) {
    clear(host);
    const rows = Array.isArray(spec.rows) ? spec.rows : [];
    if (spec.error) {
      host.appendChild(el("div", { class: "muted small" }, "Error: " + spec.error));
    }
    const incomplete = spec.partial === true || spec.status === "partial" || spec.status === "failed" || !!spec.error
      || spec.coverage?.completeness === "unknown";
    if (spec.counts && incomplete) {
      const count = (key) => Number.isSafeInteger(spec.counts[key]) && spec.counts[key] >= 0 ? spec.counts[key] : 0;
      host.appendChild(el("div", { class: "muted small" },
        `Query coverage: ${count("succeeded")} succeeded, ${count("failed")} failed, ${count("timed_out")} timed out, ${count("queried")} queried.`));
    }
    if (rows.length === 0) {
      const empty = incomplete
        ? "Error counts are incomplete; absence of rows does not establish zero errors."
        : "No error events in the last " + (spec.hours || "?") + " hours.";
      host.appendChild(el("div", { class: "muted small" }, empty));
      return;
    }
    const max = Math.max(1, ...rows.map(r => Number(r.errors) || 0));
    const wrap = el("div", { class: "bars" });
    const rowEls = [];
    rows.forEach(row => {
      const errors = Number(row.errors) || 0;
      const stack = String(row.stack || "");
      const pct = Math.round((errors / max) * 100);
      const rowEl = el("div", { class: "bar-row", dataset: { search: `${stack} ${errors}` } },
          el("span", { class: "bar-name", title: stack }, stack),
          el("div", { class: "bar-track" },
            el("div", { class: "bar-fill", style: `width: ${pct}%` }),
          ),
          el("span", { class: "bar-count", title: row.count_is_lower_bound ? "At least this many errors were returned; the count is bounded." : undefined }, `${row.count_is_lower_bound ? "≥ " : ""}${errors}`),
      );
      rowEls.push(rowEl);
      wrap.appendChild(rowEl);
    });

    const filter = el("input", {
      class: "table-filter errors-filter",
      type: "text",
      placeholder: "filter stacks or log groups (regex)...",
      autocomplete: "off",
      autocorrect: "off",
      spellcheck: "false",
    });
    const count = el("span", { class: "table-filter-count muted small" }, String(rows.length));
    const empty = el("div", { class: "lookup-empty errors-filter-empty", hidden: true },
      "No matching stacks or log groups.");
    const applyFilter = () => {
      const q = filter.value.trim();
      const { ok, test } = rowFilterMatcher(q);
      let shown = 0;
      rowEls.forEach((rowEl) => {
        const match = test(rowEl.dataset.search);
        rowEl.hidden = !match;
        if (match) shown++;
      });
      filter.classList.toggle("table-filter-bad", !ok);
      count.textContent = q ? `${shown} / ${rows.length}` : String(rows.length);
      empty.hidden = shown !== 0;
      wrap.hidden = shown === 0;
    };
    filter.addEventListener("input", applyFilter);
    host.appendChild(el("div", { class: "table-filter-bar errors-filter-bar" }, filter, count));
    host.appendChild(empty);
    host.appendChild(wrap);
  }

  function renderLogStreamLive(host, spec) {
    clear(host);
    if (spec.error) {
      host.appendChild(el("div", { class: "muted small" }, "Error: " + spec.error));
    }
    const events = Array.isArray(spec.events) ? spec.events : [];
    if (events.length === 0) {
      host.appendChild(el("div", { class: "muted small" }, resultFailure(spec) || spec.coverage?.completeness === "unknown"
        ? "Log events are unavailable or incomplete; no returned events does not establish an empty log."
        : "No log events returned in the selected window."));
      return;
    }
    // The host already has `.log-stream` CSS in the tile, but we still emit a
    // wrapper so this renderer is host-agnostic when reused elsewhere.
    events.forEach(ev => {
      const level = (ev.level || "info").toLowerCase();
      const cls = level === "error" || level === "warn" ? `log-line ${level}` : "log-line";
      const tsLabel = ev.ts ? new Date(Number(ev.ts)).toLocaleTimeString("en-GB", { hour12: false }) : "";
      host.appendChild(
        el("div", { class: cls },
          el("span", { class: "ts" }, tsLabel),
          el("span", { class: "msg" }, ev.msg || ""),
        )
      );
    });
    host.scrollTop = host.scrollHeight;
  }

  function renderReverseLookupLive(host, spec) {
    clear(host);
    const matches = Array.isArray(spec.matches) ? spec.matches : [];
    if (spec.error) {
      host.appendChild(el("div", { class: "muted small" }, "Error: " + spec.error));
    }
    if (matches.length === 0) {
      const region = spec.region ? ` in ${spec.region}` : "";
      const scanned = typeof spec.scanned === "number"
        ? ` Searched ${spec.scanned} tagged resource${spec.scanned === 1 ? "" : "s"}${spec.capped ? "+ (capped)" : ""}.`
        : "";
      host.appendChild(el("div", { class: "lookup-empty" }, resultFailure(spec) ? "Resource search is incomplete; no returned matches does not establish absence." : `No matching resources found${region}.`));
      host.appendChild(el("div", { class: "muted small" },
        "Only tagged resources in this widget's account and region can be searched. The Resource Groups Tagging API does not cover every AWS resource type." + scanned));
      return;
    }
    matches.forEach(m => {
      const type = m.type || "";
      const link = reviewedHandoff(m.handoffs?.stack, "stack");
      const confirmed = link?.status === "available" && link.source === "cfn_ownership";
      const row = el("div", { class: "lookup-row" },
          el("div", {},
            el("div", { class: "resource" }, m.arn || ""),
            el("div", { class: "meta" }, confirmed ? `Matched stack: ${link.inputs.stack_name}${type ? ` · ${type}` : ""}` : "Stack ownership is unknown."),
          ),
          el("span", { class: "badge " + (confirmed ? "badge-success" : "badge-neutral") }, confirmed ? "Matched Stack" : "Unknown"));
      host.appendChild(row);
      appendEvidenceHandoffs(host, m.handoffs, host, spec);
    });
  }

  function cfnEventBadge(status) {
    if (status.includes("FAILED"))  return "badge-error";
    if (status.includes("ROLLBACK")) return "badge-warning";
    if (status.includes("COMPLETE")) return "badge-success";
    if (status.includes("IN_PROGRESS")) return "badge-progress";
    return "badge-neutral";
  }

  // ===== Lambda logs =====
  function renderLogTail(target) {
    if (isTauri) {
      return renderLogTailFromSidecar(target);
    }
    return renderLogTailMock(target);
  }

  function renderLogTailFromSidecar(target) {
    return withWidgets("log-tail", target, (widget) => {
      const body = $(".log-tail-body", widget);
      if (!body) return;
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      if (reuseResultBrowser(body)) return;
      body.classList.remove("log-stream");
      body.classList.add("lambda-log-browser");
      invalidateRequests(body);
      clear(body);

      const state = { functions: [], selected: null, streams: [] };
      const search = el("input", {
        class: "lambda-search-input",
        type: "search",
        placeholder: "Search Lambda functions...",
      });
      const reloadBtn = el("button", { class: "btn", type: "button" }, "Reload");
      const statusHost = el("div", { class: "lambda-status muted small" });
      const listHost = el("div", { class: "lambda-list" });
      const detailHost = el("div", { class: "lambda-detail" });
      const streamHost = el("div", { class: "lambda-stream-panel" });
      const logHost = el("div", { class: "log-stream lambda-log-events", hidden: true });

      const layout = el("div", { class: "lambda-log-layout" },
        el("section", { class: "lambda-browser-pane" },
          el("div", { class: "lambda-toolbar" },
            search,
            reloadBtn,
          ),
          statusHost,
          listHost,
        ),
        el("section", { class: "lambda-log-pane" },
          detailHost,
          streamHost,
          logHost,
        ),
      );

      function setStatus(message) {
        clear(statusHost);
        if (message) statusHost.appendChild(el("div", {}, message));
      }

      function showBackendProblem(host, result, fallback) {
        clear(host);
        if (result && result.render) {
          dispatchRender(host, result);
        } else {
          host.appendChild(el("div", { class: "muted small" }, fallback || (result && result.error) || "Request failed."));
        }
      }

      function filteredFunctions() {
        const q = search.value.trim().toLowerCase();
        if (!q) return state.functions;
        return state.functions.filter(fn =>
          [fn.name, fn.arn, fn.runtime, fn.handler, fn.state]
            .filter(Boolean)
            .some(value => String(value).toLowerCase().includes(q))
        );
      }

      function timeLabel(ms) {
        const n = Number(ms) || 0;
        return n ? new Date(n).toLocaleString("en-GB", { hour12: false }) : "No events";
      }

      function infoField(label, value, opts = {}) {
        const text = value === undefined || value === null || value === "" ? "—" : String(value);
        return el("div", { class: "lambda-info-field" },
          el("span", { class: "lambda-info-label" }, label),
          el("span", { class: opts.mono ? "lambda-info-value mono" : "lambda-info-value", title: text }, text),
        );
      }

      function renderFunctionList() {
        clear(listHost);
        const rows = filteredFunctions();
        if (rows.length === 0) {
          listHost.appendChild(el("div", { class: "lambda-empty muted small" },
            state.functions.length ? "No matching functions." : "No Lambda functions returned."));
          restoreResultStatus(listHost);
          return;
        }
        rows.forEach(fn => {
          const active = state.selected && state.selected.name === fn.name;
          const row = el("button", {
            class: "lambda-row" + (active ? " active" : ""),
            type: "button",
            title: fn.arn || fn.name,
          },
            el("span", { class: "lambda-row-name" }, fn.name || "—"),
            el("span", { class: "lambda-row-meta" },
              [fn.runtime, fn.state, fn.last_modified].filter(Boolean).join(" · ")),
          );
          row.addEventListener("click", () => selectLambda(fn));
          listHost.appendChild(row);
        });
        restoreResultStatus(listHost);
      }

      function renderDetail() {
        invalidateRequests(streamHost);
        invalidateRequests(logHost);
        clear(detailHost);
        clear(streamHost);
        clear(logHost);
        logHost.hidden = true;
        const fn = state.selected;
        if (!fn) {
          detailHost.appendChild(el("div", { class: "lambda-empty muted small" }, "Select a Lambda function."));
          return;
        }
        const sub = widget.querySelector(".widget-sub");
        if (sub) sub.textContent = fn.name || "Lambda Logs";

        const copyBtn = el("button", { class: "exec-btn small", type: "button" }, "Copy log group");
        copyBtn.addEventListener("click", () => copyToClipboard(fn.log_group || "", copyBtn));
        const logs = reviewedHandoff(fn.handoffs?.logs, "logs");
        const origin = evidenceOrigin(listHost, state.sourceSpec);
        const loadStreamsBtn = el("button", { class: "exec-btn small", type: "button" },
          logs?.status === "manual" ? "Try conventional log group" : "Load streams");
        loadStreamsBtn.disabled = !logs?.widget || !origin?.current();
        loadStreamsBtn.addEventListener("click", () => loadStreams(fn, true));

        detailHost.appendChild(
          el("div", { class: "lambda-detail-card" },
            el("div", { class: "lambda-detail-head" },
              el("div", {},
                el("div", { class: "lambda-detail-name" }, fn.name || "—"),
                el("div", { class: "lambda-detail-sub muted small" },
                  [fn.runtime, fn.handler].filter(Boolean).join(" · ")),
              ),
              el("div", { class: "lambda-detail-actions" }, copyBtn, loadStreamsBtn),
            ),
            el("div", { class: "small evidence-reason" }, !origin?.current() ? "Refresh the function list before opening linked logs."
              : logs?.reason || "Log-group provenance is unavailable. Reload the function list before opening linked logs."),
            el("div", { class: "lambda-info-grid" },
              infoField("ARN", fn.arn, { mono: true }),
              infoField("Last updated", fn.last_modified),
              infoField("Log group", fn.log_group, { mono: true }),
              infoField("State", fn.state),
              infoField("Memory", fn.memory_mb ? `${fn.memory_mb} MB` : ""),
              infoField("Timeout", fn.timeout_seconds ? `${fn.timeout_seconds}s` : ""),
              infoField("Architecture", Array.isArray(fn.architectures) ? fn.architectures.join(", ") : ""),
              infoField("Code size", fn.code_size ? `${fn.code_size} bytes` : ""),
            ),
          )
        );
      }

      function renderStreams(message) {
        clear(streamHost);
        const fn = state.selected;
        if (!fn) return;
        if (message) {
          streamHost.appendChild(el("div", { class: "muted small" }, message));
          return;
        }
        if (!state.streams.length) {
          streamHost.appendChild(el("div", { class: "muted small" }, "No log streams found for this Lambda log group."));
          return;
        }

        const select = el("select", { class: "lambda-stream-select" });
        state.streams.forEach(stream => {
          select.appendChild(el("option", { value: stream.name },
            `${stream.name} · ${timeLabel(stream.last_event_timestamp)}`));
        });
        select.addEventListener("change", () => {
          invalidateRequests(logHost);
          clear(logHost);
          logHost.hidden = true;
        });
        const producedBy = streamHost._activeResultRequest;
        const viewBtn = el("button", { class: "btn btn-primary", type: "button" }, "View log");
        viewBtn.addEventListener("click", () => { if (select.isConnected) loadEvents(select.value, producedBy); });
        const controls = el("div", { class: "lambda-stream-controls evidence-navigation" },
            el("label", {},
              el("span", {}, "Log stream"),
              select,
            ),
            viewBtn,
            el("div", { class: "small evidence-stale", hidden: true }, "Refresh the source before opening linked evidence."));
        controls._evidenceCurrent = () => producedBy?.current() && streamHost._resultContext?.id === producedBy.id;
        streamHost.appendChild(controls);
      }

      async function loadFunctions() {
        setStatus("Loading Lambda functions...");
        clear(listHost);
        clear(detailHost);
        clear(streamHost);
        clear(logHost);
        logHost.hidden = true;
        invalidateRequests(streamHost);
        invalidateRequests(logHost);
        state.selected = null;
        state.streams = [];
        const request = beginOwnedRequest(listHost);
        try {
          const result = await fetchWidgetData("log-tail", { mode: "list", max_functions: 500 }, listHost, null, request);
          if (!request.accept(result)) return;
          setStatus("");
          renderArrayResult(listHost, result, "functions", functions => {
            state.sourceSpec = result;
            state.functions = functions;
            state.selected = null;
            state.streams = [];
            renderFunctionList();
            renderDetail();
          });
        } catch (err) {
          if (!request.current()) return;
          setStatus("");
          failResult(listHost, request, String(err));
        }
      }

      async function selectLambda(fn) {
        state.selected = fn;
        state.streams = [];
        renderFunctionList();
        renderDetail();
        await loadStreams(fn);
      }

      async function loadStreams(fn, manual = false) {
        const logs = reviewedHandoff(fn?.handoffs?.logs, "logs");
        const origin = evidenceOrigin(listHost, state.sourceSpec);
        if (!logs?.widget || !origin?.current() || (logs.status === "manual" && !manual)) return;
        if (state.selected !== fn) return;
        invalidateRequests(logHost);
        renderStreams("Loading streams...");
        clear(logHost);
        logHost.hidden = true;
        streamHost._evidenceSourceCurrent = origin.current;
        const request = beginOwnedRequest(streamHost, origin.context);
        try {
          const result = await fetchWidgetData(
            "log-tail",
            { ...logs.inputs, max_streams: 50 },
            streamHost, origin.context, request,
          );
          if (!request.accept(result)) return;
          renderArrayResult(streamHost, result, "streams", streams => {
            state.streams = streams;
            renderStreams();
          });
        } catch (err) {
          if (!request.current()) return;
          failResult(streamHost, request, String(err));
        }
      }

      async function loadEvents(streamName, streamRequest) {
        const fn = state.selected;
        const logs = reviewedHandoff(fn?.handoffs?.logs, "logs");
        const origin = evidenceOrigin(listHost, state.sourceSpec);
        if (!logs?.widget || !streamName || !origin?.current() || !streamRequest?.current()
            || streamHost._activeResultRequest !== streamRequest
            || streamHost._resultContext?.id !== streamRequest.id) return;
        clear(logHost);
        logHost.hidden = false;
        logHost.appendChild(el("div", { class: "muted small" }, "Loading log events..."));
        logHost._evidenceSourceCurrent = () => origin.current() && streamRequest.current();
        const request = beginOwnedRequest(logHost, origin.context);
        try {
          const result = await fetchWidgetData(
            "log-tail",
            { mode: "events", log_group: logs.inputs.log_group, log_stream: streamName, limit: 500 },
            logHost, origin.context, request,
          );
          if (!request.accept(result)) return;
          dispatchRender(logHost, result);
        } catch (err) {
          if (!request.current()) return;
          failResult(logHost, request, String(err));
        }
      }

      search.addEventListener("input", renderFunctionList);
      reloadBtn.addEventListener("click", loadFunctions);
      body.appendChild(layout);
      body._browserGeneration = body._requestGeneration || 0;
      body._reloadResult = loadFunctions;
      loadFunctions();
    });
  }

  function renderLogTailMock(target) {
    return withWidgets("log-tail", target, (widget) => {
      const body = $(".log-tail-body", widget);
      if (!body) return;
      body.classList.remove("lambda-log-browser");
      body.classList.add("log-stream");
      clear(body);
      let idx = 0;
      function appendOne() {
        const sample = Mock.logPool[idx % Mock.logPool.length];
        idx++;
        const ts = new Date().toLocaleTimeString("en-GB", { hour12: false });
        body.appendChild(
          el("div", { class: "log-line " + (sample.level === "info" ? "" : sample.level) },
            el("span", { class: "ts" }, ts),
            el("span", { class: "msg" }, sample.msg),
          )
        )
        while (body.childElementCount > 80) body.removeChild(body.firstChild);
        body.scrollTop = body.scrollHeight;
      }
      for (let i = 0; i < 12; i++) appendOne();
      if (body._logTailTimer) clearInterval(body._logTailTimer);
      body._logTailTimer = setInterval(appendOne, 1800);
    });
  }

  // ===== CloudWatch Logs (generic log group browser) =====
  // Same three-step flow as Lambda Logs — list, streams, events — but the
  // entry point is a searchable log group list instead of Lambda functions.
  // Reuses the lambda-log-browser CSS wholesale.
  function renderCloudwatchLogs(target) {
    return withWidgets("cloudwatch-logs", target, (widget) => {
      const body = $(".cw-logs-body", widget);
      if (!body) return;
      if (reuseResultBrowser(body)) return;
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      invalidateRequests(body);
      clear(body);
      if (!isTauri) {
        body.appendChild(el("div", { class: "muted small" },
          "CloudWatch Logs browsing needs the desktop app — no mock data."));
        return;
      }

      const state = { groups: [], selected: null, streams: [], capped: false };
      const search = el("input", {
        class: "lambda-search-input",
        type: "search",
        placeholder: "Filter log groups (Enter searches AWS)...",
      });
      const reloadBtn = el("button", { class: "btn", type: "button" }, "Reload");
      const statusHost = el("div", { class: "lambda-status muted small" });
      const listHost = el("div", { class: "lambda-list" });
      const detailHost = el("div", { class: "lambda-detail" });
      const streamHost = el("div", { class: "lambda-stream-panel" });
      const logHost = el("div", { class: "log-stream lambda-log-events", hidden: true });

      const layout = el("div", { class: "lambda-log-layout" },
        el("section", { class: "lambda-browser-pane" },
          el("div", { class: "lambda-toolbar" }, search, reloadBtn),
          statusHost,
          listHost,
        ),
        el("section", { class: "lambda-log-pane" },
          detailHost,
          streamHost,
          logHost,
        ),
      );

      function setStatus(message) {
        clear(statusHost);
        if (message) statusHost.appendChild(el("div", {}, message));
      }

      function showBackendProblem(host, result, fallback) {
        clear(host);
        if (result && result.render) {
          dispatchRender(host, result);
        } else {
          host.appendChild(el("div", { class: "muted small" }, fallback || (result && result.error) || "Request failed."));
        }
      }

      function timeLabel(ms) {
        const n = Number(ms) || 0;
        return n ? new Date(n).toLocaleString("en-GB", { hour12: false }) : "No events";
      }

      function retentionLabel(days) {
        const n = Number(days) || 0;
        return n ? `${n}d retention` : "no expiry";
      }

      function bytesLabel(bytes) {
        let n = Number(bytes) || 0;
        if (!n) return "0 B";
        const units = ["B", "KB", "MB", "GB", "TB"];
        let i = 0;
        while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
        return `${n >= 10 || i === 0 ? Math.round(n) : n.toFixed(1)} ${units[i]}`;
      }

      function infoField(label, value, opts = {}) {
        const text = value === undefined || value === null || value === "" ? "—" : String(value);
        return el("div", { class: "lambda-info-field" },
          el("span", { class: "lambda-info-label" }, label),
          el("span", { class: opts.mono ? "lambda-info-value mono" : "lambda-info-value", title: text }, text),
        );
      }

      function filteredGroups() {
        const q = search.value.trim().toLowerCase();
        if (!q) return state.groups;
        return state.groups.filter(g => String(g.name || "").toLowerCase().includes(q));
      }

      function renderGroupList() {
        clear(listHost);
        const rows = filteredGroups();
        if (rows.length === 0) {
          listHost.appendChild(el("div", { class: "lambda-empty muted small" },
            state.groups.length
              ? (state.capped ? "No loaded group matches — press Enter to search AWS." : "No matching log groups.")
              : "No log groups returned."));
          restoreResultStatus(listHost);
          return;
        }
        rows.forEach(group => {
          const active = state.selected && state.selected.name === group.name;
          const row = el("button", {
            class: "lambda-row" + (active ? " active" : ""),
            type: "button",
            title: group.arn || group.name,
          },
            el("span", { class: "lambda-row-name" }, envLightNodes(group.name || "—")),
            el("span", { class: "lambda-row-meta" },
              [retentionLabel(group.retention_days), bytesLabel(group.stored_bytes)].join(" · ")),
          );
          row.addEventListener("click", () => selectGroup(group));
          listHost.appendChild(row);
        });
        restoreResultStatus(listHost);
      }

      function renderDetail() {
        invalidateRequests(streamHost);
        invalidateRequests(logHost);
        clear(detailHost);
        clear(streamHost);
        clear(logHost);
        logHost.hidden = true;
        const group = state.selected;
        if (!group) {
          detailHost.appendChild(el("div", { class: "lambda-empty muted small" }, "Select a log group."));
          return;
        }
        const sub = widget.querySelector(".widget-sub");
        if (sub) sub.textContent = group.name || "CloudWatch Logs";

        const copyBtn = el("button", { class: "exec-btn small", type: "button" }, "Copy log group");
        copyBtn.addEventListener("click", () => copyToClipboard(group.name || "", copyBtn));
        const reloadStreamsBtn = el("button", { class: "exec-btn small", type: "button" }, "Reload streams");
        reloadStreamsBtn.addEventListener("click", () => loadStreams(group));

        detailHost.appendChild(
          el("div", { class: "lambda-detail-card" },
            el("div", { class: "lambda-detail-head" },
              el("div", {},
                el("div", { class: "lambda-detail-name" }, envLightNodes(group.name || "—")),
                el("div", { class: "lambda-detail-sub muted small" },
                  [retentionLabel(group.retention_days), bytesLabel(group.stored_bytes)].join(" · ")),
              ),
              el("div", { class: "lambda-detail-actions" }, copyBtn, reloadStreamsBtn),
            ),
            el("div", { class: "lambda-info-grid" },
              infoField("Log group", group.name, { mono: true }),
              infoField("Created", timeLabel(group.creation_time)),
              infoField("Retention", retentionLabel(group.retention_days)),
              infoField("Stored", bytesLabel(group.stored_bytes)),
            ),
          )
        );
      }

      function renderStreams(message) {
        clear(streamHost);
        const group = state.selected;
        if (!group) return;
        if (message) {
          streamHost.appendChild(el("div", { class: "muted small" }, message));
          return;
        }
        if (!state.streams.length) {
          streamHost.appendChild(el("div", { class: "muted small" }, "No log streams in this log group."));
          return;
        }
        const select = el("select", { class: "lambda-stream-select" });
        state.streams.forEach(stream => {
          select.appendChild(el("option", { value: stream.name },
            `${stream.name} · ${timeLabel(stream.last_event_timestamp)}`));
        });
        select.addEventListener("change", () => {
          invalidateRequests(logHost);
          clear(logHost);
          logHost.hidden = true;
        });
        const viewBtn = el("button", { class: "btn btn-primary", type: "button" }, "View log");
        viewBtn.addEventListener("click", () => { if (select.isConnected) loadEvents(select.value); });
        streamHost.appendChild(
          el("div", { class: "lambda-stream-controls" },
            el("label", {},
              el("span", {}, "Log stream"),
              select,
            ),
            viewBtn,
          )
        );
      }

      async function loadGroups(pattern) {
        setStatus("Loading log groups...");
        clear(listHost);
        clear(detailHost);
        clear(streamHost);
        clear(logHost);
        logHost.hidden = true;
        invalidateRequests(streamHost);
        invalidateRequests(logHost);
        state.selected = null;
        state.streams = [];
        const request = beginOwnedRequest(listHost);
        try {
          const inputs = { mode: "groups", max_groups: 500 };
          if (pattern) inputs.name_pattern = pattern;
          const result = await fetchWidgetData("cloudwatch-logs", inputs, listHost, null, request);
          if (!request.accept(result)) return;
          setStatus("");
          renderArrayResult(listHost, result, "groups", groups => {
            state.groups = groups;
            state.capped = result.capped === true;
            state.selected = null;
            state.streams = [];
            renderGroupList();
            renderDetail();
          });
        } catch (err) {
          if (!request.current()) return;
          setStatus("");
          failResult(listHost, request, String(err));
        }
      }

      async function selectGroup(group) {
        state.selected = group;
        state.streams = [];
        renderGroupList();
        renderDetail();
        await loadStreams(group);
      }

      async function loadStreams(group) {
        if (!group || !group.name) return;
        if (state.selected !== group) return;
        invalidateRequests(logHost);
        renderStreams("Loading streams...");
        clear(logHost);
        logHost.hidden = true;
        const request = beginOwnedRequest(streamHost);
        try {
          const result = await fetchWidgetData(
            "cloudwatch-logs",
            { mode: "streams", log_group: group.name, max_streams: 50 },
            streamHost, null, request,
          );
          if (!request.accept(result)) return;
          renderArrayResult(streamHost, result, "streams", streams => {
            state.streams = streams;
            renderStreams();
          });
        } catch (err) {
          if (!request.current()) return;
          failResult(streamHost, request, String(err));
        }
      }

      async function loadEvents(streamName) {
        const group = state.selected;
        if (!group || !group.name || !streamName) return;
        clear(logHost);
        logHost.hidden = false;
        logHost.appendChild(el("div", { class: "muted small" }, "Loading log events..."));
        const request = beginOwnedRequest(logHost);
        try {
          const result = await fetchWidgetData(
            "cloudwatch-logs",
            { mode: "events", log_group: group.name, log_stream: streamName, limit: 500 },
            logHost, null, request,
          );
          if (!request.accept(result)) return;
          dispatchRender(logHost, result);
        } catch (err) {
          if (!request.current()) return;
          failResult(logHost, request, String(err));
        }
      }

      search.addEventListener("input", renderGroupList);
      search.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          loadGroups(search.value.trim());
        }
      });
      reloadBtn.addEventListener("click", () => loadGroups(search.value.trim()));
      body.appendChild(layout);
      body._browserGeneration = body._requestGeneration || 0;
      body._reloadResult = () => loadGroups(search.value.trim());
      loadGroups("");
    });
  }

  // ===== CFN stacks =====
  async function renderCfnStacks(target) {
    if (isTauri) return renderCfnStacksFromSidecar(target);
    return renderCfnStacksMock(target);
  }

  async function renderCfnStacksFromSidecar(target) {
    return withWidgets("cfn-stacks", target, async (widget) => {
      const body = $(".cfn-stacks-body", widget);
      if (!body) return;
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      clear(body);
      body.appendChild(el("div", { class: "muted small" }, "Loading..."));
      const request = beginOwnedRequest(body);
      try {
        const result = await fetchWidgetData("cfn-stacks", {}, body, null, request);
        if (!request.accept(result)) return;
        if (result && result.render === "table") {
          renderTable(body, result, {
            alwaysFilter: true,
            filterPlaceholder: "Search stacks (name, status, or date)…",
            filterEmptyText: "No matching stacks.",
            expand: (row, cell) =>
              fetchWidgetInto(cell, "cfn-stack-detail", { stack_name: row.stack }),
          });
        } else {
          dispatchRender(body, result);
        }
      } catch (e) {
        if (!request.current()) return;
        renderError(body, "Failed to load: " + e);
      }
    });
  }

  function renderCfnStacksMock(target) {
    return withWidgets("cfn-stacks", target, (widget) => {
      const body = $(".cfn-stacks-body", widget);
      if (!body) return;
      const rows = Mock.stacks.map(stack => ({
        stack: stack.name,
        status: stack.status,
        resources: stack.resources.length,
        last_updated: "",
      }));
      renderTable(body, {
        render: "table",
        columns: ["stack", "status", "resources", "last_updated"],
        rows,
      }, {
        alwaysFilter: true,
        filterPlaceholder: "Search stacks (name, status, or date)…",
        filterEmptyText: "No matching stacks.",
        expand: (row, cell) => {
          const stack = Mock.stacks.find(item => item.name === row.stack);
          renderStackDetail(cell, {
            render: "stack_detail",
            stack: row.stack,
            resources: stack ? stack.resources.map((resource) => ({
              logical_id: resource.name,
              type: resource.type,
              status: resource.status || stack.status,
              physical_id: resource.physical_id || resource.name,
            })) : [],
            events: stack && Array.isArray(stack.events) ? stack.events : [],
          });
        },
      });
    });
  }

  // ===== Resource reverse lookup =====
  let lookupDebounceTimer = null;

  function renderLookup(target) {
    if (isTauri) return renderLookupLiveBind(target);
    return renderLookupMock(target);
  }

  function renderLookupLiveBind(target) {
    return withWidgets("resource-lookup", target, (widget) => {
      const old = $(".lookup-input", widget);
      const results = $(".lookup-results", widget);
      if (!old || !results) return;
      if (old._lookupResultGeneration === (results._requestGeneration || 0) && old.value.trim()) {
        old.dispatchEvent(new Event("input"));
        return;
      }
      if (old._lookupDebounceTimer) clearTimeout(old._lookupDebounceTimer);
      invalidateRequests(results);
      const input = old.cloneNode(true);
      old.parentNode.replaceChild(input, old);
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      clear(results);
      results.appendChild(el("div", { class: "lookup-empty" },
        "Search tagged resources by name, ARN or partial id; stack ownership may remain unknown."));
      input.addEventListener("input", () => {
        if (input._lookupDebounceTimer) clearTimeout(input._lookupDebounceTimer);
        const request = beginOwnedRequest(results);
        input._lookupResultGeneration = results._requestGeneration || 0;
        const q = input.value.trim();
        clear(results);
        if (!q) {
          results.appendChild(el("div", { class: "lookup-empty" },
            "Search tagged resources by name, ARN or partial id; stack ownership may remain unknown."));
          return;
        }
        input._lookupDebounceTimer = setTimeout(async () => {
          if (!input.isConnected || !request.current()) return;
          results.appendChild(el("div", { class: "lookup-empty muted small" }, "Searching..."));
          try {
            const result = await fetchWidgetData("resource-lookup", { query: q }, results, null, request);
            if (request.accept(result)) dispatchRender(results, result);
          } catch (err) {
            if (request.current()) renderError(results, "Lookup failed: " + err);
          }
        }, 350);
      });
    });
  }

  function renderLookupMock(target) {
    return withWidgets("resource-lookup", target, (widget) => {
      const old = $(".lookup-input", widget);
      if (!old) return;
      const input = old.cloneNode(true);
      old.parentNode.replaceChild(input, old);
      const results = $(".lookup-results", widget);
      function update() {
        const q = input.value.trim().toLowerCase();
        clear(results);
        if (!q) {
          results.appendChild(el("div", { class: "lookup-empty" },
            "Search tagged resources by name, ARN or partial id; stack ownership may remain unknown."));
          return;
        }
        const matches = Mock.allResources.filter(r =>
          r.name.toLowerCase().includes(q) ||
          r.arn.toLowerCase().includes(q) ||
          r.type.toLowerCase().includes(q) ||
          r.stack.toLowerCase().includes(q)
        ).slice(0, 8);
        if (matches.length === 0) {
          results.appendChild(el("div", { class: "lookup-empty" }, "No matching resources found."));
          return;
        }
        matches.forEach(m => {
          results.appendChild(
            el("div", { class: "lookup-row" },
              el("div", {},
                el("div", { class: "resource" }, m.arn),
                el("div", { class: "meta" }, `Owned by ${m.stack} · ${m.type}`),
              ),
              el("span", { class: "badge " + cfnEventBadge(m.stackStatus) }, m.stackStatus),
            ),
          );
        });
      }
      input.addEventListener("input", update);
      update();
    });
  }

  // ===== Errors by stack (bars) =====
  async function renderErrors(target) {
    if (isTauri) return renderErrorsFromSidecar(target);
    return renderErrorsMock(target);
  }

  async function renderErrorsFromSidecar(target) {
    return withWidgets("errors-by-stack", target, async (widget) => {
      const body = $(".errors-body", widget);
      if (!body) return;
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      clear(body);
      body.appendChild(el("div", { class: "muted small" }, "Loading..."));
      const request = beginOwnedRequest(body);
      try {
        const result = await fetchWidgetData("errors-by-stack", {}, body, null, request);
        if (!request.accept(result)) return;
        dispatchRender(body, result);
      } catch (e) {
        if (!request.current()) return;
        renderError(body, "Failed to load: " + e);
      }
    });
  }

  function renderErrorsMock(target) {
    return withWidgets("errors-by-stack", target, (widget) => {
      const body = $(".errors-body", widget);
      if (!body) return;
      renderErrorsBars(body, { hours: 24, rows: Mock.errorsByStack });
    });
  }

  // ===== CodeArtifact packages =====
  const CODEARTIFACT_DEFAULTS = {
    domain: "example-domain",
    repository: "example_pypi_repo",
    package_prefix: "example",
    max_packages: 50,
  };
  const CODEARTIFACT_LEGACY_DOMAIN = "example-legacy-domain";

  function codeArtifactDomainDefault(inputs) {
    if (!inputs.domain || inputs.domain === CODEARTIFACT_LEGACY_DOMAIN) {
      return CODEARTIFACT_DEFAULTS.domain;
    }
    return inputs.domain;
  }

  function applyCodeArtifactConfig(form) {
    const cfg = tileConfig(form);
    const inputs = cfg.inputs || {};
    $(".codeartifact-domain", form).value = codeArtifactDomainDefault(inputs);
    $(".codeartifact-repository", form).value = inputs.repository || CODEARTIFACT_DEFAULTS.repository;
    $(".codeartifact-prefix", form).value = inputs.package_prefix || CODEARTIFACT_DEFAULTS.package_prefix;
    $(".codeartifact-max", form).value = String(inputs.max_packages || CODEARTIFACT_DEFAULTS.max_packages);
  }

  function readCodeArtifactForm(form) {
    const domain = $(".codeartifact-domain", form).value.trim();
    const repository = $(".codeartifact-repository", form).value.trim();
    const packagePrefix = $(".codeartifact-prefix", form).value.trim();
    const maxRaw = Number.parseInt($(".codeartifact-max", form).value, 10);
    const maxPackages = Number.isFinite(maxRaw)
      ? Math.max(1, Math.min(1000, maxRaw))
      : CODEARTIFACT_DEFAULTS.max_packages;
    return {
      domain,
      repository,
      package_prefix: packagePrefix,
      max_packages: maxPackages,
    };
  }

  // Merge widget settings into the tile's persisted config (any element
  // inside the tile works as the anchor).
  function persistTileInputs(elInTile, inputs) {
    const item = tileItemFor(elInTile);
    if (!item) return;
    const cfg = tileConfig(elInTile);
    writeTileConfig(item, {
      ...cfg,
      inputs: {
        ...(cfg.inputs || {}),
        ...inputs,
      },
    });
    scheduleSaveLayout();
  }

  function persistCodeArtifactInputs(form, inputs) {
    persistTileInputs(form, inputs);
  }

  function requestCodeArtifactLoad(form) {
    if (!form) return;
    if (typeof form.requestSubmit === "function") form.requestSubmit();
    else form.dispatchEvent(new Event("submit", { cancelable: true }));
  }

  function codeArtifactCopyButton(version, label) {
    if (!version) return null;
    const button = el("button", {
      class: "exec-btn exec-icon-btn codeartifact-copy-btn",
      type: "button",
      title: label,
      "aria-label": label,
    }, "⧉");
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      copyToClipboard(version, button);
    });
    return button;
  }

  function codeArtifactVersions(row) {
    const candidates = Array.isArray(row.versions) ? row.versions : [row.latest_version];
    const latestVersion = String(row.latest_version || "").trim();
    const latestPublished = parseIsoDateTime(row.last_published) ? row.last_published : "";
    const seen = new Set();
    const versions = [];
    for (const candidate of candidates) {
      const version = String(
        candidate && typeof candidate === "object" ? candidate.version : candidate || ""
      ).trim();
      if (!version || seen.has(version)) continue;
      seen.add(version);
      const rawPublished = candidate && typeof candidate === "object"
        ? (candidate.published || candidate.published_at || candidate.published_time || "")
        : "";
      const published = parseIsoDateTime(rawPublished)
        ? String(rawPublished)
        : (version === latestVersion ? latestPublished : "");
      versions.push({
        version,
        published,
        error: candidate && typeof candidate === "object" ? String(candidate.error || "") : "",
      });
      if (versions.length === 10) break;
    }
    return versions;
  }

  function formatCodeArtifactPublished(value) {
    const date = parseIsoDateTime(value);
    if (!date) return "";
    return new Intl.DateTimeFormat("en-GB", {
      day: "2-digit",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(date);
  }

  function renderCodeArtifactVersionHistory(row, cell, providedVersions) {
    clear(cell);
    cell.removeAttribute("aria-busy");
    const versions = providedVersions || codeArtifactVersions(row);
    if (versions.length === 0) {
      cell.appendChild(el("div", { class: "muted small" }, "No published versions."));
      return;
    }
    const list = el("ol", { class: "codeartifact-version-list" },
      ...versions.map((item, index) => {
        const publishedLabel = formatCodeArtifactPublished(item.published);
        const published = publishedLabel
          ? el("time", {
            class: "codeartifact-version-published",
            datetime: item.published,
            title: item.published,
          }, `Published ${publishedLabel}`)
          : el("span", {
            class: "codeartifact-version-published codeartifact-version-date-missing",
            title: item.error || "Publish date unavailable",
          }, "Published date unavailable");
        return el("li", { class: "codeartifact-version-item" },
        el("span", { class: "codeartifact-version-rank muted" }, String(index + 1)),
          el("div", { class: "codeartifact-version-main" },
            el("code", { class: "codeartifact-version-value" }, item.version),
            published,
          ),
          codeArtifactCopyButton(item.version, `Copy version ${item.version}`),
        );
      }),
    );
    cell.appendChild(el("div", { class: "codeartifact-version-history" },
      el("div", { class: "codeartifact-version-history-head" },
        el("div", { class: "codeartifact-version-history-title" }, "Version history"),
        el("div", { class: "codeartifact-version-history-meta muted small" },
          `${versions.length} published · newest first`),
      ),
      list,
    ));
  }

  function renderCodeArtifactVersionError(row, cell, error, historyOptions) {
    if (cell._activeResultRequest) failResult(cell, cell._activeResultRequest, "Version history could not be loaded. Retry.");
    else clear(cell);
    cell.removeAttribute("aria-busy");
    const retry = el("button", { class: "exec-btn small", type: "button" }, "Retry");
    retry.addEventListener("click", (event) => {
      event.stopPropagation();
      loadCodeArtifactVersionHistory(row, cell, historyOptions);
    });
    cell.appendChild(el("div", { class: "codeartifact-version-history-error" },
      el("div", { class: "muted small" }, `Could not load version dates: ${error}`),
      retry,
    ));
  }

  async function loadCodeArtifactVersionHistory(row, cell, historyOptions) {
    if (!cell.isConnected) return;
    const localVersions = codeArtifactVersions(row);
    if (!isTauri || !historyOptions.form) {
      renderCodeArtifactVersionHistory(row, cell, localVersions);
      return;
    }
    const request = beginOwnedRequest(cell, historyOptions.context);
    if (!request.current()) return;
    clear(cell);
    cell.setAttribute("aria-busy", "true");
    cell.appendChild(el("div", { class: "muted small", role: "status" }, "Loading version dates…"));
    const sourceInputs = historyOptions.inputs || {};
    const detailInputs = {
      domain: sourceInputs.domain || "", repository: sourceInputs.repository || "",
      domain_owner: sourceInputs.domain_owner || "", package: String(row.package || ""),
      versions: localVersions.map(item => ({ version: item.version, published: item.published })),
    };
    beginResultRequest(request, "codeartifact-package-version-history", detailInputs);
    const cached = row._codeArtifactResultView;
    if (request.allowed && cached?.key === request.resultKey) {
      clear(cell);
      cell.removeAttribute("aria-busy");
      cell.append(...cached.nodes);
      cell._resultView = cached;
      cell._resultContext = cached.context;
      paintResultStatus(cell, cached.displayState || cached.state,
        cached.displaySpec || { coverage: cached.coverage }, cached, cached.displayMessage);
      return;
    }
    try {
      const result = await fetchWidgetData("codeartifact-package-version-history", detailInputs,
        cell, historyOptions.context, request);
      if (!request.accept(result)) return;
      cell.removeAttribute("aria-busy");
      if (result.render !== "codeartifact_version_history") {
        dispatchRender(cell, result);
        return;
      }
      renderWithResultState(cell, result, () => {
        const versions = Array.isArray(result.versions) ? result.versions : [];
        if (!versions.length) {
          clear(cell);
          cell.appendChild(el("div", { class: "muted small" }, resultFailure(result)
            ? "Version history is unavailable or incomplete." : "No version history returned."));
        } else renderCodeArtifactVersionHistory(row, cell, versions);
      });
      if (cell._resultView && validCoverage(result.coverage) && validResultShape(result) && !resultFailure(result)) {
        row._codeArtifactResultView = cell._resultView;
      }
    } catch (error) {
      if (request.current()) renderCodeArtifactVersionError(row, cell, String(error), historyOptions);
    }
  }

  function renderCodeArtifactPackagesTable(host, spec, historyOptions = {}) {
    renderTable(host, spec, {
      expand: (row, cell) => loadCodeArtifactVersionHistory(row, cell, historyOptions),
      renderCell: (column, row) => {
        if (column !== "latest_version") return null;
        const version = String(row.latest_version || "");
        if (!version) return el("span", {}, "");
        return el("span", { class: "codeartifact-latest-version" },
          el("span", { class: "codeartifact-latest-version-value", title: version }, version),
          codeArtifactCopyButton(version, `Copy latest version ${version}`),
        );
      },
    });
  }

  async function renderCodeArtifactPackages(target) {
    if (!isTauri) return renderCodeArtifactPackagesMock(target);

    return withWidgets("codeartifact-packages", target, (widget) => {
      const form = $(".codeartifact-packages-config", widget);
      const errorEl = $(".codeartifact-packages-error", widget);
      if (!form) return;
      const rows = $(".codeartifact-packages-rows", widget);
      wireRequestInputs(form, rows, errorEl);
      updateWidgetContextChip(widget.closest(".grid-stack-item"));

      if (!rows || rows.hidden) {
        applyCodeArtifactConfig(form);
      }

      if (form.dataset.wired !== "1") {
        form.dataset.wired = "1";
        form.addEventListener("keydown", (e) => {
          if (e.key !== "Enter" || !e.target.matches("input")) return;
          e.preventDefault();
          requestCodeArtifactLoad(form);
        });
        form.addEventListener("submit", async (e) => {
          e.preventDefault();
          const inputs = readCodeArtifactForm(form);
          if (!inputs.domain || !inputs.repository || !inputs.package_prefix) {
            errorEl.textContent = "Domain, repository, and package prefix are required.";
            return;
          }
          $(".codeartifact-max", form).value = String(inputs.max_packages);
          errorEl.textContent = "Loading...";
          clear(rows);
          const request = beginOwnedRequest(rows);
          try {
            const fetchInputs = { ...inputs };
            const fetchContext = contextPayloadForTile(form);
            const result = await fetchWidgetData(
              "codeartifact-packages",
              fetchInputs,
              form,
              fetchContext, request,
            );
            if (!request.accept(result)) return;
            if (result && result.error) errorEl.textContent = result.error;
            else errorEl.textContent = "";
            renderCodeArtifactPackagesTable(rows, result, {
              form,
              inputs: fetchInputs,
              context: fetchContext,
            });
            rows.hidden = false;
            persistCodeArtifactInputs(form, inputs);
          } catch (err) {
            if (!request.current()) return;
            errorEl.textContent = `Error: ${err}`;
            failResult(rows, request, String(err));
          }
        });
      }
    });
  }

  function renderCodeArtifactPackagesMock(target) {
    return withWidgets("codeartifact-packages", target, (widget) => {
      const rows = $(".codeartifact-packages-rows", widget);
      if (!rows) return;
      renderCodeArtifactPackagesTable(rows, {
        render: "table",
        columns: ["package", "latest_version", "last_published"],
        rows: Mock.codeArtifactPackages,
      });
      rows.hidden = false;
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
    });
  }

  // ===== Widget: Logs Insights Query =====
  // The user's own query, one log group, a relative time range. The backend
  // returns the closed `table` render shape (plus a stats footer).
  const LOGS_INSIGHTS_DEFAULT_QUERY =
    "fields @timestamp, @message | sort @timestamp desc | limit 20";
  const LOGS_INSIGHTS_RANGES = [
    ["15m", 900], ["1h", 3600], ["3h", 10800], ["12h", 43200],
    ["24h", 86400], ["3d", 259200], ["7d", 604800],
  ];

  function readLogsInsightsForm(form) {
    return {
      log_group: $(".li-group", form).value.trim(),
      query: $(".li-query", form).value.trim(),
      range_seconds: Number.parseInt($(".li-range", form).value, 10) || 3600,
    };
  }

  function applyLogsInsightsConfig(form) {
    const inputs = tileConfig(form).inputs || {};
    if (inputs.log_group) $(".li-group", form).value = inputs.log_group;
    if (inputs.query) $(".li-query", form).value = inputs.query;
    if (inputs.range_seconds) $(".li-range", form).value = String(inputs.range_seconds);
  }

  function formatInsightsStats(stats) {
    if (!stats) return "";
    const n = (x) => Number(x || 0).toLocaleString();
    return `${n(stats.records_matched)} matched · ${n(stats.records_scanned)} scanned`;
  }

  async function runLogsInsightsQuery(form, rows, errorEl) {
    const inputs = readLogsInsightsForm(form);
    if (!inputs.log_group || !inputs.query) {
      errorEl.textContent = "Log group and query are required.";
      return;
    }
    errorEl.textContent = "Running query…";
    clear(rows);
    const request = beginOwnedRequest(rows);
    try {
      const fetchContext = contextPayloadForTile(form);
      const result = await fetchWidgetData("logs-insights", { mode: "query", ...inputs }, rows, fetchContext, request);
      if (!request.accept(result)) return;
      if (!result) return;
      if (result.render) {
        errorEl.textContent = "";
        dispatchRender(rows, result);
        if (result.render === "table") {
          const zero = !resultFailure(result) && Array.isArray(result.rows) && result.rows.length === 0 ? "0 rows returned" : "";
          const stats = formatInsightsStats(result.stats);
          const footer = [zero, stats].filter(Boolean).join(" · ");
          if (footer) rows.appendChild(el("p", { class: "muted small" }, footer));
          persistTileInputs(form, inputs);
        }
        rows.hidden = false;
      } else {
        errorEl.textContent = result.error || "No data returned.";
        dispatchRender(rows, result);
      }
    } catch (err) {
      if (!request.current()) return;
      errorEl.textContent = `Error: ${err}`;
      failResult(rows, request, String(err));
    }
  }

  async function renderLogsInsights(target) {
    return withWidgets("logs-insights", target, (widget) => {
      const form = $(".logs-insights-config", widget);
      const errorEl = $(".logs-insights-error", widget);
      const rows = $(".logs-insights-rows", widget);
      if (!form || !rows) return;
      wireRequestInputs(form, rows, errorEl);
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      if (!isTauri) {
        errorEl.textContent = "Live data requires the desktop app.";
        return;
      }

      if (rows.hidden) applyLogsInsightsConfig(form);

      if (form.dataset.wired !== "1") {
        form.dataset.wired = "1";
        // The input owns its autocomplete cache and its pending completion.
        const groupInput = $(".li-group", form);
        groupInput.addEventListener("focus", async () => {
          if (groupInput.dataset.loaded === "1") return;
          const request = beginOwnedRequest(groupInput);
          groupInput.dataset.loaded = "1";
          const dl = $("datalist", form);
          try {
            const res = await fetchWidgetData("logs-insights", { mode: "groups", max_groups: 500 },
              groupInput, null, request);
            if (!request.accept(res)) return;
            renderArrayResult(request.resultHost, res, "groups", groups => {
              clear(request.resultHost);
              if (dl) {
                clear(dl);
                groups.forEach(g => dl.appendChild(el("option", { value: g.name })));
              }
              request.resultHost.appendChild(el("span", {}, `${groups.length} log group suggestions returned.`));
            });
            if (resultFailure(res)) delete groupInput.dataset.loaded;
          } catch (_) {
            if (request.current()) {
              delete groupInput.dataset.loaded;
              failResult(request.resultHost, request, "Log group suggestions could not be loaded. Focus to retry.");
            }
          }
        });
        form.addEventListener("submit", async (e) => {
          e.preventDefault();
          await runLogsInsightsQuery(form, rows, errorEl);
        });
      }

      // A dashboard tile with a saved query should show data, not a form.
      const saved = tileConfig(widget).inputs || {};
      if (rows.hidden && saved.log_group && saved.query) {
        form.dispatchEvent(new Event("submit", { cancelable: true }));
      }
    });
  }

  // ===== Widget: AWS CLI Table =====
  // The backend checks the exact mapping, arguments and policy, then gives the
  // constrained child the same temporary credentials verified for this context.
  // Commands can be pinned with the account/region they were saved under —
  // the same Live/Pinned tab and pin-card system as Pipeline Runs (and the
  // same pipeline-* CSS classes; cli-* classes are the JS hooks).
  const CLI_PINS_KEY = "pinned_cli_commands";

  // Session-only expand state, kept on the tile element (dies with the tile,
  // and two tiles pinning the identical command don't clobber each other).
  function cliPinExpandStateFor(widget) {
    const item = tileItemFor(widget);
    if (!item) return new Map();
    if (!item._cliPinExpand) item._cliPinExpand = new Map();
    return item._cliPinExpand;
  }

  // Display only. The backend owns the reviewed operation/argument registry.
  function describeCliCommand(command) {
    const m = command.trim().match(/^aws\s+([a-z0-9-]+)\s+([a-z0-9-]+)/);
    if (!m) return "";
    return `${m[1]} ${m[2]}`;
  }

  function cliPinKey(pin) {
    return [
      pin.profile || "",
      pin.account_id || "",
      pin.region || "",
      pin.command || "",
    ].join("|");
  }

  function normalizeCliPins(rawPins) {
    if (!Array.isArray(rawPins)) return [];
    const seen = new Set();
    const pins = [];
    rawPins.forEach((raw) => {
      if (!raw || typeof raw !== "object") return;
      const command = String(raw.command || "").trim();
      const profile = String(raw.profile || "").trim();
      const accountId = String(raw.account_id || raw.accountId || "").trim();
      const region = String(raw.region || "").trim();
      if (!command || !profile || !accountId || !region) return;
      const key = cliPinKey({ command, profile, account_id: accountId, region });
      if (seen.has(key)) return;
      seen.add(key);
      pins.push({ id: String(raw.id || key), command, profile, account_id: accountId, region });
    });
    return pins.slice(0, 50);
  }

  function cliPinsForWidget(widget) {
    const cfg = tileConfig(widget);
    return normalizeCliPins(cfg.inputs && cfg.inputs[CLI_PINS_KEY]);
  }

  function writeCliPins(widget, pins) {
    const item = tileItemFor(widget);
    if (!item) return [];
    const cfg = tileConfig(item);
    const clean = normalizeCliPins(pins);
    const inputs = { ...(cfg.inputs || {}) };
    if (clean.length) inputs[CLI_PINS_KEY] = clean;
    else delete inputs[CLI_PINS_KEY];
    writeTileConfig(item, { ...cfg, inputs });
    scheduleSaveLayout();
    return clean;
  }

  function cliPinContext(pin) {
    return {
      mode: "pinned",
      profile: pin.profile,
      account_id: pin.account_id,
      region: pin.region,
    };
  }

  function activeCliTab(widget) {
    const body = $(".aws-cli-body", widget);
    return (body && body.dataset.tab) || "live";
  }

  function setCliTab(widget, tab) {
    const body = $(".aws-cli-body", widget);
    if (!body) return;
    body.dataset.tab = tab;
    body.querySelectorAll(".cli-tab").forEach((btn) => {
      const active = btn.dataset.tab === tab;
      btn.classList.toggle("active", active);
      btn.setAttribute("aria-selected", String(active));
    });
    const live = $(".cli-pane-live", body);
    const pinned = $(".cli-pane-pinned", body);
    if (live) live.hidden = tab !== "live";
    if (pinned) pinned.hidden = tab !== "pinned";
  }

  function pinCurrentCliCommand(widget, form) {
    const errorEl = $(".aws-cli-error", widget);
    const command = $(".cli-command", form).value.trim();
    if (!command) {
      if (errorEl) errorEl.textContent = "Type a command first.";
      return;
    }
    if (!describeCliCommand(command)) {
      if (errorEl) errorEl.textContent = "Expected `aws <service> <operation> …`.";
      return;
    }
    const ctx = effectivePinnedContextForTile(form);
    if (!ctx) {
      if (errorEl) errorEl.textContent = "Pick an account and region first.";
      return;
    }
    const pin = {
      command,
      profile: ctx.profile,
      account_id: ctx.account_id,
      region: ctx.region,
    };
    pin.id = cliPinKey(pin);
    const pins = cliPinsForWidget(widget);
    if (!pins.some(p => cliPinKey(p) === pin.id)) {
      // writeCliPins normalizes (dedupe + 50 cap) — confirm the pin survived
      // before claiming success.
      const saved = writeCliPins(widget, [...pins, pin]);
      if (!saved.some(p => cliPinKey(p) === pin.id)) {
        if (errorEl) errorEl.textContent = "Pin limit (50) reached — remove a pin first.";
        return;
      }
    }
    renderCliPinList(widget);
    refreshCliPinByKey(widget, pin.id);
    if (errorEl) errorEl.textContent = "Pinned — see the Pinned tab.";
  }

  function renderCliPinList(widget) {
    const list = $(".cli-pin-list", widget);
    if (!list) return;
    const pins = cliPinsForWidget(widget);
    const expandState = cliPinExpandStateFor(widget);
    const emptyEl = $(".cli-pin-empty", widget);
    if (emptyEl) emptyEl.hidden = pins.length > 0;
    const countEl = $(".cli-pin-count", widget);
    if (countEl) {
      countEl.textContent = String(pins.length);
      countEl.hidden = pins.length === 0;
    }
    // Reconcile instead of rebuild: reusing a card node preserves its fetched
    // result, status badge, timestamp, and any in-flight run. Re-appending in
    // pin order both sorts and keeps state; stale cards are dropped.
    const existing = new Map(Array.from(list.children).map(c => [c.dataset.pinId, c]));
    existing.forEach((node, k) => {
      if (!pins.some(p => cliPinKey(p) === k)) node.remove();
    });
    // Reorders persist by reading the card order back out of the DOM, so a
    // drop only moves the node — fetched results and expansion state survive.
    const commitPinOrder = () => {
      const byKey = new Map(cliPinsForWidget(widget).map(p => [cliPinKey(p), p]));
      const order = Array.from(list.children).map(c => c.dataset.pinId);
      writeCliPins(widget, order.map(k => byKey.get(k)).filter(Boolean));
    };
    pins.forEach((pin) => {
      const key = cliPinKey(pin);
      pin.id = key;
      const reused = existing.get(key);
      if (reused) {
        list.appendChild(reused);
        return;
      }
      const handle = el("button", { class: "icon-btn pipeline-pin-handle", type: "button", title: "Drag to reorder" }, "≡");
      const toggle = el("button", { class: "icon-btn pipeline-pin-toggle", type: "button" }, "▸");
      const status = el("span", { class: "badge badge-neutral pipeline-pin-status" }, "Pinned");
      const updated = el("span", { class: "muted small pipeline-pin-updated" });
      const resultHost = el("div", { class: "pipeline-pin-result muted small" }, "Not run yet");
      const head = el("div", { class: "pipeline-pin-head" },
        handle,
        toggle,
        el("div", { class: "pipeline-pin-main" },
          el("div", { class: "pipeline-pin-name", title: pin.command }, pin.command),
          el("div", { class: "pipeline-pin-meta", title: pinMetaLabel(pin) }, envLightNodes(pinMetaLabel(pin), { wholeWord: true })),
        ),
        status,
        updated,
        el("button", { class: "icon-btn cli-pin-refresh", type: "button", title: "Run pinned command" }, "↻"),
        el("button", { class: "icon-btn cli-pin-remove", type: "button", title: "Remove pinned command" }, "✕"),
      );
      const card = el("section", { class: "pipeline-pin-card", "data-pin-id": key }, head, resultHost);
      const applyExpanded = (want) => {
        card.classList.toggle("expanded", want);
        resultHost.hidden = !want;
        toggle.textContent = want ? "▾" : "▸";
        toggle.title = want ? "Collapse pinned command" : "Expand pinned command";
        toggle.setAttribute("aria-expanded", String(want));
      };
      const setExpanded = (want) => {
        expandState.set(key, want);
        applyExpanded(want);
        // First expand of a card that has never run executes it on demand.
        if (want && card.dataset.loaded !== "1") refreshCliPinCard(widget, pin, card);
      };
      applyExpanded(expandState.get(key) === true);
      toggle.addEventListener("click", () => setExpanded(resultHost.hidden));
      head.addEventListener("click", (e) => {
        if (e.target.closest("button")) return;
        setExpanded(resultHost.hidden);
      });
      $(".cli-pin-refresh", card).addEventListener("click", () => refreshCliPinCard(widget, pin, card));
      $(".cli-pin-remove", card).addEventListener("click", () => {
        const kept = cliPinsForWidget(widget).filter(p => cliPinKey(p) !== key);
        writeCliPins(widget, kept);
        expandState.delete(key);
        renderCliPinList(widget);
      });
      // Document-level mouse reorder — same rationale as the pipeline pin
      // cards: HTML5 drag and pointer capture don't work in the Tauri webview.
      handle.addEventListener("mousedown", (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        card.classList.add("dragging");
        let moved = false;
        const onMove = (ev) => {
          ev.preventDefault();
          const over = document.elementFromPoint(ev.clientX, ev.clientY);
          const target = over && over.closest ? over.closest(".pipeline-pin-card") : null;
          if (!target || target === card || target.parentElement !== list) return;
          const r = target.getBoundingClientRect();
          list.insertBefore(card, ev.clientY < r.top + r.height / 2 ? target : target.nextSibling);
          moved = true;
        };
        const onUp = () => {
          document.removeEventListener("mousemove", onMove);
          card.classList.remove("dragging");
          if (moved) commitPinOrder();
        };
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp, { once: true });
      });
      list.appendChild(card);
    });
  }

  function refreshCliPinByKey(widget, key) {
    const pins = cliPinsForWidget(widget);
    const pin = pins.find(p => cliPinKey(p) === key);
    const card = pin && Array.from(widget.querySelectorAll(".cli-pin-list .pipeline-pin-card"))
      .find(item => item.dataset.pinId === key);
    if (pin && card) return refreshCliPinCard(widget, pin, card);
    return Promise.resolve();
  }

  async function refreshCliPinCard(widget, pin, card) {
    card.dataset.loaded = "1"; // marks an in-flight/completed run so expand doesn't rerun
    const statusEl = $(".pipeline-pin-status", card);
    const updatedEl = $(".pipeline-pin-updated", card);
    const resultHost = $(".pipeline-pin-result", card);
    const ctx = cliPinContext(pin);
    if (statusEl) {
      statusEl.className = "badge badge-progress pipeline-pin-status";
      statusEl.textContent = "Running";
    }
    if (updatedEl) updatedEl.textContent = "";
    if (resultHost) {
      resultHost.className = "pipeline-pin-result muted small";
      resultHost.textContent = "Running...";
      resultHost._widgetContextOverride = ctx;
    }
    const request = beginOwnedRequest(card, ctx);
    try {
      const result = await fetchWidgetData("aws-cli", { command: pin.command }, card, ctx, request);
      if (!request.accept(result)) return;
      if (updatedEl && (!resultFailure(result) || resultHasEvidence(result))) updatedEl.textContent = new Date().toLocaleTimeString();
      if (result && (!resultFailure(result) || resultHasEvidence(result)) && (result.render === "table" || result.render === "raw_json")) {
        if (statusEl) {
          const rowCount = result.render === "table" && Array.isArray(result.rows) ? result.rows.length : null;
          statusEl.className = "badge badge-success pipeline-pin-status";
          statusEl.textContent = requestFailureLabel(result) || (rowCount === null ? "OK" : `${rowCount} row${rowCount === 1 ? "" : "s"}`);
        }
        if (resultHost) {
          resultHost.className = "pipeline-pin-result";
          resultHost._widgetContextOverride = ctx;
          dispatchRender(resultHost, result);
        }
        return;
      }
      if (statusEl) {
        statusEl.className = "badge badge-error pipeline-pin-status";
        statusEl.textContent = requestFailureLabel(result) || "Error";
      }
      if (resultHost) {
        if (result && result.render) {
          resultHost.className = "pipeline-pin-result";
          resultHost._widgetContextOverride = ctx;
          dispatchRender(resultHost, result);
        } else {
          resultHost.className = "pipeline-pin-result muted small";
          dispatchRender(resultHost, result);
        }
      }
    } catch (err) {
      if (!request.current()) return;
      if (statusEl) {
        statusEl.className = "badge badge-error pipeline-pin-status";
        statusEl.textContent = "Error";
      }
      if (resultHost) {
        resultHost.className = "pipeline-pin-result muted small";
        failResult(resultHost, request, String(err));
      }
    }
  }

  function refreshCliPins(widget) {
    renderCliPinList(widget);
    const pins = cliPinsForWidget(widget);
    return Promise.all(pins.map(pin => refreshCliPinByKey(widget, cliPinKey(pin))));
  }

  async function runCliCommand(widget, form, rows, errorEl) {
    const cmdInput = $(".cli-command", form);
    const command = cmdInput.value.trim();
    if (!command) {
      errorEl.textContent = "Command is required.";
      return;
    }
    errorEl.textContent = "Running…";
    clear(rows);
    const request = beginOwnedRequest(rows);
    try {
      const fetchContext = contextPayloadForTile(form);
      const result = await fetchWidgetData("aws-cli", { command }, rows, fetchContext, request);
      if (!request.accept(result)) return;
      if (!result) return;
      if (result.render) {
        errorEl.textContent = "";
        dispatchRender(rows, result);
        if (!resultFailure(result) && result.render === "table" && Array.isArray(result.rows) && result.rows.length === 0) {
          rows.appendChild(el("p", { class: "muted small" }, "0 rows"));
        }
        rows.hidden = false;
        if (result.render !== "permission_denied") persistTileInputs(form, { command });
      } else {
        errorEl.textContent = result.error || "No data returned.";
        dispatchRender(rows, result);
      }
    } catch (err) {
      if (!request.current()) return;
      errorEl.textContent = `Error: ${err}`;
      failResult(rows, request, String(err));
    }
  }

  let cliAvailability = "unchecked";
  let cliAvailabilityPending = null;

  function updateCliAvailabilityUi(widget) {
    let notice = $(".cli-availability", widget);
    if (!notice) {
      notice = el("div", { class: "cli-availability small", role: "status" },
        el("span", { class: "cli-availability-message" }),
        el("button", { type: "button", class: "btn btn-ghost small cli-availability-retry",
          onclick: () => checkCliAvailability() }, "Retry CLI check"));
      $(".widget-body", widget).prepend(notice);
    }
    const messages = {
      unchecked: "Checking for an optional local AWS CLI…",
      checking: "Checking for an optional local AWS CLI…",
      available: "AWS CLI found locally. Its version has not been checked.",
      missing: "AWS CLI was not found. Install it separately for CLI widgets. SDK widgets remain available.",
      unknown: "AWS CLI availability could not be checked. SDK widgets remain available.",
    };
    notice.dataset.state = cliAvailability;
    $(".cli-availability-message", notice).textContent = messages[cliAvailability];
    $(".cli-availability-retry", notice).disabled = cliAvailability === "checking";
    widget.querySelectorAll(".cli-run-btn, .cli-pin-refresh").forEach(button => { button.disabled = cliAvailability !== "available"; });
  }

  async function checkCliAvailability() {
    if (cliAvailabilityPending) return cliAvailabilityPending;
    cliAvailability = "checking";
    widgetsOfType("aws-cli").forEach(updateCliAvailabilityUi);
    cliAvailabilityPending = (async () => {
      try {
        const result = await tauriInvoke("cli_availability");
        cliAvailability = result?.ok === true && result.status === "available" && result.available === true
          ? "available" : result?.ok === true && result.status === "missing" && result.available === false ? "missing" : "unknown";
      } catch (_) { cliAvailability = "unknown"; }
      widgetsOfType("aws-cli").forEach(updateCliAvailabilityUi);
      return cliAvailability === "available";
    })();
    try { return await cliAvailabilityPending; }
    finally { cliAvailabilityPending = null; }
  }

  async function renderAwsCli(target) {
    return withWidgets("aws-cli", target, (widget) => {
      const form = $(".aws-cli-config", widget);
      const errorEl = $(".aws-cli-error", widget);
      const rows = $(".aws-cli-rows", widget);
      if (!form || !rows) return;
      wireRequestInputs(form, rows, errorEl);
      updateWidgetContextChip(widget.closest(".grid-stack-item"));
      if (!isTauri) {
        errorEl.textContent = "Live data requires the desktop app.";
        return;
      }

      const cmdInput = $(".cli-command", form);
      const chip = $(".cli-action-chip", form);
      if (rows.hidden) {
        const inputs = tileConfig(widget).inputs || {};
        if (inputs.command) cmdInput.value = inputs.command;
      }
      const updateChip = () => {
        const command = describeCliCommand(cmdInput.value);
        chip.textContent = command
          ? `${command} — checked by the app before execution`
          : "";
      };
      updateChip();
      renderCliPinList(widget);
      updateCliAvailabilityUi(widget);
      if (cliAvailability === "unchecked") checkCliAvailability();

      if (form.dataset.wired === "1") {
        setCliTab(widget, activeCliTab(widget));
        return;
      }
      form.dataset.wired = "1";

      cmdInput.addEventListener("input", updateChip);
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        await runCliCommand(widget, form, rows, errorEl);
      });
      widget.querySelectorAll(".cli-tab").forEach((btn) => {
        btn.addEventListener("click", () => setCliTab(widget, btn.dataset.tab));
      });
      $(".cli-run-btn", widget)?.addEventListener("click", () => {
        setCliTab(widget, "live");
        form.dispatchEvent(new Event("submit", { cancelable: true }));
      });
      $(".cli-pin-btn", widget)?.addEventListener("click", () => pinCurrentCliCommand(widget, form));

      // A tile that already has pins opens on the Pinned tab; otherwise Live,
      // where a saved command auto-runs so the dashboard shows data.
      const pins = cliPinsForWidget(widget);
      setCliTab(widget, pins.length > 0 ? "pinned" : "live");
      const saved = tileConfig(widget).inputs || {};
      if (pins.length === 0 && rows.hidden && saved.command) {
        form.dispatchEvent(new Event("submit", { cancelable: true }));
      }
    });
  }

  // ===== Side panel: add widget =====
  const prebuiltWidgets = [
    { name: "Lambda Logs",             desc: "Browse Lambda functions, streams, and logs.",              gsId: "log-tail",                phase: 2 },
    { name: "CloudWatch Logs",         desc: "Search log groups, browse streams, view events.",          gsId: "cloudwatch-logs",         phase: 2 },
    { name: "Pipeline Runs",           desc: "Recent CodePipeline executions with status color.",        gsId: "pipeline-runs",           phase: 1 },
    { name: "CodeArtifact Packages",   desc: "Latest CodeArtifact package versions by prefix.",          gsId: "codeartifact-packages",   phase: 1 },
    { name: "CloudFormation Stacks",   desc: "Browse stacks, resources, and recent events.",              gsId: "cfn-stacks",              phase: 2 },
    { name: "Resource Reverse Lookup", desc: "Search resources and inspect confirmed stack associations.",                     gsId: "resource-lookup",         phase: 2 },
    { name: "Errors by Stack",         desc: "CloudWatch errors by stack over the last 24 hours.",       gsId: "errors-by-stack",         phase: 2 },
    { name: "Logs Insights Query",     desc: "Run your own Logs Insights query on any log group.",       gsId: "logs-insights",           phase: 2 },
    { name: "AWS CLI Table",           desc: "Supported AWS read commands rendered as a table.",       gsId: "aws-cli",                 phase: 2 },
  ];

  function setTileIdentity(tile, widgetType, tileId) {
    tile.setAttribute("gs-id", tileId);
    tile.setAttribute("data-widget-type", widgetType);
    const widget = tile.querySelector(".widget");
    if (widget) widget.dataset.widget = widgetType;
  }

  function highlightTile(tile) {
    const widget = tile.querySelector(".widget");
    if (!widget) return;
    widget.scrollIntoView({ behavior: "smooth", block: "center" });
    widget.style.transition = "box-shadow 0.6s";
    widget.style.boxShadow = "0 0 0 2px var(--accent)";
    setTimeout(() => { widget.style.boxShadow = ""; }, 900);
  }

  function buildPlaceholderTile(w) {
    // Gives a visible placeholder if a catalog entry has no dedicated live
    // renderer yet.
    const item = el("div", {
      class: "grid-stack-item",
      "gs-id": w.gsId,
      "data-widget-type": w.gsId,
      "gs-w": "4", "gs-h": "3", "gs-min-w": "3", "gs-min-h": "2",
    });
    const content = el("div", { class: "grid-stack-item-content" });
    const article = el("article", { class: "widget", "data-widget": w.gsId },
      el("header", { class: "widget-header" },
        el("h2", { class: "widget-title" }, w.name),
        el("span", { class: "widget-sub" }, "Runtime wiring pending"),
        el("div", { class: "widget-actions" },
          el("button", { class: "icon-btn", title: "Refresh" }, "↻"),
          el("button", { class: "icon-btn fs-btn", title: "Fullscreen" }, "⛶"),
          el("button", { class: "icon-btn cfg-btn", title: "Configure widget" }, "⚙"),
          el("button", { class: "icon-btn rm-btn", title: "Remove widget" }, "✕"),
        ),
      ),
      el("div", { class: "widget-body" },
        el("div", { class: "muted small" }, w.desc),
        el("div", { class: "muted small", style: "margin-top:8px;" },
          "This widget has no live data source yet. Runtime wiring is pending."),
      ),
    );
    content.appendChild(article);
    item.appendChild(content);
    return item;
  }

  function widgetActions(opts = {}) {
    const actions = [];
    if (opts.liveDot) actions.push(el("span", { class: "dot dot-live" }));
    if (opts.pause) actions.push(el("button", { class: "icon-btn", title: "Pause" }, "‖"));
    if (opts.refresh !== false) actions.push(el("button", { class: "icon-btn", title: "Refresh" }, "↻"));
    actions.push(el("button", { class: "icon-btn fs-btn", title: "Fullscreen" }, "⛶"));
    actions.push(el("button", { class: "icon-btn cfg-btn", title: "Configure widget" }, "⚙"));
    actions.push(el("button", { class: "icon-btn rm-btn", title: "Remove widget" }, "✕"));
    return el("div", { class: "widget-actions" }, actions);
  }

  function buildWidgetTile(w, body, opts = {}) {
    return el("div", {
      class: "grid-stack-item",
      "gs-id": w.gsId,
      "data-widget-type": w.gsId,
      "gs-w": String(opts.w || 4),
      "gs-h": String(opts.h || 3),
      "gs-min-w": String(opts.minW || 3),
      "gs-min-h": String(opts.minH || 2),
    },
      el("div", { class: "grid-stack-item-content" },
        el("article", { class: "widget", "data-widget": w.gsId },
          el("header", { class: "widget-header" },
            el("h2", { class: "widget-title" }, w.name),
            el("span", { class: "widget-sub" }, opts.sub || w.desc),
            widgetActions(opts),
          ),
          body,
        ),
      ),
    );
  }

  function buildLogTailTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body lambda-log-browser log-tail-body" }),
      { w: 8, h: 6, liveDot: true, pause: true, refresh: true, sub: "Lambda functions and CloudWatch logs" },
    );
  }

  function buildCloudwatchLogsTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body lambda-log-browser cw-logs-body" }),
      { w: 8, h: 6, refresh: true, sub: "Log groups, streams, and events" },
    );
  }

  function buildCfnStacksTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body cfn-stacks-body" }),
      { sub: "Stacks in this widget's region" },
    );
  }

  function buildResourceLookupTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body" },
        el("input", {
          class: "lookup-input",
          type: "text",
          placeholder: "Resource name, ARN, or partial id...",
        }),
        el("div", { class: "lookup-results" }),
      ),
      { refresh: false, sub: "Search resources; inspect confirmed stack associations" },
    );
  }

  function buildErrorsByStackTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body errors-body" }),
      { w: 8, sub: "CloudWatch errors in the last 24 hours" },
    );
  }

  function buildPipelineRunsTile() {
    return el("div", {
      class: "grid-stack-item",
      "gs-id": "pipeline-runs",
      "data-widget-type": "pipeline-runs",
      "gs-w": "4", "gs-h": "3", "gs-min-w": "3", "gs-min-h": "2",
    },
      el("div", { class: "grid-stack-item-content" },
        el("article", { class: "widget", "data-widget": "pipeline-runs" },
          el("header", { class: "widget-header" },
            el("h2", { class: "widget-title" }, "Pipeline Runs"),
            el("span", { class: "widget-sub" }, "Recent CodePipeline executions"),
            el("div", { class: "widget-actions" },
              el("button", { class: "icon-btn", title: "Refresh" }, "↻"),
              el("button", { class: "icon-btn fs-btn", title: "Fullscreen" }, "⛶"),
              el("button", { class: "icon-btn cfg-btn", title: "Configure widget" }, "⚙"),
              el("button", { class: "icon-btn rm-btn", title: "Remove widget" }, "✕"),
            ),
          ),
          el("div", { class: "widget-body pipeline-runs-body", "data-tab": "live" },
            el("div", { class: "pipeline-toolbar" },
              el("div", { class: "pipeline-tabs", role: "tablist" },
                el("button", { type: "button", class: "pipeline-tab active", "data-tab": "live", role: "tab", "aria-selected": "true" }, "Live"),
                el("button", { type: "button", class: "pipeline-tab", "data-tab": "pinned", role: "tab", "aria-selected": "false" },
                  "Pinned",
                  el("span", { class: "pipeline-pin-count", hidden: true }),
                ),
              ),
              el("div", { class: "pipeline-config-actions" },
                el("button", { type: "button", class: "btn btn-primary pipeline-load-btn" }, "Load runs"),
                el("button", { type: "button", class: "btn btn-ghost pipeline-pin-btn" }, "Pin"),
              ),
            ),
            el("div", { class: "pipeline-tab-pane pipeline-pane-live" },
              el("form", { class: "widget-config pipeline-config", "data-mode": "setup" },
                el("label", {},
                  el("span", {}, "Pipeline name"),
                  el("div", { class: "combo" },
                    el("input", {
                      class: "pipeline-name-search",
                      type: "text",
                      role: "combobox",
                      "aria-autocomplete": "list",
                      "aria-expanded": "false",
                      autocomplete: "off",
                      autocorrect: "off",
                      spellcheck: "false",
                      placeholder: "(pick an account to list pipelines)",
                      disabled: true,
                    }),
                  ),
                  el("select", { class: "pipeline-name-select", hidden: true, "aria-hidden": "true" }),
                  el("span", { class: "muted small pipeline-name-hint" }),
                ),
                el("p", { class: "muted small pipeline-error" }),
                el("p", { class: "muted small" }, "Profile, account, and region come from this widget's context."),
              ),
              el("div", { class: "widget-data pipeline-runs-rows", hidden: true }),
            ),
            el("div", { class: "pipeline-tab-pane pipeline-pane-pinned", hidden: true },
              el("div", { class: "pipeline-pin-panel" },
                el("div", { class: "pipeline-pin-list" }),
                el("p", { class: "muted small pipeline-pin-empty" }, "No pinned pipelines yet. Pick one on the Live tab and hit Pin."),
              ),
            ),
          ),
        ),
      ),
    );
  }

  function buildCodeArtifactPackagesTile() {
    return el("div", {
      class: "grid-stack-item",
      "gs-id": "codeartifact-packages",
      "data-widget-type": "codeartifact-packages",
      "gs-w": "6", "gs-h": "4", "gs-min-w": "4", "gs-min-h": "3",
    },
      el("div", { class: "grid-stack-item-content" },
        el("article", { class: "widget", "data-widget": "codeartifact-packages" },
          el("header", { class: "widget-header" },
            el("h2", { class: "widget-title" }, "CodeArtifact Packages"),
            el("span", { class: "widget-sub" }, "Latest package versions by prefix"),
            el("div", { class: "widget-actions" },
              el("button", {
                class: "icon-btn",
                title: "Refresh",
                "aria-label": "Load or refresh packages",
              }, "↻"),
              el("button", { class: "icon-btn fs-btn", title: "Fullscreen" }, "⛶"),
              el("button", { class: "icon-btn cfg-btn", title: "Configure widget" }, "⚙"),
              el("button", { class: "icon-btn rm-btn", title: "Remove widget" }, "✕"),
            ),
          ),
          el("div", { class: "widget-body codeartifact-packages-body" },
            el("form", { class: "widget-config codeartifact-packages-config", "data-mode": "setup" },
              el("label", {},
                el("span", {}, "Domain"),
                el("input", {
                  class: "codeartifact-domain",
                  type: "text",
                  value: CODEARTIFACT_DEFAULTS.domain,
                  autocomplete: "off",
                  autocorrect: "off",
                  spellcheck: "false",
                }),
              ),
              el("label", {},
                el("span", {}, "Repository"),
                el("input", {
                  class: "codeartifact-repository",
                  type: "text",
                  value: CODEARTIFACT_DEFAULTS.repository,
                  autocomplete: "off",
                  autocorrect: "off",
                  spellcheck: "false",
                }),
              ),
              el("label", {},
                el("span", {}, "Package prefix"),
                el("input", {
                  class: "codeartifact-prefix",
                  type: "text",
                  value: CODEARTIFACT_DEFAULTS.package_prefix,
                  autocomplete: "off",
                  autocorrect: "off",
                  spellcheck: "false",
                }),
              ),
              el("label", {},
                el("span", {}, "Max packages"),
                el("input", {
                  class: "codeartifact-max",
                  type: "number",
                  min: "1",
                  max: "1000",
                  value: String(CODEARTIFACT_DEFAULTS.max_packages),
                }),
              ),
              el("p", { class: "muted small codeartifact-packages-error" }),
            ),
            el("div", { class: "widget-data codeartifact-packages-rows", hidden: true }),
          ),
        ),
      ),
    );
  }

  function buildLogsInsightsTile(w) {
    const dlId = `li-groups-${Math.random().toString(36).slice(2, 8)}`;
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body logs-insights-body" },
        el("form", { class: "widget-config logs-insights-config", "data-mode": "setup" },
          el("label", {},
            el("span", {}, "Log group"),
            el("input", {
              class: "li-group",
              type: "text",
              list: dlId,
              placeholder: "/aws/lambda/my-fn",
              autocomplete: "off",
              autocorrect: "off",
              spellcheck: "false",
            }),
          ),
          el("datalist", { id: dlId }),
          el("label", {},
            el("span", {}, "Query"),
            el("textarea", { class: "li-query", rows: "3", spellcheck: "false" },
              LOGS_INSIGHTS_DEFAULT_QUERY),
          ),
          el("label", {},
            el("span", {}, "Time range"),
            el("select", { class: "li-range" },
              LOGS_INSIGHTS_RANGES.map(([label, secs]) =>
                el("option", { value: String(secs), selected: secs === 3600 ? "selected" : undefined }, label)),
            ),
          ),
          el("button", { type: "submit", class: "btn btn-primary" }, "Run query"),
          el("p", { class: "muted small logs-insights-error" }),
        ),
        el("div", { class: "widget-data logs-insights-rows", hidden: true }),
      ),
      { w: 8, h: 6, refresh: true, sub: "Your own Logs Insights query" },
    );
  }

  function buildAwsCliTile(w) {
    return buildWidgetTile(
      w,
      el("div", { class: "widget-body aws-cli-body", "data-tab": "live" },
        el("div", { class: "pipeline-toolbar" },
          el("div", { class: "pipeline-tabs", role: "tablist" },
            el("button", { type: "button", class: "pipeline-tab cli-tab active", "data-tab": "live", role: "tab", "aria-selected": "true" }, "Live"),
            el("button", { type: "button", class: "pipeline-tab cli-tab", "data-tab": "pinned", role: "tab", "aria-selected": "false" },
              "Pinned",
              el("span", { class: "pipeline-pin-count cli-pin-count", hidden: true }),
            ),
          ),
          el("div", { class: "pipeline-config-actions" },
            el("button", { type: "button", class: "btn btn-primary cli-run-btn" }, "Run"),
            el("button", { type: "button", class: "btn btn-ghost cli-pin-btn" }, "Pin"),
          ),
        ),
        el("div", { class: "pipeline-tab-pane cli-pane-live" },
          el("form", { class: "widget-config aws-cli-config", "data-mode": "setup" },
            el("label", {},
              el("span", {}, "Command"),
              el("input", {
                class: "cli-command",
                type: "text",
                placeholder: "aws sts get-caller-identity",
                autocomplete: "off",
                autocorrect: "off",
                spellcheck: "false",
              }),
            ),
            el("p", { class: "muted small cli-action-chip" }),
            el("p", { class: "muted small aws-cli-error" }),
          ),
          el("div", { class: "widget-data aws-cli-rows", hidden: true }),
        ),
        el("div", { class: "pipeline-tab-pane cli-pane-pinned", hidden: true },
          el("div", { class: "pipeline-pin-panel" },
            el("div", { class: "pipeline-pin-list cli-pin-list" }),
            el("p", { class: "muted small pipeline-pin-empty cli-pin-empty" }, "No pinned commands yet. Type one on the Live tab and hit Pin."),
          ),
        ),
      ),
      { w: 8, h: 5, refresh: true, sub: "Read-only aws command as a table" },
    );
  }

  function buildTileForWidget(w) {
    if (w.gsId === "pipeline-runs") return buildPipelineRunsTile();
    if (w.gsId === "codeartifact-packages") return buildCodeArtifactPackagesTile();
    if (w.gsId === "log-tail") return buildLogTailTile(w);
    if (w.gsId === "cloudwatch-logs") return buildCloudwatchLogsTile(w);
    if (w.gsId === "cfn-stacks") return buildCfnStacksTile(w);
    if (w.gsId === "resource-lookup") return buildResourceLookupTile(w);
    if (w.gsId === "errors-by-stack") return buildErrorsByStackTile(w);
    if (w.gsId === "logs-insights") return buildLogsInsightsTile(w);
    if (w.gsId === "aws-cli") return buildAwsCliTile(w);
    return buildPlaceholderTile(w);
  }

  function addWidgetToGrid(w) {
    if (!grid) {
      console.warn("GridStack not initialised — cannot add widget");
      return;
    }
    const tile = buildTileForWidget(w);
    setTileIdentity(tile, w.gsId, makeTileId(w.gsId));
    grid.el.appendChild(tile);
    grid.makeWidget(tile);
    // Re-wire interactions for the new tile.
    wireFullscreenButtons();
    wireRefreshButtons();
    wireRemoveButtons();
    wireConfigButtons();
    updateWidgetContextChip(tile);
    if (w.gsId === "pipeline-runs") {
      renderPipelineRuns(tile);
      if (isTauri) loadPipelineList(tile);
    }
    if (w.gsId === "codeartifact-packages") {
      renderCodeArtifactPackages(tile);
    }
    if (w.gsId === "log-tail") {
      renderLogTail(tile);
    }
    if (w.gsId === "cloudwatch-logs") {
      renderCloudwatchLogs(tile);
    }
    if (w.gsId === "cfn-stacks") {
      renderCfnStacks(tile);
    }
    if (w.gsId === "resource-lookup") {
      renderLookup(tile);
    }
    if (w.gsId === "errors-by-stack") {
      renderErrors(tile);
    }
    if (w.gsId === "logs-insights") {
      renderLogsInsights(tile);
    }
    if (w.gsId === "aws-cli") {
      renderAwsCli(tile);
    }
    highlightTile(tile);
    closeSidePanel();
    renderPrebuiltList();
    scheduleSaveLayout();
  }

  function renderPrebuiltList() {
    const root = $("#prebuilt-list");
    clear(root);
    prebuiltWidgets.forEach(w => {
      root.appendChild(
        el("div", { class: "prebuilt-item" },
          el("div", {},
            el("div", { class: "prebuilt-name" }, w.name),
            el("div", { class: "prebuilt-desc" }, w.desc),
          ),
          el("button", {
            class: "prebuilt-add",
            onclick: () => addWidgetToGrid(w),
          }, "Add"),
        )
      );
    });
  }

  function openSidePanel() {
    $("#side-panel").classList.add("open");
    $("#side-panel").setAttribute("aria-hidden", "false");
    $("#scrim").classList.add("open");
    $("#scrim").hidden = false;
  }
  function closeSidePanel() {
    $("#side-panel").classList.remove("open");
    $("#side-panel").setAttribute("aria-hidden", "true");
    $("#scrim").classList.remove("open");
    setTimeout(() => { $("#scrim").hidden = true; }, 220);
  }
  $("#add-widget-btn").addEventListener("click", openSidePanel);
  $("#side-panel-close").addEventListener("click", closeSidePanel);
  $("#scrim").addEventListener("click", () => {
    closeSidePanel();
    closeSettingsPanel();
    closeAuditPanel();
    closeIdentityPanel();
    closeWidgetConfigPanel();
  });

  // Settings & audit panels — only meaningful when Tauri is present.
  if (isTauri) {
    const gear = $("#settings-btn");
    if (gear) {
      gear.hidden = false;
      gear.addEventListener("click", openSettingsPanel);
    }
    $("#settings-panel-close").addEventListener("click", closeSettingsPanel);
    $("#settings-cancel").addEventListener("click", closeSettingsPanel);
    $("#settings-form").addEventListener("submit", saveSettings);
    $("#settings-form").addEventListener("input", markSettingsDirty);
    $("#policy-save")?.addEventListener("click", savePolicy);
    $("#policy-reload")?.addEventListener("click", loadPolicy);
    // input re-renders content and re-syncs scroll; scroll only re-syncs.
    $("#policy-editor")?.addEventListener("input", () => renderPolicyHighlight(true));
    $("#policy-editor")?.addEventListener("scroll", syncPolicyScroll);

    const auditBtn = $("#audit-btn");
    if (auditBtn) {
      auditBtn.hidden = false;
      auditBtn.addEventListener("click", openAuditPanel);
    }
    $("#identity-panel-close")?.addEventListener("click", closeIdentityPanel);
    $("#audit-panel-close").addEventListener("click", closeAuditPanel);
  }

  // ===== Widget-config panel wiring (works in browser mode too, Save no-ops
  // the dashboard.json persistence path but still updates the live tile).
  $("#widget-config-panel-close")?.addEventListener("click", closeWidgetConfigPanel);
  $("#cfg-cancel")?.addEventListener("click", closeWidgetConfigPanel);
  $("#cfg-save")?.addEventListener("click", saveWidgetConfig);
  $("#cfg-reset")?.addEventListener("click", resetWidgetConfig);
  $("#cfg-use-override")?.addEventListener("change", refreshOverrideFieldsVisibility);
  $("#cfg-source-details")?.addEventListener("toggle", (e) => {
    if (!e.target.open || !currentCfgTile) return;
    const widget = currentCfgTile.querySelector(".widget");
    const name = widget && widget.dataset.widget;
    if (!name) return;
    if (cfgSourceLoadedFor === name) return;  // already loaded
    loadSourceIntoPanel(name);
  });

  // ===== Mock AI generation =====
  $("#ai-generate").addEventListener("click", () => {
    const prompt = $("#ai-prompt").value.trim();
    const out = $("#ai-output");
    if (!prompt) { out.hidden = true; return; }
    out.hidden = false;
    const sample = generateWidgetCode(prompt);
    out.textContent = "// Generating widget…\n";
    let i = 0;
    const tick = () => {
      out.textContent = "// Generating widget…\n" + sample.slice(0, i);
      i += 18;
      if (i < sample.length) setTimeout(tick, 18);
      else out.textContent = sample;
    };
    tick();
  });
  $("#ai-cancel").addEventListener("click", () => {
    $("#ai-prompt").value = "";
    $("#ai-output").hidden = true;
    $("#ai-output").textContent = "";
  });

  function generateWidgetCode(prompt) {
    return `# widget.yaml
name: "Lambdas with errors, grouped by stack"
version: 1
inputs:
  hours: { type: number, default: 1 }
refresh: 60s
permissions:
  - cloudwatch:read
  - cloudformation:read

# widget.py
def fetch(ctx):
    rows = ctx.steampipe.query("""
      with errored as (
        select log_group_name, count(*) as errors
        from aws_cloudwatch_log_event
        where region = $1
          and timestamp > now() - interval '$2 hours'
          and message ilike '%error%'
        group by log_group_name
      )
      select e.log_group_name as lambda, e.errors,
             r.stack_name as stack
      from errored e
      left join aws_cloudformation_stack_resource r
        on r.physical_resource_id = replace(e.log_group_name, '/aws/lambda/', '')
      order by e.errors desc
      limit 25
    """, [ctx.region, ctx.input("hours")])

    return {
      "render": "table",
      "columns": ["lambda", "errors", "stack"],
      "rows": rows,
      "row_actions": [
        {"label": "Tail",       "spawn_widget": "log-tail",
         "inputs": {"log_group": "{lambda}"}},
        {"label": "Open stack", "spawn_widget": "cfn-stack-detail",
         "inputs": {"stack_name": "{stack}"}},
      ],
    }

# (you wrote: ${truncate(prompt, 90)})`;
  }

  function truncate(s, n) { return s.length > n ? s.slice(0, n - 1) + "…" : s; }

  // ===== Spawn widget (from "Tail this lambda" buttons) =====
  function spawnLogTailWidget(logGroup) {
    const tile = $(".log-tail-body")?.closest(".widget");
    if (!tile) return;
    const sub = tile.querySelector(".widget-sub");
    if (sub) sub.textContent = logGroup;
    renderLogTail(tile);
    tile.style.transition = "box-shadow 0.6s";
    tile.style.boxShadow = "0 0 0 2px var(--accent)";
    setTimeout(() => { tile.style.boxShadow = ""; }, 700);
    tile.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }

  // ===== GridStack: drag, resize, persist =====
  // Tauri mode persists layout via the backend (dashboard_get / dashboard_set,
  // writes to ~/.cloud_burrito/dashboard.json). Browser mode falls back
  // to localStorage so the offline demo still remembers a layout.
  const LAYOUT_KEY = "acc.layout.v1";
  const LAYOUT_SAVE_DEBOUNCE_MS = 400;
  let grid = null;
  let layoutSaveTimer = null;
  let dashboardStorageReady = !isTauri;
  let restoringLayout = false;
  let layoutSavePending = false;
  let layoutSaveQueued = false;
  let layoutEditRevision = 0;
  let layoutLoadId = 0;
  let starterLayout = [];

  function catalogWidget(type) {
    return prebuiltWidgets.find(w => w.gsId === type) || {
      name: widgetDisplayName(type),
      desc: "Custom widget",
      gsId: type,
    };
  }

  function widgetTypeForSavedTile(tile) {
    return (tile && (tile.widget || tile.type)) || String(tile?.id || "").split(":")[0];
  }

  function renderWidgetTile(tile) {
    const widget = tile && tile.querySelector && tile.querySelector(".widget");
    if (!widget) return;
    const type = widget.dataset.widget;
    if (type === "pipeline-runs") {
      renderPipelineRuns(widget);
      if (isTauri) loadPipelineList(widget);
    } else if (type === "codeartifact-packages") {
      renderCodeArtifactPackages(widget);
    } else if (type === "log-tail") {
      renderLogTail(widget);
    } else if (type === "cloudwatch-logs") {
      renderCloudwatchLogs(widget);
    } else if (type === "cfn-stacks") {
      renderCfnStacks(widget);
    } else if (type === "resource-lookup") {
      renderLookup(widget);
    } else if (type === "errors-by-stack") {
      renderErrors(widget);
    } else if (type === "logs-insights") {
      renderLogsInsights(widget);
    } else if (type === "aws-cli") {
      renderAwsCli(widget);
    }
    updateWidgetContextChip(tile);
  }

  function ensureSavedTileElement(savedTile) {
    if (!savedTile || !savedTile.id || tileById(String(savedTile.id))) return;
    const type = widgetTypeForSavedTile(savedTile);
    const tile = buildTileForWidget(catalogWidget(type));
    setTileIdentity(tile, type, String(savedTile.id));
    grid.el.appendChild(tile);
    grid.makeWidget(tile);
    wireFullscreenButtons();
    wireRefreshButtons();
    wireRemoveButtons();
    wireConfigButtons();
    // Stamp the saved config BEFORE the first render, so one-time wiring
    // (initial tab choice, saved-input auto-run, pin lists) sees the real
    // inputs instead of an empty config. The stamp loop in initGridStack
    // re-stamps the same values afterwards — harmless.
    if (savedTile.config) writeTileConfig(tile, normalizeConfig(savedTile.config));
    renderWidgetTile(tile);
  }

  function pickTileFields(t) {
    // The backend persists position/size plus an opaque `config` dict per
    // tile (per-widget context, header color, inputs). Everything
    // else from `grid.save(false)` is dropped.
    const item = t.id != null ? tileById(String(t.id)) : null;
    const out = {
      id: t.id != null ? String(t.id) : "",
      widget: item ? widgetTypeForItem(item) : "",
      x: Number(t.x) || 0,
      y: Number(t.y) || 0,
      w: Number(t.w) || 1,
      h: Number(t.h) || 1,
    };
    if (out.id && item) {
      if (item) {
        const cfg = tileConfig(item);
        // Keep the round-trip compact: only persist non-default fields.
        const persisted = {};
        if (cfg.context && cfg.context.mode === "pinned") persisted.context = cfg.context;
        if (cfg.header_color)     persisted.header_color = cfg.header_color;
        if (cfg.inputs && Object.keys(cfg.inputs).length > 0) persisted.inputs = cfg.inputs;
        if (Object.keys(persisted).length > 0) out.config = persisted;
      }
    }
    return out;
  }

  async function initGridStack() {
    if (typeof GridStack === "undefined") {
      console.warn("GridStack failed to load (offline?). Drag/resize disabled.");
      return;
    }
    grid = GridStack.init({
      column: 12,
      cellHeight: 90,
      margin: 12,
      handle: ".widget-header",
      // Buttons, inputs, the picker etc. should not initiate drag.
      cancel: ".icon-btn, button, input, textarea, select, .picker, .lookup-input, .widget-actions",
      resizable: { handles: "e, se, s" },
      float: false,
      animate: true,
      disableOneColumnMode: false,
    }, "#grid-stack");
    starterLayout = grid.save(false).map(pickTileFields).filter(tile => tile.id);

    // Restore saved layout if present. In Tauri mode the source of truth is
    // the backend dashboard file; the localStorage cache is only used in
    // browser mode (no Tauri shell).
    const saved = await loadLayout();
    if (Array.isArray(saved)) restoreLayout(saved);

    // Persist only after a valid/missing file has been established. A failed
    // read must never turn the preview/default layout into a replacement file.
    grid.on("change added removed resizestop dragstop", () => scheduleSaveLayout());
  }

  function restoreLayout(saved) {
    if (!grid) return;
    restoringLayout = true;
    try {
      saved.forEach(ensureSavedTileElement);
      grid.load(saved);
      // GridStack only restores position/size; per-tile config is opaque to
      // it. Walk the saved entries and stamp config back onto each tile.
      saved.forEach(t => {
        if (!t || !t.id || !t.config) return;
        const item = tileById(String(t.id));
        if (item) {
          const merged = normalizeConfig(t.config);
          writeTileConfig(item, merged);
          applyTileHeaderColor(item);
          updateWidgetContextChip(item);
        }
      });
      document.querySelectorAll(".grid-stack-item").forEach(renderWidgetTile);
    } catch (_) {
      dashboardStorageReady = false;
      showStorageLoadWarning("dashboard", retryLayoutLoad, resetLayout);
      console.warn("Could not restore layout.");
    } finally {
      restoringLayout = false;
    }
  }

  function scheduleSaveLayout() {
    if (!grid || restoringLayout || (isTauri && !dashboardStorageReady)) return;
    layoutEditRevision++;
    setLayoutStatus("Unsaved layout changes.");
    if (layoutSaveTimer) clearTimeout(layoutSaveTimer);
    layoutSaveTimer = setTimeout(saveLayout, LAYOUT_SAVE_DEBOUNCE_MS);
  }

  function setLayoutSavePending(pending) {
    layoutSavePending = pending;
    $("#reset-layout-btn").disabled = pending;
  }

  function setLayoutStatus(message) {
    if (!isTauri) return;
    const status = $("#layout-save-status");
    status.textContent = message;
    status.hidden = !message;
  }

  async function saveLayout() {
    if (!grid || restoringLayout || (isTauri && !dashboardStorageReady)) return;
    if (layoutSavePending) { layoutSaveQueued = true; return; }
    let data;
    try {
      data = grid.save(false); // false = positions only, no inner HTML
    } catch (_) { return; }
    const tiles = (data || []).map(pickTileFields).filter(t => t.id);
    if (isTauri) {
      const revision = layoutEditRevision;
      setLayoutSavePending(true);
      setLayoutStatus("Saving layout…");
      try {
        await tauriInvoke("dashboard_set", { params: { tiles } });
        $("#layout-save-warning")?.remove();
        setLayoutStatus(revision === layoutEditRevision ? "Layout saved." : "Unsaved layout changes.");
      } catch (e) {
        console.warn("Dashboard could not be saved.");
        setLayoutStatus("Layout save failed. Current changes are not saved.");
        showLayoutSaveWarning("Dashboard changes were not saved. The current layout is still displayed.");
      } finally {
        setLayoutSavePending(false);
        if (layoutSaveQueued) {
          layoutSaveQueued = false;
          if (dashboardStorageReady) saveLayout();
        }
      }
      return;
    }
    // Browser-mode cache only.
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify(tiles));
    } catch (_) {}
  }

  async function loadLayout() {
    if (isTauri) {
      const request = ++layoutLoadId;
      dashboardStorageReady = false;
      layoutSaveQueued = false;
      if (layoutSaveTimer) clearTimeout(layoutSaveTimer);
      try {
        const resp = await tauriInvoke("dashboard_get");
        if (request !== layoutLoadId) return undefined;
        if (!resp || resp.ok === false || resp._storage?.status === "failed" || !Array.isArray(resp.tiles)) {
          throw new Error("Dashboard unavailable");
        }
        dashboardStorageReady = true;
        $("#dashboard-storage-warning")?.remove();
        return resp._storage?.status === "missing" ? null : resp.tiles;
      } catch (e) {
        if (request !== layoutLoadId) return undefined;
        dashboardStorageReady = false;
        showStorageLoadWarning("dashboard", retryLayoutLoad, resetLayout);
        console.warn("Dashboard could not be loaded.");
        return undefined;
      }
    }
    try {
      const raw = localStorage.getItem(LAYOUT_KEY);
      return raw ? JSON.parse(raw) : null;
    } catch (_) {
      return null;
    }
  }

  function showLayoutSaveWarning(message, retry = saveLayout) {
    let warning = $("#layout-save-warning");
    if (!warning) {
      warning = el("div", { id: "layout-save-warning", class: "layout-save-warning", role: "status" },
        el("span", {}), el("button", { class: "btn btn-ghost small", type: "button" }));
      $(".dashboard").before(warning);
    }
    warning.querySelector("span").textContent = message;
    const button = warning.querySelector("button");
    button.textContent = retry === resetLayout ? "Retry reset" : "Retry save";
    button.onclick = retry;
  }

  async function retryLayoutLoad() {
    if (layoutSavePending) return;
    const saved = await loadLayout();
    if (saved !== undefined) restoreLayout(saved === null ? starterLayout : saved);
  }

  async function resetLayout() {
    if (isTauri) {
      if (layoutSavePending) return;
      if (layoutSaveTimer) clearTimeout(layoutSaveTimer);
      layoutSaveQueued = false;
      ++layoutLoadId;
      const wasReady = dashboardStorageReady;
      dashboardStorageReady = false;
      setLayoutSavePending(true);
      setLayoutStatus("Saving layout reset…");
      try {
        await tauriInvoke("dashboard_set", { params: { tiles: starterLayout } });
      } catch (e) {
        console.warn("Dashboard reset could not be saved.");
        setLayoutStatus("Layout reset failed.");
        showLayoutSaveWarning("Layout reset was not saved. The current layout is unchanged.", resetLayout);
        return;
      } finally {
        dashboardStorageReady = wasReady;
        layoutSaveQueued = false;
        setLayoutSavePending(false);
      }
      location.reload();
      return;
    }
    try { localStorage.removeItem(LAYOUT_KEY); } catch (e) {}
    location.reload();
  }
  $("#reset-layout-btn").addEventListener("click", resetLayout);

  // ===== Fullscreen widget toggle (Jira-style) =====
  let fullscreenWidget = null;

  function enterFullscreen(widget) {
    if (fullscreenWidget) exitFullscreen();
    widget.classList.add("fullscreen");
    document.body.classList.add("has-fullscreen-widget");
    fullscreenWidget = widget;
    // Disable grid drag/resize while a widget is fullscreen.
    if (grid) grid.disable();
  }
  function exitFullscreen() {
    if (!fullscreenWidget) return;
    fullscreenWidget.classList.remove("fullscreen");
    document.body.classList.remove("has-fullscreen-widget");
    fullscreenWidget = null;
    if (grid) grid.enable();
  }

  function wireFullscreenButtons() {
    document.querySelectorAll(".fs-btn").forEach((btn) => {
      if (btn.dataset.wired === "1") return;
      btn.dataset.wired = "1";
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const widget = btn.closest(".widget");
        if (!widget) return;
        if (widget.classList.contains("fullscreen")) exitFullscreen();
        else enterFullscreen(widget);
      });
    });
  }

  // Per-widget refresh: maps each widget's data-widget id to its render fn.
  function refreshHandlers() {
    return {
      "pipeline-runs": (widget) => {
        // Refresh follows the active tab: Pinned refetches the pin cards,
        // Live refetches the currently loaded pipeline (if any).
        if (activePipelineTab(widget) === "pinned") {
          refreshPipelinePins(widget);
          return;
        }
        // If real data is loaded, re-submit the existing config to refetch;
        // otherwise just re-render the inline form (mock or initial state).
        const rows = $(".pipeline-runs-rows", widget);
        const form = $(".pipeline-config", widget);
        if (isTauri && rows && !rows.hidden && form) {
          form.dispatchEvent(new Event("submit", { cancelable: true }));
        } else {
          renderPipelineRuns(widget);
        }
      },
      "codeartifact-packages": (widget, options = {}) => {
        const rows = $(".codeartifact-packages-rows", widget);
        const form = $(".codeartifact-packages-config", widget);
        const explicitlyRefreshed = options.explicit === true;
        if (isTauri && form && (explicitlyRefreshed || (rows && (!rows.hidden || (options.afterSelection && rows._reloadAfterSelection))))) {
          requestCodeArtifactLoad(form);
        } else {
          renderCodeArtifactPackages(widget);
        }
      },
      "logs-insights": (widget, options = {}) => {
        const rows = $(".logs-insights-rows", widget);
        const form = $(".logs-insights-config", widget);
        if (isTauri && rows && form && (!rows.hidden || (options.afterSelection && rows._reloadAfterSelection))) {
          form.dispatchEvent(new Event("submit", { cancelable: true }));
        } else {
          renderLogsInsights(widget);
        }
      },
      "aws-cli": (widget, options = {}) => {
        // Refresh follows the active tab: Pinned reruns every pin card under
        // its own saved context, Live reruns the current command (if loaded).
        if (activeCliTab(widget) === "pinned") {
          refreshCliPins(widget);
          return;
        }
        const rows = $(".aws-cli-rows", widget);
        const form = $(".aws-cli-config", widget);
        const savedCommand = tileConfig(widget).inputs?.command;
        if (isTauri && rows && form && (!rows.hidden
            || (options.afterSelection && (rows._reloadAfterSelection || savedCommand)))) {
          form.dispatchEvent(new Event("submit", { cancelable: true }));
        } else {
          renderAwsCli(widget);
        }
      },
      "log-tail":        (widget) => renderLogTail(widget),
      "cloudwatch-logs": (widget) => renderCloudwatchLogs(widget),
      "cfn-stacks":      (widget) => renderCfnStacks(widget),
      "resource-lookup": (widget) => renderLookup(widget),
      "errors-by-stack": (widget) => renderErrors(widget),
    };
  }

  // After an account/region switch, reload every widget so it shows data for
  // the new default context — except tiles pinned to their own account/region.
  function refreshAllUnpinnedWidgets() {
    if (!isTauri) return;
    const handlers = refreshHandlers();
    document.querySelectorAll(".widget").forEach((widget) => {
      const id = widget.dataset.widget;
      const fn = handlers[id];
      if (!fn) return;
      const tile = widget.closest(".grid-stack-item");
      const cfg = tile ? tileConfig(tile) : emptyConfig();
      if (cfg && cfg.context && cfg.context.mode === "pinned") return;
      // Saved pin cards carry their own account/region context and stay mounted.
      if (id === "pipeline-runs" && activePipelineTab(widget) === "pinned") {
        return;
      }
      // Same for pinned-tab AWS CLI tiles.
      if (id === "aws-cli" && activeCliTab(widget) === "pinned") {
        return;
      }
      fn(widget, { afterSelection: true });
    });
  }

  function wireRefreshButtons() {
    const handlers = refreshHandlers();
    document.querySelectorAll('.widget-header .icon-btn[title="Refresh"]').forEach((btn) => {
      if (btn.dataset.wired === "1") return;
      btn.dataset.wired = "1";
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const widget = btn.closest(".widget");
        if (!widget) return;
        const id = widget.dataset.widget;
        const fn = handlers[id];
        // Visual feedback: brief spin on the icon so the click is obviously alive.
        btn.style.transition = "transform 0.6s";
        btn.style.transform = "rotate(360deg)";
        setTimeout(() => { btn.style.transform = ""; }, 600);
        if (fn) fn(widget, { explicit: true });
      });
    });
  }

  // ===== Per-widget configure panel =====
  // Per-tile gear -> opens `#widget-config-panel` and remembers which tile
  // we're editing on a module-level variable. Save commits in-memory state
  // back to the tile's dataset, applies header color, and persists via
  // dashboard_set (Tauri) or the localStorage fallback.
  let currentCfgTile = null;       // .grid-stack-item being edited
  let currentCfgDraft = null;      // edited-but-not-saved copy of cfg
  let cfgSourceLoadedFor = null;   // widget name whose source is in the DOM

  function openWidgetConfigPanel(tileItem) {
    const panel = $("#widget-config-panel");
    if (!panel) return;
    currentCfgTile = tileItem;
    const widget = tileItem.querySelector(".widget");
    const widgetName = widget ? widget.dataset.widget : "";
    const gsId = tileItem.getAttribute("gs-id") || widgetName || "";
    const title = widget && widget.querySelector(".widget-title");
    const readableName = title ? title.textContent : widgetDisplayName(widgetName);
    $("#cfg-title").textContent = readableName ? `Configure ${readableName}` : "Configure Widget";
    $("#cfg-subline").textContent = widgetName
      ? `Widget: ${widgetDisplayName(widgetName)}`
      : (gsId ? `Widget: ${displayName(gsId)}` : "");

    // Start from a deep-ish copy so Cancel really discards changes.
    const existing = tileConfig(widget || tileItem);
    currentCfgDraft = {
      context: existing.context ? { ...existing.context } : { ...INHERIT_CONTEXT },
      header_color: existing.header_color || null,
      inputs: { ...(existing.inputs || {}) },
    };

    // Account section
    $("#cfg-context-error").textContent = "";
    populateOverrideProfileSelect();
    populateOverrideRegionSelect(currentCfgDraft.context.mode === "pinned"
      ? currentCfgDraft.context.region : topbarState.region || cachedSettings?.default_region);
    const useOverride = $("#cfg-use-override");
    useOverride.checked = currentCfgDraft.context.mode === "pinned";
    refreshDefaultContextLine();
    refreshOverrideFieldsVisibility();
    if (currentCfgDraft.context.mode === "pinned") {
      const ov = currentCfgDraft.context;
      const profSel = $("#cfg-override-profile");
      const matchOpt = Array.from(profSel.options).find(o => o.value === ov.profile);
      if (matchOpt) profSel.value = ov.profile;
      $("#cfg-override-account").value = ov.account_id || "";
      const regSel = $("#cfg-override-region");
      if (Array.from(regSel.options).some(o => o.value === ov.region)) {
        regSel.value = ov.region;
      }
    }

    // Color swatches
    renderColorSwatches();

    // Source: collapsed by default; lazily fetched on first expand.
    const details = $("#cfg-source-details");
    if (details) details.open = false;
    cfgSourceLoadedFor = null;
    clear($("#cfg-source-body"));
    $("#cfg-source-body").appendChild(
      el("p", { class: "muted small" }, "Expand to load widget manifest and source."),
    );

    panel.classList.add("open");
    panel.setAttribute("aria-hidden", "false");
    $("#scrim").classList.add("open");
    $("#scrim").hidden = false;
  }

  function closeWidgetConfigPanel() {
    invalidateRequests($("#cfg-source-body"));
    const panel = $("#widget-config-panel");
    if (!panel) return;
    panel.classList.remove("open");
    panel.setAttribute("aria-hidden", "true");
    $("#scrim").classList.remove("open");
    setTimeout(() => { $("#scrim").hidden = true; }, 220);
    currentCfgTile = null;
    currentCfgDraft = null;
  }

  function populateOverrideProfileSelect() {
    const sel = $("#cfg-override-profile");
    if (!sel) return;
    resetSelect(sel);
    const cached = readProfilesCache();
    const profiles = cached ? cached.profiles.filter(profile => profile.eligibility === "supported_sso") : [];
    if (profiles.length === 0) {
      addOption(sel, "", "(no profiles — open Settings)");
      sel.disabled = true;
      return;
    }
    sel.disabled = false;
    addOption(sel, "", "(pick profile)");
    profiles.forEach(p => {
      const opt = addOption(sel, p.name, `${p.name}${p.account_id ? " · " + p.account_id : ""}`, {
        accountId: p.account_id || "",
        region: p.region || "",
      });
      void opt;
    });
    sel.onchange = () => {
      const opt = sel.options[sel.selectedIndex];
      if (!opt) return;
      $("#cfg-override-account").value = opt.dataset.accountId || "";
      // Keep unsupported values visible rather than selecting another region.
      const pr = opt.dataset.region || "";
      if (pr) populateOverrideRegionSelect(pr);
    };
  }

  function populateOverrideRegionSelect(current) {
    const sel = $("#cfg-override-region");
    if (!sel) return;
    resetSelect(sel);
    allowedRegions.forEach(r => addOption(sel, r, r));
    if (current && !allowedRegions.includes(current)) addOption(sel, current, `${current} — unsupported`);
    if (!current) addOption(sel, "", "(choose a supported region)");
    sel.value = current || "";
    sel.disabled = !allowedRegions.length;
  }

  function refreshDefaultContextLine() {
    const out = $("#cfg-default-context");
    if (!out) return;
    const profile = topbarState.profile || "(none)";
    const acct = topbarState.accountId || "(none)";
    const region = topbarState.region || "(none)";
    out.textContent = `Default (topbar): ${profile} · ${acct} · ${region}`;
  }

  function refreshOverrideFieldsVisibility() {
    const wrap = $("#cfg-override-fields");
    const checked = $("#cfg-use-override").checked;
    if (wrap) wrap.hidden = !checked;
  }

  function renderColorSwatches() {
    const wrap = $("#cfg-color-swatches");
    if (!wrap) return;
    clear(wrap);
    const colors = ["neutral", ...ALLOWED_HEADER_COLORS];
    const current = currentCfgDraft.header_color || "neutral";
    colors.forEach(c => {
      const btn = el("button", {
        type: "button",
        class: "color-swatch" + (c === current ? " active" : ""),
        "data-color": c,
        title: c,
      });
      btn.addEventListener("click", () => {
        currentCfgDraft.header_color = c === "neutral" ? null : c;
        renderColorSwatches();
      });
      wrap.appendChild(btn);
    });
  }

  async function loadSourceIntoPanel(widgetName) {
    const host = $("#cfg-source-body");
    clear(host);
    if (!isTauri) {
      host.appendChild(el("p", { class: "muted small" },
        "Source viewer is only available in the desktop app."));
      return;
    }
    host.appendChild(el("p", { class: "muted small" }, "Loading source…"));
    const tile = currentCfgTile;
    const request = beginOwnedRequest(host);
    const current = () => request.current() && currentCfgTile === tile && !!tile?.isConnected;
    let res;
    try {
      res = await tauriInvoke("widget_get_source", { params: { widget: widgetName } });
    } catch (e) {
      if (!current()) return;
      clear(host);
      host.appendChild(el("p", { class: "muted small" }, "Failed to load: " + e));
      return;
    }
    if (!current()) return;
    clear(host);
    if (!res || !res.ok) {
      host.appendChild(el("p", { class: "muted small" }, (res && res.error) || "Not found."));
      return;
    }
    if (res.doc) {
      host.appendChild(el("div", { class: "cfg-source-doc" }, res.doc));
    }
    const yamlBlock = el("div", { class: "cfg-source-block" },
      el("h4", {}, "Widget Manifest"),
      el("pre", {}, el("code", {}, res.yaml || "")),
    );
    const pyBlock = el("div", { class: "cfg-source-block" },
      el("h4", {}, "Source"),
      el("pre", {}, el("code", {}, res.py || "")),
    );
    host.appendChild(yamlBlock);
    host.appendChild(pyBlock);
    cfgSourceLoadedFor = widgetName;
  }

  function saveWidgetConfig() {
    if (!currentCfgTile || !currentCfgDraft) return;
    // Collect the widget context section into the draft.
    const useOverride = $("#cfg-use-override").checked;
    if (useOverride) {
      const profile = $("#cfg-override-profile").value.trim();
      const accountId = $("#cfg-override-account").value.trim();
      const region = $("#cfg-override-region").value.trim();
      if (!allowedRegions.includes(region)) {
        $("#cfg-context-error").textContent = "Choose a supported region before saving this pinned context.";
        return;
      }
      if (profile && accountId && region) {
        currentCfgDraft.context = { mode: "pinned", profile, account_id: accountId, region };
      } else {
        currentCfgDraft.context = { ...INHERIT_CONTEXT };
      }
    } else {
      currentCfgDraft.context = { ...INHERIT_CONTEXT };
    }
    invalidateRequests(currentCfgTile);
    clearWidgetResults($(".widget", currentCfgTile));
    // Explicit reconfiguration also invalidates/removes the old pin surfaces.
    currentCfgTile.querySelectorAll(".pipeline-pin-card").forEach(card => card.remove());
    writeTileConfig(currentCfgTile, currentCfgDraft);
    applyTileHeaderColor(currentCfgTile);
    updateWidgetContextChip(currentCfgTile);
    scheduleSaveLayout();
    renderWidgetTile(currentCfgTile);
    closeWidgetConfigPanel();
  }

  function resetWidgetConfig() {
    if (!currentCfgTile) return;
    currentCfgDraft = emptyConfig();
    $("#cfg-use-override").checked = false;
    refreshOverrideFieldsVisibility();
    renderColorSwatches();
    // Reset the inputs in the form too.
    $("#cfg-override-account").value = "";
  }

  function wireConfigButtons() {
    document.querySelectorAll('.widget-header .cfg-btn').forEach((btn) => {
      if (btn.dataset.wired === "1") return;
      btn.dataset.wired = "1";
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const item = btn.closest(".grid-stack-item");
        if (!item) return;
        openWidgetConfigPanel(item);
      });
    });
  }

  function wireRemoveButtons() {
    document.querySelectorAll('.widget-header .rm-btn').forEach((btn) => {
      if (btn.dataset.wired === "1") return;
      btn.dataset.wired = "1";
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const item = btn.closest(".grid-stack-item");
        if (!item) return;
        invalidateRequests(item);
        if (grid) {
          grid.removeWidget(item, true);
        } else {
          item.remove();
        }
        // Re-render the "+ Widget" panel so removed tiles flip from Show → Add.
        renderPrebuiltList();
      });
    });
  }

  // Esc exits fullscreen first; if no fullscreen, closes side panel.
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && fullscreenWidget) {
      e.preventDefault();
      exitFullscreen();
    }
  }, true);

  // ===== Boot =====
  async function boot() {
    pingCore();
    if (isTauri) {
      $("#account-select").disabled = true;
      $("#region-select").disabled = true;
      wireTopbarSearchablePickers();
      syncTopbarPicker($("#account-select"));
      syncTopbarPicker($("#region-select"));
      await loadSettings();
    } else $("#theme-toggle").disabled = false;
    renderPipelineRuns();
    renderLogTail();
    renderCfnStacks();
    renderLookup();
    renderErrors();
    renderCodeArtifactPackages();
    renderPrebuiltList();
    initGridStack();
    wireRefreshButtons();
    wireRemoveButtons();
    wireConfigButtons();
    wireFullscreenButtons();
    updateAllWidgetContextChips();
    bootComplete = true;
    if (isTauri) startDesktopPickers();
    else initTopbarPickers();
  }
  document.addEventListener("DOMContentLoaded", boot);
})();
