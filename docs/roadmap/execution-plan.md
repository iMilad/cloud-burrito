# Cloud Burrito — From working tool to public beta

**P1-01 through P1-03 complete locally. P1 remains in progress; P1-04 is next. P2–P4 remain planned. Device tests deferred.**

Decision reference: 2026-09-03. Keep Rust + Tauri. The user subsequently authorized P1-01 through P1-03 implementation with local synthetic tests, without live AWS execution. Changes and checks are recorded in [P1-01](p1-01-evidence.md), [P1-02](p1-02-evidence.md), and [P1-03 evidence](p1-03-evidence.md). Laptop checks remain deferred, and no release state has changed.

## The product we are building

An engineer installs Cloud Burrito, connects a verified AWS account, follows a failed deployment through its evidence, and compares accounts while keeping the context visible. The portfolio story is the combination of workflow design, defensible security boundaries and measured product quality.

| Stage | What the user should notice | Why this comes next |
| --- | --- | --- |
| [P1 · Trust](phase-1.md) | “I know which account this result belongs to and what this app can execute.” | Every workflow and cache depends on correct execution and identity |
| [P2 · Dependability](phase-2.md) | “I can investigate a problem, save my setup and recover from errors.” | Polish the existing core before optimizing or distributing it |
| [P3 · Performance](phase-3.md) | “Useful results arrive promptly; refresh and cancellation remain responsive.” | Measure real costs and bound work without sacrificing correctness |
| [P4 · Native artifacts](phase-4.md) | “There is a clearly labelled package and setup path for my platform.” | Produce candidates that can enter repeatable device acceptance |

