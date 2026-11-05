// Cloud Burrito — shared core: DOM helpers, bridge, request ownership,
// result states, evidence handoffs and renderers. Plain script, no build step.
// Pages adapt it through configure(); the code below is moved from app.js.
(function () {
  "use strict";

  const hooks = {
    contextFor: () => ({ mode: "inherit", profile: null, account_id: null, region: null }),
    isIndependent: (_owner, context) => context.mode === "pinned",
    selection: () => 0,
    selectionContext: () => ({ profile: null, account_id: null, region: null }),
    requestsAllowed: () => true,
    beforeFetch: () => {},
    afterFetch: () => {},
    afterPaint: () => {},
    resultTitle: host => host.closest?.("[data-result-title]")?.dataset.resultTitle || "Result",
    announceAllowed: () => true,
    openAudit: () => {},
    browserInvoke: null,
  };
  function configure(partial) { Object.assign(hooks, partial); }

  // ---- moved ranges follow ----
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
    if (tag === "button" && attrs.title && !attrs["aria-label"] && !/[A-Za-z0-9]/.test(node.textContent)) {
      node.setAttribute("aria-label", attrs.title);
    }
    return node;
  };
  const clear = (node) => {
    cancelPendingWithin(node, false);
    disposeRenderTree(node);
    while (node.firstChild) node.removeChild(node.firstChild);
  };
  // Presentation choices are immediate and local; they never save AWS settings.
  const displayPreferenceKey = "cloud-burrito.display-preferences";
  const displayPreferences = { showTips: false, expandResultDetails: false };
  let resultDetailsPreferenceRevision = 0;
  function markResultDetailsPreferenceChanged() { resultDetailsPreferenceRevision++; }
  try {
    const saved = JSON.parse(localStorage.getItem(displayPreferenceKey));
    for (const key of Object.keys(displayPreferences)) {
      if (typeof saved?.[key] === "boolean") displayPreferences[key] = saved[key];
    }
  } catch (_) { /* Safe compact defaults also work without local storage. */ }
  document.documentElement.dataset.showTips = String(displayPreferences.showTips);
  const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

  function focusableWithin(root) {
    return Array.from(root.querySelectorAll(FOCUSABLE)).filter(node => node.tabIndex >= 0
      && !node.closest("[hidden], [inert]") && node.getClientRects().length);
  }

  function focusSafely(node) {
    if (!node?.isConnected || node.disabled || node.closest("[hidden], [inert]") || !node.getClientRects().length) return false;
    node.focus({ preventScroll: true });
    return document.activeElement === node;
  }
  function announceResult(host, state) {
    if (host._announcedResultState === state) return;
    host._announcedResultState = state;
    if (!hooks.announceAllowed(host)) return;
    const announcement = $("#result-announcement");
    if (!announcement) return;
    const title = hooks.resultTitle(host);
    const messages = { loading: "loading", success: "updated", empty: "empty response", limited: "limited result",
      partial: "partial result", stale: "refresh failed; previous evidence retained", denied: "request denied",
      expired: "credentials expired", failed: "request failed", cancelled: "request cancelled" };
    const message = `${title}: ${messages[state] || "updated"}.`;
    if (announcement.textContent !== message) announcement.textContent = message;
  }
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
    if (["settings_set", "dashboard_set", "watchlist_set", "policy_set"].includes(command) && result?.ok === false) {
      const errorType = ["StorageWriteFailed", "StorageSuperseded", "AuditPreserveRequired"].includes(result.error_type) ? result.error_type : "InvalidRequest";
      const error = new Error(errorType === "StorageWriteFailed"
        ? "Changes could not be saved. Try again."
        : errorType === "StorageSuperseded" ? "Save was superseded by a newer change. Review and retry if needed."
        : errorType === "AuditPreserveRequired" ? "Preserve current audit history before enabling bounded retention."
        : "Changes were rejected. Review the input and try again.");
      error.error_type = errorType;
      if (command === "settings_set") error.settings = result._settings;
      throw error;
    }
    return result;
  } : null;
  function canInvoke() { return isTauri || typeof hooks.browserInvoke === "function"; }
  async function invoke(command, args) {
    if (isTauri) return tauriInvoke(command, args);
    if (typeof hooks.browserInvoke !== "function") throw new Error("Desktop request failed. Try again.");
    const result = await hooks.browserInvoke(command, args);
    observeDiagnostics(result);
    return result;
  }

  function observeDiagnostics(result) {
    if (result?._diagnostics?.audit_write_failed !== true || $("#audit-write-warning")) return;
    // A lost entry cannot be restored by a later successful write. Keep this
    // warning for the session, even if older responses arrive afterward.
    const warning = el("div", { id: "audit-write-warning", class: "diagnostics-warning", role: "status" },
      el("span", {}, "Some audit entries could not be saved. Inspection is still available."),
      el("button", { class: "btn btn-ghost small", type: "button", onclick: () => hooks.openAudit() }, "Open audit log"));
    document.body.appendChild(warning);
  }
  function contextOverrideFromElement(node) {
    let cur = node;
    while (cur) {
      if (cur._widgetContextOverride) return cur._widgetContextOverride;
      cur = cur.parentNode;
    }
    return null;
  }
  // A request belongs to a particular mounted surface, its ancestor lifetime,
  // and (unless explicitly pinned) one selection attempt. Equality of account
  // labels alone cannot distinguish A -> B -> A. Detaching also asks the
  // backend to cancel this subscriber; the acknowledgement is not proof that
  // remote work has stopped. A mounted explicit cancel can receive cleanup.
  let nextOwnedRequestId = 0;
  let configurationGeneration = 0;
  const ownedRequestPageId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  const pendingOwnedRequests = new Set();
  const MAX_PENDING_OWNED_REQUESTS = 256;

  function cancelOwnedRequest(request) {
    request.cancelRequested = true;
    if (request.cancelPromise) return request.cancelPromise;
    if (!pendingOwnedRequests.delete(request)) return Promise.resolve({ ok: true, cancelled_locally: true, cleanup_confirmed: false });
    request.cancelPromise = invoke("request_cancel", { params: { request_id: request.id } });
    return request.cancelPromise;
  }

  function cancelPendingWithin(node, includeOwner = true) {
    if (!node) return;
    for (const request of pendingOwnedRequests) {
      for (let ancestor = includeOwner ? request.owner : request.owner.parentNode; ancestor; ancestor = ancestor.parentNode) {
        if (ancestor !== node) continue;
        // Removed owners cannot display an acknowledgement. Their response
        // remains generation-guarded even if this control message fails.
        cancelOwnedRequest(request).catch(() => {});
        break;
      }
    }
  }

  async function invokeOwnedRequest(request, command, params) {
    if (!request.current()) throw new Error("Request owner is no longer current.");
    if (request.cancelRequested) return { ok: false, error_type: "Cancelled", error: "Request cancelled before dispatch.",
      _request: { id: request.id, outcome: "cancelled", context_id: null, provider_revision: null,
        settings_revision: null, profile: null, account_id: null, region: null } };
    if (pendingOwnedRequests.size >= MAX_PENDING_OWNED_REQUESTS) throw new Error("Too many pending requests. Wait for a result or cancel a request, then retry.");
    pendingOwnedRequests.add(request);
    try { return await invoke(command, { params: { ...params, request_id: request.id } }); }
    finally { pendingOwnedRequests.delete(request); }
  }

  function invalidateRequests(node, preserveResult = false) {
    if (!node) return;
    cancelPendingWithin(node);
    disposeRenderTree(node);
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
    const id = `ui-${ownedRequestPageId}-${++nextOwnedRequestId}`;
    const ancestors = [];
    for (let node = owner; node; node = node.parentNode) {
      ancestors.push([node, node._requestGeneration || 0]);
    }
    const ownerContext = hooks.contextFor(owner);
    const independent = hooks.isIndependent(owner, ownerContext);
    const selection = hooks.selection();
    const configuration = configurationGeneration;
    const context = { ...(contextOverride || contextOverrideFromElement(owner) || ownerContext) };
    const expected = context.mode === "pinned" ? context : hooks.selectionContext();
    const request = {
      id, owner, context, freshnessContext: JSON.stringify([configuration, independent ? "pinned" : selection, expected]),
      allowed: hooks.requestsAllowed(independent),
      current() {
        return owner.isConnected && owner._ownedRequest === request
          && configuration === configurationGeneration
          && (independent || selection === hooks.selection())
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
    if (!canInvoke()) return Promise.resolve(null);
    if (!request) throw new Error("An owned widget request is required.");
    if (!request.current()) return Promise.reject(new Error("Request owner is no longer current."));
    beginResultRequest(request, widgetName, inputs || {});
    if (!request.allowed) return Promise.reject(new Error("Select and verify an AWS account first."));
    // Await only a pending check, so a page with nothing to check dispatches in the same tick.
    const before = hooks.beforeFetch({ widgetName, inputs: inputs || {}, owner, request });
    if (typeof before?.then === "function") await before;
    const result = await invokeOwnedRequest(request, "widget_fetch", {
      widget: widgetName, inputs: inputs || {}, context: request.context,
      ...(["cfn-stack-detail", "pipeline-execution-detail", "codeartifact-package-version-history"].includes(widgetName)
        ? { reuse_result: !request.resultHost?._resultView } : {}),
    });
    hooks.afterFetch({ widgetName, inputs: inputs || {}, owner, request, result });
    return result;
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
      const retainedFocus = host.contains(document.activeElement) ? document.activeElement : null;
      clear(host);
      host.append(...host._resultView.nodes);
      host.hidden = false;
      if (retainedFocus?.isConnected) focusSafely(retainedFocus);
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
    if (spec?._request?.outcome === "cancelled" || spec?.query_state === "cancelled"
        || ["Superseded", "Cancelled", "QueryCancelled"].includes(spec?.error_type || spec?.data?.error_type)) return "cancelled";
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

  function rememberResultDetails(host) {
    const previousDetails = host.querySelector(":scope > .result-status > .result-details");
    if (previousDetails) host._resultDetailsOpen = previousDetails.open;
    return previousDetails?.querySelector("summary") === document.activeElement;
  }

  function paintResultStatus(host, state, spec, view, message, restoreDetailsFocus = false) {
    restoreDetailsFocus = rememberResultDetails(host) || restoreDetailsFocus;
    if (host._resultDetailsPreferenceRevision !== resultDetailsPreferenceRevision) {
      host._resultDetailsOpen = displayPreferences.expandResultDetails;
      host._resultDetailsPreferenceRevision = resultDetailsPreferenceRevision;
    }
    host.querySelectorAll(":scope > .result-status").forEach(node => node.remove());
    const labels = { loading: "Loading", success: "Updated", empty: "Empty response", limited: "Limited result", partial: "Partial result", stale: "Stale evidence", denied: "Denied", expired: "Credentials expired", failed: "Failed", cancelled: "Cancelled" };
    const status = el("div", { class: "result-status small", "data-state": state },
      el("strong", {}, labels[state] || "Result"));
    if (view?.cached) status.appendChild(el("span", { class: "result-cached" }, "Cached evidence · original capture time"));
    if (view?.receivedAt) status.appendChild(el("time", { datetime: view.receivedAt, class: "result-received" }, `Received ${new Date(view.receivedAt).toLocaleString()}`));
    const details = el("details", { class: "result-details", open: host._resultDetailsOpen ?? displayPreferences.expandResultDetails },
      el("summary", {}, "Result details"));
    const detailBody = el("div", { class: "result-details-body" });
    details.appendChild(detailBody);
    details.addEventListener("toggle", () => {
      if (details.isConnected) host._resultDetailsOpen = details.open;
    });
    const context = view?.context || (spec?._request?.context_id ? spec._request : null);
    if (context) detailBody.appendChild(el("div", { class: "result-context" }, `Profile: ${context.profile} · Verified account: ${context.account_id} · Region: ${context.region}`));
    const coverage = ["loading", "stale"].includes(state) && view ? view.coverage : spec?.coverage;
    if (coverage?.completeness === "limited" || state === "limited") {
      const count = coverage?.counts?.returned;
      const summary = coverage?.has_more === true
        ? `${Number.isSafeInteger(count) ? `Showing ${count} results. ` : ""}More were not loaded.`
        : "Some results were omitted or a result limit was reached.";
      status.appendChild(el("span", { class: "result-limit" }, summary));
    }
    if (state === "loading" && view) status.appendChild(el("div", {}, "Showing the previous result while refreshing."));
    const activeRequest = host._activeResultRequest;
    if (canInvoke() && state === "loading" && activeRequest?.current()) {
      const cancelState = el("span", { class: "request-cancel-status", role: "status" });
      const cancel = el("button", { type: "button", class: "btn btn-ghost small request-cancel", "aria-label": "Cancel current request" }, "Cancel");
      cancel.addEventListener("click", async () => {
        if (!activeRequest.current()) return;
        cancel.disabled = true;
        cancel.textContent = "Cancelling…";
        cancelState.textContent = "Cancellation requested. Waiting for the operation outcome.";
        try {
          await cancelOwnedRequest(activeRequest);
          if (!activeRequest.current() || !cancelState.isConnected) return;
          cancelState.textContent = "Cancelled locally. Remote cleanup is not yet confirmed.";
        } catch (_) {
          if (!activeRequest.current() || !cancelState.isConnected) return;
          cancelState.textContent = "Cancellation could not be confirmed. Waiting for the operation outcome.";
        }
      });
      status.append(cancel, cancelState);
    }
    if (state === "stale") {
      status.appendChild(el("div", {}, "Refresh did not complete. Retained evidence keeps its original timestamp."));
      status.appendChild(el("div", {}, `Refresh outcome: ${labels[resultState(spec || { ok: false })]}.`));
    }
    if (message) status.appendChild(el("div", { class: "result-failure" }, message));
    if (view && ["loading", "stale"].includes(state)) {
      status.appendChild(el("div", {}, `Retained result: ${labels[view.state] || "Updated"}.`));
      appendCoverage(detailBody, view.coverage, "Retained coverage");
    }
    if (spec && state !== "loading") appendCoverage(detailBody, spec.coverage, state === "stale" ? "Refresh coverage" : "Coverage");
    if (detailBody.childElementCount) status.appendChild(details);
    const cleanup = spec?.cleanup || spec?.data?.cleanup;
    if (cleanup) {
      const confirmed = cleanup.status === "stopped" && cleanup.remote_stop_confirmed === true;
      const labels = { stopped: "Remote stop was not confirmed.", not_confirmed: "Remote stop was not confirmed.", denied: "Remote stop was denied.", failed: "Remote stop failed.", not_attempted: "Remote stop was not attempted." };
      status.appendChild(el("div", { class: "result-cleanup" }, confirmed ? "Remote query stop confirmed." : labels[cleanup.status] || "Remote query status is unknown."));
      if (!confirmed && cleanup.remote_queries_may_still_run === true) status.appendChild(el("div", {}, "Remote queries may still be running."));
    } else if (state === "cancelled" || resultState(spec) === "cancelled") status.appendChild(el("div", { class: "result-cleanup" }, "The request is no longer awaited. This does not confirm that remote work stopped."));
    host.prepend(status);
    if (restoreDetailsFocus && document.activeElement === document.body) focusSafely(details.querySelector("summary"));
    announceResult(host, state);
    updateEvidenceLinks(host);
    hooks.afterPaint(host);
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
    const restoreDetailsFocus = rememberResultDetails(host);
    clear(host);
    if (previous) host.append(...previous.nodes);
    else host.appendChild(el("div", { class: "muted small" }, message));
    host.hidden = false;
    if (previous) {
      previous.displayState = "stale";
      previous.displaySpec = { coverage: spec?.coverage, error_type: spec?.error_type, _request: spec?._request, cleanup: spec?.cleanup || spec?.data?.cleanup, ok: false };
      previous.displayMessage = message;
    }
    paintResultStatus(host, previous ? "stale" : resultState(spec || { ok: false }), spec, previous, message, restoreDetailsFocus);
  }

  function renderWithResultState(host, spec, render) {
    const request = host._activeResultRequest;
    if (!request || !spec?._request || host._renderingResult) return render();
    if (!request.current() || spec._request.id !== request.id) return;
    if (!validCoverage(spec.coverage)) return failResult(host, request, "Response coverage metadata was invalid.");
    if (!validResultShape(spec)) return failResult(host, request, "Response data shape was invalid.");
    const cache = spec._cache;
    if (cache !== undefined && (!cache || cache.hit !== true || !Number.isFinite(cache.captured_at)
        || cache.captured_at < 0 || cache.captured_at * 1000 > Date.now() + 1000 || cache.max_age_seconds !== 15)) {
      return failResult(host, request, "Response freshness metadata was invalid.");
    }
    const previous = host._resultView?.key === request.resultKey ? host._resultView : null;
    if (resultFailure(spec) && !resultHasEvidence(spec) && previous) {
      return failResult(host, request, spec.error || spec.data?.error || spec.reason || "The refresh did not complete.", spec);
    }
    const restoreDetailsFocus = rememberResultDetails(host);
    host._renderingResult = true;
    try { render(); } finally { host._renderingResult = false; }
    const state = resultState(spec);
    const usable = !resultFailure(spec) || resultHasEvidence(spec);
    const view = { key: request.resultKey, receivedAt: new Date(cache ? cache.captured_at * 1000 : Date.now()).toISOString(), cached: !!cache, context: spec._request, state, coverage: spec.coverage,
      nodes: Array.from(host.childNodes).filter(node => !node.classList?.contains("result-status")) };
    if (usable) host._resultView = view;
    else delete host._resultView;
    host.hidden = false;
    paintResultStatus(host, state, spec, usable ? view : null, undefined, restoreDetailsFocus);
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
      invalidateRequests(next);
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

  function disposeRenderTree(node) {
    node?._disposeRender?.();
    node?.querySelectorAll?.("[data-render-resource]").forEach(child => child._disposeRender?.());
  }

  function createRowMatcher(owner, initialValues) {
    const values = initialValues.slice();
    let worker = null, ready = false, pending = null, generation = 0;
    let debounce = null, deadline = null, startupDeadline = null;
    const finish = result => {
      clearTimeout(debounce); clearTimeout(deadline); clearTimeout(startupDeadline);
      debounce = deadline = startupDeadline = null;
      const waiting = pending;
      pending = null;
      owner.dataset.filterPending = "false";
      waiting?.resolve(result);
    };
    const terminate = () => { worker?.terminate(); worker = null; ready = false; };
    const dispose = () => { generation++; terminate(); finish(null); };
    const dispatch = () => {
      if (!pending || !ready) return;
      clearTimeout(startupDeadline); startupDeadline = null;
      const id = pending.id;
      worker.postMessage({ type: "match", id, query: pending.query });
      pending.running = true;
      deadline = setTimeout(() => {
        if (pending?.id !== id) return;
        terminate();
        finish({ error: "Filter exceeded the 150 ms work limit. Previous matches are unchanged; simplify the expression." });
      }, 150);
    };
    const start = () => {
      if (!pending || !owner.isConnected) { finish(null); return; }
      if (worker && ready) { dispatch(); return; }
      if (worker) {
        startupDeadline = setTimeout(() => { terminate(); finish({ error: "The filter worker did not become ready. Previous matches are unchanged." }); }, 2000);
        return;
      }
      try {
        worker = new Worker(new URL("row-filter-worker.js", document.baseURI));
        const instance = worker;
        worker.onmessage = event => {
          if (worker !== instance) return;
          if (event.data?.type === "ready") { ready = true; dispatch(); return; }
          if (!pending || event.data?.id !== pending.id) return;
          finish(event.data);
        };
        worker.onerror = event => {
          if (worker !== instance) return;
          event.preventDefault(); terminate();
          finish({ error: "The filter worker is unavailable. Previous matches are unchanged." });
        };
        worker.postMessage({ type: "init", values });
        startupDeadline = setTimeout(() => {
          terminate(); finish({ error: "The filter worker did not become ready. Previous matches are unchanged." });
        }, 2000);
      } catch (_) { terminate(); finish({ error: "The filter worker is unavailable. Previous matches are unchanged." }); }
    };
    owner.dataset.renderResource = "filter";
    owner._disposeRender = dispose;
    return {
      dispose,
      patch(entries) {
        entries.forEach(([index, value]) => { values[index] = value; });
        if (worker) worker.postMessage({ type: "patch", entries });
      },
      match(query) {
        generation++;
        if (pending?.running) terminate();
        finish(null);
        if (!query) return Promise.resolve({ valid: true, matches: values.map((_, index) => index) });
        if (query.length > 512) return Promise.resolve({ error: "Filter is limited to 512 characters. Previous matches are unchanged." });
        owner.dataset.filterPending = "true";
        return new Promise(resolve => {
          pending = { id: generation, query, resolve, running: false };
          debounce = setTimeout(start, 30);
        });
      },
    };
  }

  function renderTable(host, spec, opts) {
    if (spec.render !== "table" && spec.render) return dispatchRender(host, spec);
    return renderWithResultState(host, spec, () => renderTableContent(host, spec, opts));
  }

  function renderTableContent(host, spec, opts = {}) {
    const active = host.contains(document.activeElement) ? document.activeElement : null;
    const focusedRow = active?.closest?.("tr[data-source-row]");
    let restoreTarget = active?.matches?.(".table-filter")
      ? { kind: "filter", start: active.selectionStart, end: active.selectionEnd }
      : active?.matches?.(".table-page-previous") ? { kind: "previous" }
      : active?.matches?.(".table-page-next") ? { kind: "next" }
      : focusedRow ? { kind: "row", key: focusedRow._logicalRowKey,
        control: active === focusedRow ? -1 : Array.from(focusedRow.querySelectorAll("button,a,input,select,textarea,summary")).indexOf(active) } : null;
    clear(host);
    if (spec.render !== "table") { host.appendChild(el("pre", { class: "raw-json" }, JSON.stringify(spec, null, 2))); return; }
    if (spec.error) host.appendChild(el("div", { class: "muted small table-error" }, "Error: " + spec.error));
    const allColumns = Array.isArray(spec.columns) ? spec.columns : [];
    const columns = allColumns.slice(0, 32);
    const rows = Array.isArray(spec.rows) ? spec.rows : [];
    if (!rows.length && resultFailure(spec)) return;
    if (allColumns.length > columns.length) host.appendChild(el("p", { class: "muted small" }, `Display limited to ${columns.length} of ${allColumns.length} columns. Narrow the source request to inspect other columns.`));
    const expandable = typeof opts.expand === "function";
    const paginated = rows.length > 100;
    const key = host._activeResultRequest?.resultKey || "browser-table";
    const presentation = host._tablePresentation?.key === key ? host._tablePresentation : { key, page: 0, query: "", expanded: new Set(), focused: null };
    host._tablePresentation = presentation;
    const baseKeys = rows.map((row, index) => String(opts.rowKey?.(row) ?? row[columns[0]] ?? index));
    const keyCounts = new Map();
    baseKeys.forEach(value => keyCounts.set(value, (keyCounts.get(value) || 0) + 1));
    const rowKeys = baseKeys.map((value, index) => keyCounts.get(value) > 1 ? `${value}:${index}` : value);
    const existingKeys = new Set(rowKeys);
    presentation.expanded.forEach(value => { if (!existingKeys.has(value)) presentation.expanded.delete(value); });
    let matching = rows.map((_, index) => index);
    let filterGeneration = 0;
    let matcher = null;
    let filterControl = null;
    let activePreview = null;
    let reapplyMetadataFilter = () => {};
    const mounted = new Map();
    const details = new Map();
    for (const detail of spec.cell_details || []) {
      if (detail && Number.isSafeInteger(detail.row) && detail.row >= 0 && detail.row < rows.length && typeof detail.column === "string") {
        details.set(`${detail.row}:${detail.column}`, detail);
      }
    }
    const tbody = el("tbody", {});
    const table = el("table", { class: "events-table" },
      el("thead", {}, el("tr", {}, ...columns.map(column => el("th", { title: String(column).slice(0, 256) }, tableHeaderLabel(column))))), tbody);
    const feedback = el("div", { class: "table-filter-feedback muted small", role: "status", hidden: true });
    const empty = el("div", { class: "table-filter-empty", role: "status", "aria-live": "polite", hidden: true }, opts.filterEmptyText || "No matching rows.");
    const count = el("span", { class: "table-filter-count muted small" }, String(rows.length));
    const pageLabel = el("span", { class: "table-page-count small", role: "status" });
    const previous = el("button", { type: "button", class: "btn btn-ghost small table-page-previous" }, "Previous rows");
    const next = el("button", { type: "button", class: "btn btn-ghost small table-page-next" }, "Next rows");
    const pager = el("nav", { class: "table-pagination", "aria-label": "Result pages", hidden: !paginated }, previous, pageLabel, next);
    const textFor = (row, index) => allColumns.map(column => {
      const detail = details.get(`${index}:${column}`);
      if (detail && !detail.unavailable && Object.hasOwn(detail, "value")) {
        try { return typeof detail.value === "string" ? detail.value : JSON.stringify(detail.value); }
        catch (_) { /* A malformed detail does not replace its safe preview. */ }
      }
      return String(row[column] ?? "");
    }).join(" ");

    function makeCell(row, index, column) {
      const raw = String(row[column] ?? "");
      const custom = typeof opts.renderCell === "function" ? opts.renderCell(column, row) : null;
      if (custom) return el("td", {}, custom);
      const formatted = formatDisplayValue(column, raw);
      const td = el("td", { ...(formatted.className ? { class: formatted.className } : {}),
        ...(raw && raw !== formatted.text ? { title: raw.slice(0, 256) } : {}) });
      const preview = formatted.text.length > 512 ? formatted.text.slice(0, 511) + "…" : formatted.text;
      td.appendChild(column === "status" ? el("span", { class: "badge " + statusToBadge(raw) }, preview) : document.createTextNode(preview));
      const detail = details.get(`${index}:${column}`);
      if (raw.length <= 512 && !detail) return td;
      const disclosure = el("details", { class: "table-cell-detail" });
      disclosure.appendChild(el("summary", { "aria-label": `Inspect full ${String(column).slice(0, 100)} value` }, "Full value"));
      disclosure.addEventListener("toggle", () => {
        if (!disclosure.isConnected) return;
        disclosure.querySelector(".table-full-value")?.remove();
        if (!disclosure.open) return;
        if (activePreview && activePreview !== disclosure) activePreview.open = false;
        activePreview = disclosure;
        let value = raw;
        let unavailable = detail?.unavailable === true;
        if (detail && Object.hasOwn(detail, "value")) {
          try {
            value = typeof detail.value === "string" ? detail.value : JSON.stringify(detail.value, null, 2);
            if (value?.length > 65536 && typeof detail.value !== "string") value = JSON.stringify(detail.value);
          } catch (_) { unavailable = true; }
        }
        if (typeof value !== "string" || value.length > 65536) unavailable = true;
        disclosure.appendChild(unavailable
          ? el("p", { class: "table-full-value muted small" }, "This value exceeds the display limit. Use --query to return a smaller value, or narrow the source request.")
          : el("pre", { class: "table-full-value" }, value));
      });
      td.appendChild(disclosure);
      return td;
    }

    function createRow(index) {
      const row = rows[index];
      const tr = el("tr", { "data-source-row": String(index) }, ...columns.map(column => makeCell(row, index, column)));
      // Search source lives in the worker; the bounded DOM attribute remains
      // available to the existing small progressive package renderer.
      tr._logicalRowKey = rowKeys[index];
      tr.dataset.search = textFor(row, index).slice(0, 1024);
      tr._tableRowUpdated = () => {
        matcher?.patch([[index, textFor(row, index)]]);
        reapplyMetadataFilter();
      };
      tr.addEventListener("focusin", () => { presentation.focused = rowKeys[index]; });
      if (expandable) {
        tr.classList.add("expandable"); tr.tabIndex = 0;
        tr.setAttribute("aria-label", (`Inspect ${columns.map(column => String(row[column] ?? "")).filter(Boolean).join(" · ")}`).slice(0, 256));
        tr.setAttribute("aria-expanded", "false");
        const toggle = () => {
          const expanded = tr.classList.contains("expanded");
          if (!expanded && presentation.expanded.size >= 10) {
            feedback.hidden = false; feedback.textContent = "Up to 10 row details can remain open. Close one before opening another."; return;
          }
          toggleRowDetail(tr, row, opts, columns.length);
          if (expanded) presentation.expanded.delete(rowKeys[index]);
          else presentation.expanded.add(rowKeys[index]);
        };
        tr.addEventListener("click", event => { if (!event.target.closest("button, a, input, select, textarea, summary, details")) toggle(); });
        tr.addEventListener("keydown", event => {
          if (event.target === tr && ["Enter", " "].includes(event.key)) { event.preventDefault(); toggle(); }
        });
      }
      opts.onRowCreated?.(tr, row);
      mounted.set(index, tr);
      return tr;
    }

    function renderPage() {
      const maxPage = Math.max(0, Math.ceil(matching.length / 100) - 1);
      presentation.page = Math.min(presentation.page, maxPage);
      if (paginated || mounted.size === 0) {
        if (activePreview) { activePreview.open = false; activePreview = null; }
        clear(tbody); mounted.clear();
        const visible = paginated ? matching.slice(presentation.page * 100, (presentation.page + 1) * 100) : rows.map((_, index) => index);
        for (const index of visible) {
          const tr = createRow(index); tbody.appendChild(tr);
          if (expandable && presentation.expanded.has(rowKeys[index])) toggleRowDetail(tr, rows[index], opts, columns.length);
        }
      }
      if (!paginated) {
        const shown = new Set(matching);
        mounted.forEach((tr, index) => {
          tr.hidden = !shown.has(index);
          if (tr.nextElementSibling?.classList.contains("row-detail")) tr.nextElementSibling.hidden = tr.hidden;
        });
      }
      const first = matching.length ? presentation.page * 100 + 1 : 0;
      pageLabel.textContent = `Showing ${first}–${Math.min((presentation.page + 1) * 100, matching.length)} of ${matching.length} matching rows`;
      previous.disabled = presentation.page === 0;
      next.disabled = presentation.page >= maxPage;
      empty.hidden = matching.length !== 0;
      table.hidden = matching.length === 0;
      host.scrollTop = 0;
    }
    function restoreControlFocus() {
      const saved = restoreTarget;
      restoreTarget = null;
      if (!saved || document.activeElement !== document.body) return;
      let target;
      if (saved.kind === "filter") target = filterControl;
      else if (saved.kind === "previous") target = previous;
      else if (saved.kind === "next") target = next;
      else {
        const tr = mounted.get(rowKeys.indexOf(saved.key));
        target = saved.control >= 0 ? tr?.querySelectorAll("button,a,input,select,textarea,summary")[saved.control] : tr;
      }
      if (!target || target.disabled || target.hidden) return;
      focusSafely(target);
      if (saved.kind === "filter" && Number.isInteger(saved.start) && Number.isInteger(saved.end)) target.setSelectionRange(saved.start, saved.end);
    }
    previous.addEventListener("click", () => { if (!previous.disabled) { presentation.page--; renderPage(); } });
    next.addEventListener("click", () => { if (!next.disabled) { presentation.page++; renderPage(); } });

    if (rows.length > 10 || opts.alwaysFilter) {
      const placeholder = opts.filterPlaceholder || "filter rows (regex)…";
      const filter = el("input", { class: "table-filter", type: "text", placeholder, "aria-label": placeholder,
        autocomplete: "off", autocorrect: "off", spellcheck: "false", value: presentation.query });
      filterControl = filter;
      matcher = createRowMatcher(filter, rows.map(textFor));
      let metadataFilterScheduled = false;
      let failedQuery = null;
      const applyFilter = async () => {
        const generation = ++filterGeneration;
        const query = filter.value.trim();
        feedback.hidden = false; feedback.textContent = "Filtering…";
        const result = await matcher.match(query);
        if (!result || generation !== filterGeneration || !filter.isConnected) return;
        filter.classList.toggle("table-filter-bad", !!result.error || !result.valid);
        filter.setAttribute("aria-invalid", String(!!result.error || !result.valid));
        if (result.error) { failedQuery = query; feedback.textContent = result.error; restoreControlFocus(); return; }
        failedQuery = null;
        matching = result.matches;
        presentation.query = query;
        presentation.page = 0;
        feedback.textContent = result.valid ? "" : "Invalid regex; showing case-insensitive literal matches.";
        feedback.hidden = result.valid;
        count.textContent = query ? `${matching.length} / ${rows.length}` : String(rows.length);
        renderPage();
        restoreControlFocus();
      };
      reapplyMetadataFilter = () => {
        if (metadataFilterScheduled || !filter.value.trim() || failedQuery === filter.value.trim()) return;
        metadataFilterScheduled = true;
        queueMicrotask(() => {
          metadataFilterScheduled = false;
          if (filter.isConnected) applyFilter();
        });
      };
      filter.addEventListener("input", () => { failedQuery = null; applyFilter(); });
      host.appendChild(el("div", { class: "table-filter-bar" }, filter, count));
      // The initial restore must occur after mounting, just like user input.
      if (presentation.query) requestAnimationFrame(() => { if (filter.isConnected) applyFilter(); });
    }
    host.append(feedback, empty, table, pager);
    renderPage();
    if (!presentation.query || !filterControl) restoreControlFocus();
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
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const nextTab = event.key === "Home" ? resourceTab : event.key === "End" ? eventTab
        : event.target === resourceTab ? eventTab : resourceTab;
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

  async function copyToClipboard(text, btn) {
    // Feedback reflects the clipboard result; a rejected fallback is not a copy.
    const intent = btn ? (btn._copyIntent || 0) + 1 : 0;
    if (btn) { btn._copyIntent = intent; btn._copyLabel ??= btn.textContent; }
    let copied = false;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
        copied = true;
      }
    } catch (_) { /* Try the local fallback below. */ }
    if (!copied) copied = fallbackCopy(text);
    if (!btn || !btn.isConnected || btn._copyIntent !== intent) return;
    btn.textContent = copied ? btn.classList.contains("exec-icon-btn") ? "✓" : "Copied" : "Copy failed";
    btn.dataset.copyState = copied ? "copied" : "failed";
    const status = $("#clipboard-status");
    if (status) status.textContent = copied ? "Copied to clipboard." : "Copy failed. Try again.";
    clearTimeout(btn._copyTimer);
    btn.dataset.renderResource = "clipboard-timer";
    btn._disposeRender = () => { clearTimeout(btn._copyTimer); btn._copyTimer = null; };
    btn._copyTimer = setTimeout(() => {
      if (!btn.isConnected) return;
      if (btn._copyIntent !== intent) return;
      btn.textContent = btn._copyLabel;
      delete btn.dataset.copyState;
    }, 1600);
  }

  function fallbackCopy(text) {
    const focused = document.activeElement;
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    ta.setAttribute("aria-label", "Temporary clipboard text");
    document.body.appendChild(ta);
    ta.select();
    let copied = false;
    try { copied = document.execCommand("copy") === true; } catch (_) { /* Report failure. */ }
    ta.remove();
    if (focused?.isConnected) focused.focus({ preventScroll: true });
    return copied;
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
      "aria-label": "Filter stacks or log groups",
      placeholder: "filter stacks or log groups (regex)...",
      autocomplete: "off",
      autocorrect: "off",
      spellcheck: "false",
    });
    const count = el("span", { class: "table-filter-count muted small" }, String(rows.length));
    const empty = el("div", { class: "lookup-empty errors-filter-empty", hidden: true },
      "No matching stacks or log groups.");
    const matcher = createRowMatcher(filter, rowEls.map(row => row.dataset.search));
    const filterFeedback = el("div", { class: "table-filter-feedback muted small", role: "status", hidden: true });
    let generation = 0;
    const applyFilter = async () => {
      const current = ++generation;
      const q = filter.value.trim();
      filterFeedback.hidden = false;
      filterFeedback.textContent = "Filtering…";
      const result = await matcher.match(q);
      if (!result || current !== generation || !filter.isConnected) return;
      filter.classList.toggle("table-filter-bad", !!result.error || !result.valid);
      filter.setAttribute("aria-invalid", String(!!result.error || !result.valid));
      if (result.error) { filterFeedback.textContent = result.error; return; }
      const shown = new Set(result.matches);
      rowEls.forEach((row, index) => { row.hidden = !shown.has(index); });
      filterFeedback.textContent = result.valid ? "" : "Invalid regex; showing case-insensitive literal matches.";
      filterFeedback.hidden = result.valid;
      count.textContent = q ? `${shown.size} / ${rows.length}` : String(rows.length);
      empty.hidden = shown.size !== 0;
      wrap.hidden = shown.size === 0;
    };
    filter.addEventListener("input", applyFilter);
    host.appendChild(el("div", { class: "table-filter-bar errors-filter-bar" }, filter, count));
    host.append(filterFeedback, empty, wrap);
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

  function codeArtifactCopyButton(version, label) {
    if (!version) return null;
    const button = el("button", {
      class: "exec-btn exec-icon-btn codeartifact-copy-btn",
      type: "button",
      title: label.slice(0, 256),
      "aria-label": label.slice(0, 256),
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
    if (historyOptions.ensureEnriched && row.enrichment_state && row.enrichment_state !== "complete") {
      const waiting = beginOwnedRequest(cell, historyOptions.context);
      clear(cell);
      cell.appendChild(el("div", { class: "muted small", role: "status" }, "Waiting for package details…"));
      await historyOptions.ensureEnriched(row);
      if (!waiting.current()) return;
      if (row.enrichment_state === "failed" && codeArtifactVersions(row).length === 0) {
        clear(cell);
        cell.appendChild(el("div", { class: "muted small" }, "Package details could not be loaded. Version history is unknown."));
        cell.appendChild(el("button", { type: "button", class: "exec-btn small", onclick: () => loadCodeArtifactVersionHistory(row, cell, historyOptions) }, "Retry package details"));
        return;
      }
    }
    const localVersions = codeArtifactVersions(row);
    if (!canInvoke() || !historyOptions.form) {
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

  function codeArtifactLatestCell(row) {
    const version = String(row.latest_version || "");
    const state = row.enrichment_state;
    if (!version) return el("span", { class: "codeartifact-enrichment-state" }, state === "pending"
      ? "Details pending" : state === "failed" ? "Details unavailable" : "");
    return el("span", { class: "codeartifact-latest-version" },
      el("span", { class: "codeartifact-latest-version-value", title: version.slice(0, 256) }, version.length > 512 ? version.slice(0, 511) + "…" : version),
      codeArtifactCopyButton(version, `Copy latest version ${version}`),
      ...(state === "failed" ? [el("span", { class: "codeartifact-enrichment-state muted small" }, "Some details unavailable")] : []),
    );
  }

  function renderCodeArtifactPackagesTable(host, spec, historyOptions = {}) {
    renderTable(host, spec, {
      expand: (row, cell) => loadCodeArtifactVersionHistory(row, cell, historyOptions),
      onRowCreated: (tr, row) => {
        tr.dataset.package = row.package;
        tr.dataset.enrichmentState = row.enrichment_state || "complete";
        historyOptions.rowElements?.set(row.package, tr);
      },
      renderCell: (column, row) => {
        if (column !== "latest_version") return null;
        return codeArtifactLatestCell(row);
      },
    });
  }

  function updateCodeArtifactPackageRow(tr, row, columns) {
    if (!tr?.isConnected) return;
    tr.dataset.enrichmentState = row.enrichment_state;
    for (let index = 0; index < columns.length; index++) {
      const column = columns[index];
      if (!["latest_version", "last_published"].includes(column)) continue;
      const td = tr.children[index];
      if (!td) continue;
      const hadFocus = td.contains(document.activeElement);
      clear(td);
      if (column === "latest_version") td.appendChild(codeArtifactLatestCell(row));
      else {
        const formatted = formatDisplayValue(column, String(row[column] || ""));
        td.textContent = formatted.text;
        if (formatted.className) td.className = formatted.className;
      }
      if (hadFocus) focusSafely(td.querySelector("button") || tr);
    }
    tr.dataset.search = columns.map(column => String(row[column] ?? "")).join(" ").slice(0, 1024);
    tr._tableRowUpdated?.();
    tr.setAttribute("aria-label", (`Inspect ${columns.map(column => String(row[column] ?? "")).filter(Boolean).join(" · ")}`).slice(0, 256));
  }
  window.CloudBurritoCore = {
    configure, configurationGeneration: () => configurationGeneration,
    bumpConfiguration: () => ++configurationGeneration,
    displayPreferenceKey, displayPreferences, markResultDetailsPreferenceChanged,
    resultDetailsPreferenceRevision: () => resultDetailsPreferenceRevision,
    $, el, clear, focusableWithin, focusSafely, announceResult,
    CAMEL_WORD_SPLIT, displayName, tableHeaderLabel, breakableIdentifier,
    isTauri, tauriInvoke, invoke, canInvoke, observeDiagnostics, contextOverrideFromElement,
    cancelOwnedRequest, cancelPendingWithin, invokeOwnedRequest, invalidateRequests, requestFailureLabel,
    beginOwnedRequest, fetchWidgetData, resultHostForRequest, beginResultRequest, validCoverage,
    resultHasEvidence, resultFailure, resultState, appendCoverage, rememberResultDetails, paintResultStatus,
    HANDOFF_SOURCES, evidenceOrigin, reviewedHandoff, updateEvidenceLinks, appendEvidenceHandoffs,
    renderEvidenceLookup, loadEvidenceStreams, failResult, renderWithResultState, validResultShape,
    renderArrayResult, restoreResultStatus, reuseResultBrowser, sectionEmptyMessage, fetchWidgetInto,
    wireRequestInputs, ISO_DATE_TIME_RE, DATE_VALUE_COLUMNS, STATUS_VALUE_COLUMNS, normalizedColumnKey,
    parseIsoDateTime, sameLocalDate, formatLocalTime, formatDateTime, formatDisplayValue, formatStatusLabel,
    toggleRowDetail, disposeRenderTree, createRowMatcher, renderTable, renderTableContent, statusToBadge,
    renderStackDetail, supportedConsoleUrl, renderExecutionDetail, copyToClipboard, fallbackCopy,
    renderRawJson, renderError, renderPermissionDenied, dispatchRender, dispatchRenderContent,
    renderErrorsLive, renderErrorsBars, renderLogStreamLive, renderReverseLookupLive, cfnEventBadge,
    codeArtifactCopyButton, codeArtifactVersions, formatCodeArtifactPublished,
    renderCodeArtifactVersionHistory, renderCodeArtifactVersionError, loadCodeArtifactVersionHistory,
    codeArtifactLatestCell, renderCodeArtifactPackagesTable, updateCodeArtifactPackageRow,
  };
})();
