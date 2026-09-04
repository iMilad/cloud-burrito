# P3 — Earn the performance claim

Implementation status: **P3-01–05 complete locally; P3-06 is next. Native performance metrics remain unmeasured.**

Planning source review: `0.2.9`, 2026-09-03. [P3-01 evidence](p3-01-evidence.md) records the unchanged synthetic baseline at `6c0377e`, its method, raw measurements and limitations. The remaining work packages below define implementation acceptance; their targets are not product claims.

## Decision and dependencies

Keep Rust + Tauri and the existing in-process SDK architecture. The source exposes avoidable serial requests, unbounded aggregate work and repeated rendering; it does not establish that another language would improve user-visible latency.

- P0 native measurements remain pending. That does not block this plan; capture a comparable baseline before optimizing each path or claiming an improvement.
- P1 owns verified identity, request generations, exact operation/argument controls and CLI process safety. Preserve those contracts through scheduling, caching and cancellation.
- P2 owns durable settings, explicit partial/error/freshness states and dependable investigation journeys. Progressive updates must use those same states.
- P4 provides reproducible unsigned packages for measurement. P5 owns acceptance on actual supported devices; no laptop availability is assumed here.
- P6 may publish only measured results with environment, dataset and method attached. None of the targets below is a product claim.

## Ordered work packages

### P3-01 — Capture a reproducible baseline

**Outcome:** distinguish local startup/rendering cost from credential resolution, network waits and AWS execution time.

- Record source revision, release/debug profile, target architecture, toolchain, OS/webview version, machine class, power mode and fixture revision.
- Measure launch-to-interactive separately from connect-to-verified-identity, first useful widget result and complete widget result.
- Count logical operations, actual attempts/retries, pages, bytes, queued/active work and cancellations; do not log credentials, query text or returned customer data.
- Measure backend and webview process memory together, idle CPU, peak memory, and retained memory after repeated refresh/close cycles.
- Capture the unchanged path first under the protocol below. Instrumentation may precede optimization; a missing native baseline leaves native comparisons pending.

**Acceptance:** the same synthetic scenario can be replayed with the same build mode and recorded parameters; failures and partial results remain in the report rather than being discarded.