The [roadmap overview](README.md#canonical-phases) contains the visual sequence. Each phase document carries source evidence, ordered work IDs, acceptance cases and the completion gate.

## Execution order

Use one work ID per reviewable change where practical. A work ID may need multiple small diffs; these are dependency units, not calendar estimates. Re-estimate after P1 establishes the test boundaries rather than promising dates before the first implementation slice.

| Order | Work | Completion evidence before moving on |
| --- | --- | --- |
| 1 | P1-01: inject storage, identity, transport and process boundaries — complete locally | Critical tests run without personal files, credentials or real AWS/CLI |
| 2 | P1-02: exact capability and argument rules — complete locally | Forbidden operation stays denied under wildcard policy before execution |
| 3 | P1-03: verify active/pinned identity and isolate configuration — complete locally | Synthetic identity mismatch, ordering, configuration/refresh and auth-status cases pass |
| 4 | P1-04: constrain the CLI child — next | Verified credential handoff; controlled environment; bounded streams; timeout/cancel reaps child |
| 5 | P1-05/06: bind results, validate commands, render safely and report audit outcomes | Late results cannot cross contexts; diagnostics accurately distinguish outcomes |
| 6 | P2-01/02: durable storage, defaults, theme and save feedback | Failed saves remain failures; accepted settings survive reopen |
| 7 | P2-03/04: first run, recovery and result states | Missing prerequisites, stale data and partial failures are visible and recoverable |
| 8 | P2-05/06/07: connected investigation, honest beta surface and keyboard use | Synthetic flagship journey works; unknown relationships stay explicit |
| 9 | P3-01: record the baseline before optimizing each path | Comparable workload/environment and raw measurements exist |
| 10 | P3-02/03/04: scheduler, progressive results and query cleanup | Work stays within budgets; cancellation and remote cleanup are distinguished |
| 11 | P3-05/06/07/08: bounded caches, CLI aggregate memory, audit and rendering | No identity leak, unbounded retained work or misleading performance claim |
| 12 | P4 work packages in their documented order | Same-candidate unsigned artifacts, checksums and reviewed setup instructions |

A dependent unit may consume a reviewed interface fixture before a whole phase finishes, but it cannot claim integration success until the real dependency passes. P2 defines error/partial-state contracts; P3 extends their producers. P1 supplies the process byte limits; P3 profiles their aggregate memory cost. This avoids implementing the same boundary twice.

## Current implementation progress

**P1-01–03 are complete locally. Full P1 is not complete.** No Windows or Ubuntu input was required for these offline implementation slices.

P1-01 introduced explicit test storage and fake identity/process boundaries, isolated the pinned-context persistence test, and demonstrated delayed completions and zero-spawn denial. Its historical checks remain in [P1-01 evidence](p1-01-evidence.md).

P1-02 added exact operation/argument denial. Its forbidden-operation regression recorded one fake process call before the fix; the historical results remain in [P1-02 evidence](p1-02-evidence.md).

P1-03 adds explicit legacy/named-session SSO configuration, supported named-session token renewal, frozen credentials shared by STS and resources, account/principal/expiry verification, refresh and configuration invalidation, latest-attempt connection state, independent pinned contexts, and frontend auth-status guards. **97 Rust library tests and 5 Node production-handler tests pass.** Expired legacy tokens require external SSO login. Live AWS/provider behavior, native GUI and Windows/Ubuntu validation were not exercised; see [P1-03 evidence](p1-03-evidence.md).

**Next: P1-04.** The desktop CLI returns `CliContextUnavailable` until the verified credential snapshot can be handed to a child with a controlled environment, bounded streams and supervised termination. P1-05 still owns broader tile/detail/selector lifetime and result ownership. Run meaningful offline checks with each unit; device and live-provider acceptance remain attached to their later gates.

## Validation ledger

Existing P0 results remain recorded in [phase-0.md](phase-0.md#baseline-recorded-in-this-phase). P1-01–03 supply isolated boundaries, capability/argument denial and verified-context evidence; full journey, security and device acceptance remain pending as described below.

| ID | Pending evidence | Responsible stage | Dependency / effect of deferral |
| --- | --- | --- | --- |
| V01 | Windows and Ubuntu OS, architecture, tool/runtime inventory | User + P0 device checks | Needed to choose the actual support matrix; no planning blocker |
| V02 | Early Windows/Ubuntu source build and controlled native launch | P0, with platform fixes in P4 | Reveals compatibility gaps; no support claim until evidence exists |
| V03 | Native launch, CPU and memory baseline before relevant optimization | P0 / P3-01 | Required for native before/after claims; synthetic work can be designed now |
| V04 | Remaining process, save-failure and broader result-generation cases | P1/P2 implementation | P1-01 boundary, P1-02 capability and P1-03 identity/configuration/connection-ordering cases pass locally; process supervision, save failures and broader tile ownership remain pending |
| V05 | Broader production frontend journeys through a synthetic bridge | P1/P2 implementation | P1-03 has 5 passing Node production-handler tests for auth-status behavior; these do not establish full browser, rendering or native GUI acceptance |
| V06 | Scheduler, cancellation, cache and output stress fixtures | P3 implementation | Confirms budgets and preserves P1/P2 contracts |
| V07 | Native build, artifact inventory, checksums and privacy inspection | P4 implementation | A created package is a candidate, not an installation pass |
| V08 | Fresh install, first run, restart, upgrade, uninstall and journeys on each supported OS/architecture | User + P5 guided acceptance | Required before claiming that exact platform supported |
| V09 | Minimal integration check against the explicitly chosen test AWS context | Later controlled validation | Synthetic tests cannot establish real provider/network behavior; scope separately before live use |
| V10 | Fresh dependency advisory, license, history/privacy and final candidate review | P4/P6 release preparation | Prior scans do not establish current public-release readiness |

Device checks will resume sequentially with one action, expected result and recorded outcome at a time. No current user response is needed. Any live AWS validation must use an explicitly selected test context and bounded operations; query execution may incur cost. No live AWS action is authorized by this planning document.

## Decisions and defaults

| Decision | Status | Consequence |
| --- | --- | --- |
| Keep Rust + Tauri | Agreed | Improve current architecture; no Go/Wails migration |
| Finish four phase plans before device testing | Agreed | Continue planning now; device evidence stays pending |
| P1-01–03 local implementation and synthetic tests authorized | Complete locally | P1-04 is next; live AWS and publication remain outside the completed scope |
| Unsigned, identity-free distribution | Repository requirement | No publisher certificates, Apple Developer IDs, notarization or personal publisher metadata |
| Exact approved CLI operations and validated arguments | Implemented and locally validated in P1-02 | The 18 reviewed resource-read schemas remain; P1-03 temporarily blocks desktop execution until P1-04 provides verified credentials and child isolation |
| Explicit legacy and named-session SSO support | Implemented and locally validated in P1-03 | Frozen STS-verified credentials serve resource calls; unsupported credential/endpoint indirection fails before provider work; live renewal and provider behavior remain unverified |
| Retain the current frontend; hide unfinished AI/global-search controls in beta | Proposed P2 design | Focus effort on a complete investigation workflow |
| Numeric performance/resource budgets | Provisional P3 experiments | Adjust from evidence before adopting as release gates; none achieved yet |
| Windows NSIS; Ubuntu deb plus secondary AppImage; existing macOS DMG/ZIP | Proposed P4 packaging | OS versions/architectures remain provisional until device inventory and build evidence |

P0 support evidence may change package targets, prerequisite instructions or a proposed default. Record the reason and affected work IDs instead of silently expanding the scope. A new AWS service, credential mechanism or executable capability needs its own reviewed contract.

## How completion is recorded

For each work ID, record: source revision/diff, intended behavior, acceptance cases and observed results, remaining device evidence, and any compatibility change. Use distinct states: **planned → implementing → automated checks passed → native evidence pending/verified**. “Plan complete” never means “feature shipped.”

Phase completion uses the exit criteria in that phase's document. A known context leak, forbidden execution, false save success or lost/hidden failure keeps its unit open. A pending native result remains attached to the candidate and blocks the corresponding platform/performance claim.

P4 hands a candidate to P5. P5 establishes device evidence. P6 packages the portfolio explanation, recorded decisions, demo and release evidence. A GitHub action, tag, draft release or public publication remains a later explicit action governed by [AGENTS.md](../../AGENTS.md); P1-01–03 performed none of them.
