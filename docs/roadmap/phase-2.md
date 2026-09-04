# P2 — Make the core dependable

Status: **P2-01 complete locally; P2-02 next. Each P2 unit is validated and committed separately; native and live-provider acceptance remains pending.**

Source baseline: `0.2.9`, commit `095d1ad`, reviewed 2026-09-03. This document changes no application behavior and records no new test result. Planning does not wait for laptop availability; native acceptance remains P5.

## Outcome and boundaries

A user can establish a verified context, complete the flagship investigation, retain useful settings, and recover from failures without guessing whether data or a save is trustworthy.

Use [CB-J01–CB-J05](phase-0.md#acceptance-fixtures) as the acceptance contracts. P2 implements their product behavior; P1 owns execution/identity enforcement, P3 owns performance and bounded-work optimization, P4 owns packages, and P5 owns real-device acceptance.

Keep Rust + Tauri and the current frontend. Extract shared state/rendering helpers only where a work unit needs them; do not combine this phase with a framework migration, whole-file rewrite, new AWS services, automatic login tooling, or an AI assistant.

## Inputs required from P1

- Verified active and pinned context, including profile, actual account, region, and request generation.
- A response contract that identifies its originating context and rejects obsolete generations before display.
- Distinct policy denial, identity failure, credential expiry, logical cancellation, and execution failure outcomes with redacted diagnostics. A discarded response is not evidence that a remote query stopped.
- Injected filesystem, credential, transport, and process boundaries for synthetic tests; unexpected real AWS/CLI work fails closed.
- A supported credential/operation contract. P2 recovery text must not suggest widening policy for an operation P1 forbids.

P1 now supplies these interfaces; see [P1-06 evidence](p1-06-evidence.md) for the accepted local boundary and deferred native checks. P2 planning and fixture design can proceed independently; interface mocks do not prove P1 enforcement.

## Source-grounded starting point

| Current observation | Implementation touchpoint |
| --- | --- |
| Settings load failures silently become defaults; save failures are discarded | [settings.rs](../../src-tauri/src/settings.rs): `load`, `save`; [commands.rs](../../src-tauri/src/commands.rs): `settings_get`, `settings_set` |
| Dashboard uses temporary-file replacement but discards write/rename errors | [dashboard.rs](../../src-tauri/src/dashboard.rs): `load`, `save`; [app.js](../../frontend/app.js): `saveLayout`, `loadLayout` |
| Frontend/backend default-profile values differ; regions are duplicated; theme is session-only | [app.js](../../frontend/app.js): `SETTINGS_DEFAULTS`, `ALLOWED_REGIONS`, theme handler; [settings.rs](../../src-tauri/src/settings.rs): `defaults`, `ALLOWED_REGIONS` |
| Discovery/auth UI exists, but some failures are console-only and labels can advance before verified identity | [app.js](../../frontend/app.js): `initTopbarPickers`, `applyTopbarSelection`, `refreshAuthStatus`, `renderIdentityPanel` |
| Pipeline → execution → CodeBuild log exists; stack/log browsing is separate | [app.js](../../frontend/app.js): `loadSelectedPipelineRuns`, `renderExecutionDetail`, `toggleActionLog`, `renderStackDetail`, `renderCloudwatchLogs` |
| Live lookup replaces its input while the handler still reads the old node | [app.js](../../frontend/app.js): `renderLookupLiveBind` |
| Global Search only receives shortcut focus; AI Generate returns a fixed sample | [app.js](../../frontend/app.js): global-search shortcut, mock AI handler; [index.html](../../frontend/index.html): visible controls |

## Ordered work units

All units below are **not started**. Review and complete one unit before combining unrelated changes. Each completion record should identify the diff, journey IDs, relevant automated evidence, remaining native checks, and known limitations.

### P2-01 — Make persistence truthful

**Complete locally:** [implementation and validation evidence](p2-01-evidence.md).

**Scope:** settings/dashboard storage and command responses; CB-J05.

- Define validated settings/layout inputs and structured load/save errors. Treat a missing file as first-run state; distinguish malformed content and permission/I/O failures.
- Return save success only after replacement succeeds; preserve the previous valid file on serialization, write, or rename failure. Serialize overlapping saves so an older completion cannot replace newer accepted state.
- Preserve supported existing values and pinned contexts during schema changes. Do not overwrite a malformed file merely by opening the app; expose recovery before resetting it.
- Reuse P1's injected filesystem boundary instead of mutating process HOME or reading personal settings in tests.

**Accept:** synthetic settings/layout survive reopen; injected write/rename failure returns an error and leaves the old file unchanged; a corrupt file produces a diagnostic without an automatic overwrite.

**Review unit:** storage behavior plus command propagation and focused failure tests. No unrelated UI redesign.

### P2-02 — Make routine settings predictable

**Scope:** Settings panel, theme, region/profile defaults, layout-save feedback; CB-J01/CB-J05. Depends on P2-01.

- Use backend-resolved defaults as the authority for AWS config path, optional SSO session, default profile, and default region. Define blank/reset behavior once; remove the current conflicting default-profile assumption.
- Use one reviewed region catalogue for default and pinned selectors. Validate against the supported beta scope; do not silently replace an unsupported saved region with another region.
- Persist explicit light/dark theme and restore it at startup. Retain existing layout, inputs, and pinned-context persistence; no new refresh scheduler is required for beta.
- Show saving, saved, unsaved, and failed states. Failed saves retain editable input and offer retry; closing a panel does not falsely imply success.
- Apply credential/config changes through P1's context invalidation and re-verification contract. Do not retain a verified badge from the previous configuration.

**Accept:** changed values survive restart; invalid values identify the field; save failure never shows “Saved”; overlapping saves leave the latest accepted value durable; a config change cannot reuse obsolete identity.

**Review units:** settings/defaults and save feedback first, then theme/layout feedback as a separate small change.

### P2-03 — Provide a clear first-run and recovery path

**Scope:** existing Settings, account picker, auth status, and Identity panel; CB-J01. Depends on P1 identity/errors and P2-02.

- Present configuration discovery, no profiles, malformed config, unsupported credential mechanism, verifying, connected, expired, and failed as understandable states.
- Show the selected profile and actual verified account/region together. Keep account-dependent work unavailable until verification succeeds; show the exact supported recovery action and retry control.
- Explain how to use an existing SSO login/session. Do not add automatic credential creation or a login subprocess to this phase.
- Explain optional CLI absence at the CLI feature; valid SDK-backed widgets remain available. Consume a backend availability result with synthetic present/missing cases in P2; P4 completes platform discovery rather than duplicating path rules in JavaScript.
- Make recovery reachable from visible controls, not a hidden label-click or console warning. Keep diagnostics free of tokens and configuration contents.

**Accept:** synthetic missing/malformed config, expired SSO, mismatch, and absent CLI each reach the correct recovery state; correcting the dependency and retrying succeeds without an app restart.

**Review unit:** one existing-panel flow and production-bridge scenarios, without introducing a separate onboarding framework.

### P2-04 — Make result state and freshness explicit

**Scope:** core widget renderers and refresh handlers; CB-J02/CB-J03/CB-J05. Depends on P1 response/context contract.

| State | Required visible behavior |
| --- | --- |
| Loading/verifying | Explain what is pending; do not label old results as the new context |
| Success/empty | Show verified context and last-success time; a successful empty result is distinct from failure |
| Stale | Retained same-context evidence shows its original timestamp and stale label |
| Partial/truncated | Show available evidence plus known coverage/limit and failure reason; never imply completeness |
| Denied/expired/failed | Show a safe reason and appropriate recovery; forbidden operations get no “grant this action” hint |
| Cancelled | Show that the request is no longer awaited; distinguish logical cancellation from confirmed child/query termination and pending/failed cleanup; reject obsolete results |

On account/region change, clear inherited old-context results during verification; pinned results retain their own context. This is stricter than retaining explicitly stale data after a same-context refresh fails.

P2 must preserve and propagate partial/truncated/error metadata for existing capped and failed-page paths, including [codebuild_log.rs](../../src-tauri/src/widgets/codebuild_log.rs) and [errors_by_stack.rs](../../src-tauri/src/widgets/errors_by_stack.rs). Update current producer models and bridge/render adapters where they discard coverage, limits, or failure reasons. Report present limits without changing request concurrency, page budgets, or remote query-cleanup behavior.

**Accept:** controlled response ordering cannot cross context; current capped/failed-page fixtures retain their available evidence and visible reason; empty/error/partial outcomes differ; a successful retry replaces the result once and clears the failure. P1 supplies logical cancellation. P2 renders agreed synthetic cleanup outcomes; this does not establish remote cancellation. P3 later implements query cleanup and extends the same metadata model for progressive results. P2 completion does not wait for that P3 integration.

**Review units:** shared rendering/state contract, existing producer-metadata propagation, then migrate the flagship widgets before other retained beta widgets. Avoid independent per-widget state conventions.

### P2-05 — Complete the flagship investigation without invented links

**Scope:** Pipeline → Build → Stack → Logs and lookup; CB-J03, with CB-J02 context guarantees. Depends on P2-04.

- First repair live lookup binding so the visible input supplies the query; cancel/deprecate prior lookups using P1 generation rules. Preserve the existing mock path's useful behavior.
- Retain the working pipeline execution and inline build-log route. Carry verified context and stable execution/build identifiers into every subsequent action.
- Where returned evidence includes a stack or log-group identifier, offer an explicit handoff to the existing stack/log view. Validate the identifier/context; do not infer ownership from similar names.
- When the relationship is absent, say it is unknown and offer lookup/manual selection in the same context. The user may choose a candidate; the app must distinguish that choice from an established relationship.
- Preserve build/log absence, permission failure, partial-page failure, and truncation information. An optional console link supplements the in-app evidence and must identify the intended resource/context.

**Accept:** a synthetic failed execution reaches its expected build error, stack event, and log evidence without reselecting account/region; unknown-association and denied/partial variants remain navigable and honest. Pinned A/B variants cannot exchange results.

**Review units:** live lookup repair; explicit handoff/context plumbing; complete flagship synthetic acceptance. These are separate reviewable slices.

### P2-06 — Align the beta surface with working behavior

**Scope:** visible controls and help text; CB-J01/CB-J03/CB-J04.

- Beta default: omit the placeholder Global Search and fixed-sample AI generator from the released interface. Keep working per-widget search/filter controls. Do not retain a keyboard shortcut that focuses an unavailable control.
- Keep “read-only” and Identity-panel wording aligned with P1's actual R/Q/C guarantees; query execution and credential acquisition must not be described as ordinary resource reads.
- Inventory retained controls and their empty/error behavior. Show a clear disabled explanation only for a real prerequisite, not an unfinished promised feature.

**Accept:** no visible beta control appears to perform a capability it lacks; no sample generation is presented as live AI; no identity/permission text contradicts P1 enforcement.

**Review unit:** scoped beta-surface changes and updated user copy; no global-search engine or AI integration.

### P2-07 — Cover keyboard and readable desktop use

**Scope:** account/region selectors, settings/panels, pipeline/stack tabs, row expansion, refresh and retry controls in CB-J01–CB-J05.

- Ensure keyboard reachability, visible focus, meaningful labels, Enter/Space activation, and correct tab/expanded state. Preserve existing keyboard-capable tables and pickers.
- Manage focus on panel open/close and Escape; hidden panels must not retain keyboard focus. Announce loading/save/failure status without repeatedly interrupting screen-reader output.
- Check both themes for readable text/status contrast; do not communicate failure or freshness by colour alone.
- Use 1280×800 and 1024×768 CSS viewports as desktop layout fixtures; check enlarged text/200% zoom. Keep primary actions reachable, permitting table-local scrolling for wide data.

**Accept:** the critical workflow and recovery routes can be completed without a pointer; focus returns predictably; text enlargement does not hide Save, Retry, context, or failure information.

**Review units:** critical controls/focus first, then contrast and compact-window defects observed during review. No claim of complete accessibility certification.

## Future validation and completion evidence

The existing frontend suite has six tests: five browser/demo tests and one CodeArtifact production path with a fake Tauri bridge. These do not prove native onboarding, flagship navigation, CLI safety, context ordering, or durable-error recovery. Existing Rust persistence tests cover successful normalization/filtering, not injected I/O failures.

| Evidence layer | Planned evidence | When |
| --- | --- | --- |
| Rust unit/integration | Injected missing/corrupt/read-only/write/rename cases; overlapping saves; preserve previous data; command errors propagate | During P2 implementation |
| Existing widget response models | Current cap, truncation and failed-page fixtures preserve available evidence, coverage and reasons without implying completeness | During P2 implementation |
| Production frontend + fake bridge | CB-J01/02/03/05 success and controlled failure/order cases; CB-J04 displays P1's deny/error outcomes; no real AWS/process work | During P2 implementation |
| Browser interaction review | Keyboard/focus, both themes, compact layouts, enlarged text and honest controls | During P2 implementation |
| Native packaged app | Real filesystem paths, restart persistence, OS/webview focus, clipboard, font/layout and complete journey behavior | P5 on each declared OS/architecture |

Record assertions and observed outcomes against journey/work IDs; do not replace failed paths with demo-only coverage. P1 owns execution safety and logical cancellation; P2 owns presentation plus existing partial/error metadata propagation; P3 owns remote query cleanup, progressive-work extensions and their measurements. P2's synthetic cleanup-state rendering is an interface check, not a completed query-cancellation claim.

P2 implementation is complete only when the scoped behavior and automated evidence above pass, retained beta controls are truthful, and remaining native checks are explicitly handed to P5. Any context leak, false save success, hidden partial failure, or inaccessible critical recovery action keeps the affected unit open.

No implementation, test execution, build, dependency installation, native-device result, or publication is recorded by this planning document.