Sources: [commands.rs, lines 408–419](../../src-tauri/src/commands.rs#L408-L419); [context.rs, lines 44–65](../../src-tauri/src/aws/context.rs#L44-L65); [app.js, lines 343–352](../../frontend/app.js#L343-L352).

### P3-02 — Bound request volume and concurrency

**Outcome:** refreshing many tiles remains responsive and cannot multiply work without a defined limit.

- Add a shared scheduler with account/region/service budgets plus an app-wide ceiling; treat CLI children and active Logs Insights queries as separately limited resources.
- Start experiments with four ordinary requests per context/service and eight app-wide, two CLI children, and two active Insights queries. These are provisional local limits, not AWS quota assertions.
- Bound pending work; coalesce identical refreshes for the same verified context, operation and normalized inputs. Removing one subscriber must not cancel work still needed by another.
- Give interactive detail requests priority over enrichment without starving older work. Respect SDK retries within the same deadline and attempt budget; avoid a second unlimited retry loop.
- Define total request deadlines and page/result/byte ceilings per widget. Preserve continuation or an explicit partial-result explanation when a ceiling is reached.

**Acceptance:** synthetic 50-pin refresh, repeated clicks, delayed pages, repeated pagination tokens and throttled responses never exceed configured ceilings, cross contexts or create an endless queue. Queue cancellation prevents dispatch.

Sources: [app.js, lines 1785–1788](../../frontend/app.js#L1785-L1788), [lines 4237–4240](../../frontend/app.js#L4237-L4240); [cfn_stacks.rs, lines 30–72](../../src-tauri/src/widgets/cfn_stacks.rs#L30-L72); existing bounded scan in [resource_lookup.rs, lines 12–16](../../src-tauri/src/widgets/resource_lookup.rs#L12-L16).

### P3-03 — Deliver CodeArtifact data progressively

**Outcome:** package names become useful before every package has completed enrichment.

Current initial enrichment awaits each package serially. A normal non-empty package adds a version-list and a detail request; this is source-derived request behavior, not a measured duration.

- Return a bounded page of package identities first, then enrich visible/requested rows through P3-02. Use an explicit request/subscription ID if updates cross the IPC boundary.
- Keep stable package keys and ordering while updates arrive; label pending, failed and completed enrichment separately. A failed row must not erase successful rows.
- Retain lazy version-history expansion and its ordered output. Existing concurrency-four history loading is a useful pattern, not an app-wide resource limit.
- Prefer cached or already-returned metadata when sufficient; avoid speculative enrichment of every historical version. Document the exact request budget before and after changes.
- On new inputs, context switch or tile removal, detach obsolete work and reject late updates using P1 request generations.

**Acceptance:** fixtures of 50 and 1,000 packages show their first page before all enrichment completes; delayed and failed details preserve row identity, selection and explicit partial state. Reopening the same detail does not duplicate in-flight work.

Sources: [codeartifact_packages.rs, lines 18–21](../../src-tauri/src/widgets/codeartifact_packages.rs#L18-L21), [lines 87–114](../../src-tauri/src/widgets/codeartifact_packages.rs#L87-L114), [lines 117–190](../../src-tauri/src/widgets/codeartifact_packages.rs#L117-L190), [lines 315–376](../../src-tauri/src/widgets/codeartifact_packages.rs#L315-L376).

### P3-04 — Cancel queries and report cleanup outcomes

**Outcome:** cancelling local work has a defined relationship to remote query execution and scan cost.

- Unify query lifecycle handling for Logs Insights and Errors by Stack: queued, starting, polling, complete, failed, cancelled locally, cleanup pending and cleanup outcome.
- Apply a deadline to each network attempt and the whole lifecycle, including `StartQuery`; checking the clock only between responses is insufficient.
- Cancel on explicit user action, superseding inputs/context and tile removal. If a query ID arrives after cancellation, schedule cleanup under that original verified context.
- Attempt permitted `StopQuery` on timeout/cancellation and polling failure when a query may still be running. Keep cleanup bounded and reserve scheduler capacity so ordinary work cannot starve it.
- Report whether stop was accepted, denied by policy, failed or remains unknown; do not equate stopping local polling with confirmed remote completion. Never widen policy for cleanup.
- Bound time ranges, query counts and discovery pages; show scanned/selected/failed group coverage. Failed groups must not become a successful zero-error chart.

**Acceptance:** a fake service covers slow start, lost start response without query ID, polling failure, denied/failed stop, late completion and context switch. When remote state is unknown, the UI says so and offers recovery without launching a replacement query automatically.

Sources: [logs_insights.rs, lines 14–17](../../src-tauri/src/widgets/logs_insights.rs#L14-L17), [lines 68–134](../../src-tauri/src/widgets/logs_insights.rs#L68-L134); [errors_by_stack.rs, lines 25–89](../../src-tauri/src/widgets/errors_by_stack.rs#L25-L89), [lines 114–145](../../src-tauri/src/widgets/errors_by_stack.rs#L114-L145).

### P3-05 — Cache only within verified identity and policy boundaries

**Outcome:** repeated work is cheaper while account selection and displayed authority remain correct.

- Separate SDK/context reuse, in-flight deduplication and result caching; each needs explicit lifetime and invalidation rules.
- Key contexts by verified identity, profile, region, canonical configuration path and configuration/session generation. Do not use secrets as keys or assume a displayed account ID proves identity.
- Key results by that context plus operation, normalized inputs and authorization generation. Bound entries, total bytes and age; keep response data in memory by default.
- Invalidate affected entries and pending work on configuration/profile/session changes, identity mismatch, logout or policy changes. Re-evaluate policy before serving cached protected data.
- Preserve SDK-managed credential refresh. A cached result never proves a session is currently valid; stale display, where allowed, keeps its original context, timestamp and stale label.
- Deduplication must not erase actual-attempt audit evidence or silently turn refresh into an indefinite cache hit.

**Acceptance:** identical resources in Demo A and Demo B never share results; changing a profile's underlying identity, configuration path, region or deny policy invalidates the right entries. Expiry, eviction and concurrent refresh preserve the configured memory ceiling.

Sources: [state.rs, lines 12–22](../../src-tauri/src/state.rs#L12-L22); [commands.rs, lines 255–284](../../src-tauri/src/commands.rs#L255-L284), [lines 436–439](../../src-tauri/src/commands.rs#L436-L439); [context.rs, lines 21–23](../../src-tauri/src/aws/context.rs#L21-L23).

### P3-06 — Budget CLI memory after the P1 process boundary

**Outcome:** optional CLI work has enforceable resource bounds without weakening command authorization.

- Consume P1-04's authorized process boundary: exact operations/arguments, selected executable, controlled environment, streaming limits and proven terminate/reap behavior. P3 profiles this implementation rather than rebuilding it.
- Inherit P1-04's provisional 2 MiB stdout and 256 KiB stderr caps. Revise them only with measured evidence and renewed P1 boundary acceptance.
- Measure aggregate memory across queued/active children, pipe buffers, decoding, JSON parsing and retained results. Bound table rows, columns and retained raw JSON in addition to per-process output.
- Count CLI jobs against P3-02 and preserve P1 identity attribution. Release permits and result buffers after completion/cancellation; never truncate JSON and present it as a successful complete response.

**Acceptance:** reuse P1-04's fake-process overflow, deadline and cancellation fixtures with concurrent jobs and repeated 50-pin refreshes. Aggregate memory respects the agreed budget, permits are recovered, parsing failures remain accurate, and existing process-safety tests still pass. No real AWS CLI is used.

Source: [aws_cli.rs, lines 21–24](../../src-tauri/src/widgets/aws_cli.rs#L21-L24), [lines 61–100](../../src-tauri/src/widgets/aws_cli.rs#L61-L100). The current output check occurs after collection.

### P3-07 — Make audit cost independent of full history size

**Outcome:** opening the audit panel does not repeatedly load the entire historical file.

- Use bounded reverse/incremental reads with a cursor, maximum record length and response-byte ceiling; handle partial final lines and file replacement explicitly.
- Move potentially slow filesystem work off async request workers. Preserve append ordering and surface persistence failure according to P1/P2 audit contracts; never silently drop events to hit a target.
- Proposed default: five files total, each capped at 10 MiB, including the active file (50 MiB total). Rotate before an append would exceed the cap; the bounded-record rule must fit within that cap. This is a planning proposal; no files are rotated or deleted during planning.
- P2 Settings must show the active limits, storage location and oldest-file expiry behavior before enabling retention or lowering limits, with an explicit save action and a route to preserve existing history. For migration, preview oversized legacy history and require a retention choice before pruning it; disclose expiry instead of promising indefinite history.
- Send new entries or a bounded replacement page to the UI. Pause polling when the panel is hidden and prevent overlapping polls.

**Acceptance:** synthetic 1, 10 and 100 MiB logs, malformed/oversized lines, concurrent appends, rotation and disk-write failure produce bounded reads and accurate ordering/errors. Opening/closing the panel repeatedly does not retain old tables or polling tasks.

Sources: [audit.rs, lines 37–82](../../src-tauri/src/audit.rs#L37-L82); [app.js, lines 534–551](../../frontend/app.js#L534-L551), [lines 609–647](../../frontend/app.js#L609-L647).

### P3-08 — Bound rendering work and preserve interaction

**Outcome:** large result sets do not block filtering, scrolling or context changes.

- Benchmark the existing full-table rendering first. Introduce pagination or virtualization only where the measured row/cell workload requires it; retain keyboard access, selection, expanded details and readable counts.
- Batch progressive updates and avoid rebuilding unchanged rows. Bound displayed cells and long-value previews while offering intentional access to full permitted values.
- Keep text filtering responsive; constrain expensive regex processing and use the P2 invalid-input state. A fast result must not silently change matching semantics.
- Dispose of event listeners, timers and subscriptions when tiles/details close. P1 generation checks must run before every asynchronous DOM update.

**Acceptance:** synthetic 100/1,000/10,000-row tables with long cells and nested details remain cancellable; filtering, keyboard navigation and context labels stay correct during updates. Compare memory after repeated add/remove cycles.

Sources: [app.js, lines 2194–2241](../../frontend/app.js#L2194-L2241), [lines 2264–2288](../../frontend/app.js#L2264-L2288), [lines 5233–5255](../../frontend/app.js#L5233-L5255).

## Synthetic benchmark protocol

1. Inject fake SDK, identity, filesystem and process boundaries; use fictional Demo A/Demo B data. Unexpected network, credential-provider or real executable access fails closed. Do not repurpose the user's real home or application directory.
2. Fix datasets, seeds and delay schedules: immediate, 100 ms and 500 ms responses; selected 5-second outliers; scripted throttling, pagination failure and permanent denial. These are test inputs, not estimates of AWS latency.
3. Cover a six-tile dashboard, 50 pins, the CodeArtifact/table sizes above, and query cancellation at each lifecycle stage. Use isolated disposable directories for audit/process fixtures.
4. For each scenario/build, perform five warm-up runs then at least 30 recorded trials. Report median, empirical p95, maximum, failures, request counts and memory; label this a small-sample benchmark and repeat when results overlap or vary materially.
5. Measure application-cold starts as separate fresh-process trials; define whether filesystem caches are warm. Do not call these machine-cold starts or flush system caches on a user's laptop.
6. Compare the unchanged and changed path on the same device, build mode and fixture. Retain raw measurements and methodology; do not compare a debug baseline with a release candidate.
7. Repeat native startup, rendering, memory and subprocess checks on P4 packages during P5 device validation. Browser/fake-IPC results cannot establish native support or live AWS performance.

## Provisional acceptance targets — all unmeasured

These starting targets are proposals. Confirm or revise them after P3-01 with a written reason before treating them as release gates; never describe them as achieved.

| Measure | Proposed target / contract | Evidence needed |
| --- | --- | --- |
| Immediate UI feedback | p95 within 100 ms of refresh/cancel/filter input | Timestamped synthetic production-path interaction |
| First useful data | p95 within 250 ms after the fake first-page response becomes available | Separately measured IPC/render delay; excludes network wait |
| Ordinary request concurrency | Initial four per context/service, eight app-wide | Fake transport counters including retry attempts |
| CLI / active Insights concurrency | Initial two children / two active queries | Process/query lifecycle counters, cleanup included |
| Cancellation | Local acknowledgement p95 within 100 ms; no new ordinary dispatch for the cancelled subscriber | Queue and UI traces; remote cleanup reported separately |
| Audit tail | p95 within 100 ms for 300 bounded entries from a 100 MiB local fixture | Native filesystem timing and bounded bytes read |
| Startup, idle CPU, total/peak memory | No absolute claim yet; set device-specific budgets after baseline | Release-mode process-tree measurements |
| Retained memory | No sustained upward trend across ten identical refresh/add/remove cycles | Per-cycle settled memory and retained-object evidence |

## Exit evidence and handoff

- Planning is complete when these contracts, dependencies and provisional targets are reviewed; no device is required to complete planning.
- Implementation completion requires P1/P2 regression contracts, deterministic budget/cancellation/cache tests, comparable before/after measurements, and no hidden partial-result or audit failures.
- Native measurements and package-specific performance claims remain open until the relevant P4/P5 evidence exists. A missing measurement is recorded as unmeasured, not passed.
- Handoff to P4/P5: exact revision/build flags, fixture version, provisional/accepted budget decisions, measured results and outstanding limitations. Handoff to P6: only reproducible, supported claims.
