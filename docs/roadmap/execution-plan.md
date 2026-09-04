# Cloud Burrito — From working tool to public beta

**P1 and P2 complete locally. P3-01–02 complete locally; P3-03 is next. Device tests deferred.**

Decision reference: 2026-09-03. Keep Rust + Tauri. P1-01–03 are committed locally in `a879851`, with their historical results in [P1-01](p1-01-evidence.md), [P1-02](p1-02-evidence.md), and [P1-03 evidence](p1-03-evidence.md). P1-04 is locally implemented and validated; [P1-04 evidence](p1-04-evidence.md) records the results for its separately authorized local commit. No push or live AWS work is included. Laptop checks remain deferred, and no release state has changed.

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
| 4 | P1-04: constrain the CLI child — complete locally | Exact verified credential handoff; isolated child environment; streaming caps; supervised direct-child cleanup and visible cleanup failure |
| 5 | P1-05 and P1-06 complete locally: bind results, validate commands, render safely and report audit outcomes | Late results cannot cross tile/detail/selector owners; diagnostics accurately distinguish outcomes |
| 6 | P2-01/02: durable storage, defaults, theme and save feedback | Failed saves remain failures; accepted settings survive reopen |
| 7 | P2-03/04: first run, recovery and result states | Missing prerequisites, stale data and partial failures are visible and recoverable |
| 8 | P2-05/06/07: connected investigation, honest beta surface and keyboard use | Synthetic flagship journey works; unknown relationships stay explicit |
| 9 | P3-01: record the baseline before optimizing each path | Comparable workload/environment and raw measurements exist |
| 10 | P3-02/03/04: scheduler, progressive results and query cleanup | Work stays within budgets; cancellation and remote cleanup are distinguished |
| 11 | P3-05/06/07/08: bounded caches, CLI aggregate memory, audit and rendering | No identity leak, unbounded retained work or misleading performance claim |
| 12 | P4 work packages in their documented order | Same-candidate unsigned artifacts, checksums and reviewed setup instructions |

A dependent unit may consume a reviewed interface fixture before a whole phase finishes, but it cannot claim integration success until the real dependency passes. P2 defines error/partial-state contracts; P3 extends their producers. P1 supplies the process byte limits; P3 profiles their aggregate memory cost. This avoids implementing the same boundary twice.

## Current implementation progress

**P1 and all seven P2 units are complete locally. P3-01–02 complete locally; P3-03 is next.**

[P3-01 evidence](p3-01-evidence.md) records the unchanged synthetic baseline, 249 Rust and 17 Node regressions, and 270 successful measured browser trials. No native or live AWS performance is claimed.

[P2-07 evidence](p2-07-evidence.md): keyboard/focus, both themes, compact windows and enlargement pass the final 248 Rust, 17 Node, 94 browser and 16 Python helper cases. Repository gates pass with the documented cached-advisory and scanner-cleanup limitations. All seven P2 units are separately committed locally.

[P2-06 evidence](p2-06-evidence.md): honest beta controls and clipboard outcomes pass 17 Node and 23 focused browser cases.

[P2-05 evidence](p2-05-evidence.md): evidence-bound investigation and exact same-context ownership pass 248 Rust, 17 Node and 84 browser cases.

[P2-04 evidence](p2-04-evidence.md): explicit result states, retained partial evidence and coverage pass 235 Rust, 17 Node and 75 browser cases.

[P2-03 evidence](p2-03-evidence.md): fresh discovery, visible recovery and optional CLI checks pass 200 Rust, 17 Node, a full 60-case browser run and 14 final focused recovery cases.

[P2-02 evidence](p2-02-evidence.md): 186 Rust, 15 Node and 48 browser cases validate authoritative defaults/regions, full-form settings, persisted theme and credential-context invalidation.

[P2-01 evidence](p2-01-evidence.md): 177 Rust, 15 Node and 40 browser tests pass; truthful storage, controlled write faults, preserved drafts and explicit recovery. The user authorized all P2 units sequentially with a separate local commit per unit, without push or AWS. No Windows or Ubuntu input was required for these offline implementation slices.

P1-01 introduced explicit test storage and fake identity/process boundaries, isolated the pinned-context persistence test, and demonstrated delayed completions and zero-spawn denial. Its historical checks remain in [P1-01 evidence](p1-01-evidence.md).

P1-02 added exact operation/argument denial. Its forbidden-operation regression recorded one fake process call before the fix; the historical results remain in [P1-02 evidence](p1-02-evidence.md).

P1-03 adds explicit legacy/named-session SSO configuration, supported named-session token renewal, frozen credentials shared by STS and resources, account/principal/expiry verification, refresh and configuration invalidation, latest-attempt connection state, independent pinned contexts, and frontend auth-status guards. **97 Rust library tests and 5 Node production-handler tests pass.** Expired legacy tokens require external SSO login. Live AWS/provider behavior, native GUI and Windows/Ubuntu validation were not exercised; see [P1-03 evidence](p1-03-evidence.md).

**P1-04 complete locally.** The implementation replaces the temporary desktop CLI block with exact frozen STS credentials and an isolated environment/home/cwd/null AWS configuration. Executable support covers native installers plus a narrow Unix absolute-Python wrapper, launched with its validated native interpreter at the original absolute virtual-environment path and `[-I, canonical aws script, validated argv]`. Shell, environment-relative and batch wrappers are unsupported. AWS CLI v2 version and publisher trust remain local-installation requirements; no version probe was run.

The child execution deadline is the earlier of 30 seconds or credential expiry; stdout/stderr have 2 MiB/256 KiB streaming caps. Affected-context invalidation is monitored every 100 ms, and cancellation/caller drop retain direct-child termination/reap ownership. Cleanup can outlast the execution deadline while awaiting OS-confirmed exit; only an empty isolated directory is removed, so a nonempty directory can remain. Raw stderr is withheld from the UI and exact credential values are redacted from runner errors. Cleanup failure survives a superseded context as the stable `CliCleanupFailed` UI/audit error; a controlled command regression covers that ordering.

At the P1-04 boundary, the full Rust library suite passed **128 tests**, with no failed, ignored or filtered tests; **5 Node production-handler tests** and **13 release-helper tests** also pass. The repository security check exits successfully. Its cached dependency audit retains 20 allowed warnings and does not certify fresh advisories; see [P1-04 evidence](p1-04-evidence.md) for scoped privacy results and scanner limitations. These checks cannot establish whole-process-tree termination, actual AWS CLI execution or native OS cleanup. P1-04 is recorded in local commit `43a168c`. **P1-05 is also complete locally:** see [its evidence](p1-05-evidence.md) for tile/detail/selector ownership and synthetic browser checks. **P1-06 is complete locally:** **163 Rust, 15 Node and 33 browser tests pass**; [its evidence](p1-06-evidence.md) records strict input, hostile rendering, audit lifecycle, diagnostic failures and scoped scanner results. **Next: P3-03**. [Latest evidence](p3-02-evidence.md). Device and live-provider acceptance remain attached to their later gates.

## Validation ledger

Existing P0 results remain recorded in [phase-0.md](phase-0.md#baseline-recorded-in-this-phase). P1-01–06 supply isolated boundaries, capability/argument denial, verified-context, constrained-CLI, result-ownership and diagnostic evidence; full journey, security and device acceptance remain pending as described below.

| ID | Pending evidence | Responsible stage | Dependency / effect of deferral |
| --- | --- | --- | --- |
| V01 | Windows and Ubuntu OS, architecture, tool/runtime inventory | User + P0 device checks | Needed to choose the actual support matrix; no planning blocker |
| V02 | Early Windows/Ubuntu source build and controlled native launch | P0, with platform fixes in P4 | Reveals compatibility gaps; no support claim until evidence exists |
| V03 | Native launch, CPU and memory baseline before relevant optimization | P0 / P3-01 | Required for native before/after claims; synthetic work can be designed now |
| V04 | Native child behavior, save-failure and broader ownership cases | P1/P2 implementation | P1-01–04 cases pass locally; native child/process-tree behavior, save failures remain unverified; P1-05 adds deterministic owner-lifetime coverage |
| V05 | Broader production frontend journeys through a synthetic bridge | P1/P2 implementation | P1-05 adds 15 Node production-handler tests and deterministic browser ownership journeys; P1-06 adds hostile rendering, audit warning and rejection journeys. Broader P2/native GUI acceptance remains open |
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
| P1-01–03 local implementation and synthetic tests | Complete locally in `a879851` | Historical evidence remains unchanged |
| P1-04–06 implementation, offline checks and separate local commits | Complete locally; see each evidence file | Native evidence remains pending; no push, live AWS or publication |
| Unsigned, identity-free distribution | Repository requirement | No publisher certificates, Apple Developer IDs, notarization or personal publisher metadata |
| Exact approved CLI operations and validated arguments | P1-02 and P1-04 validated locally | The same 18 resource-read schemas remain; desktop CLI is re-enabled with frozen verified credentials and isolated child execution |
| Explicit legacy and named-session SSO support | Implemented and locally validated in P1-03 | Frozen STS-verified credentials serve resource calls; unsupported credential/endpoint indirection fails before provider work; live renewal and provider behavior remain unverified |
| Retain the current frontend; hide unfinished AI/global-search controls in beta | Proposed P2 design | Focus effort on a complete investigation workflow |
| Numeric performance/resource budgets | Provisional P3 experiments | Adjust from evidence before adopting as release gates; none achieved yet |
| Windows NSIS; Ubuntu deb plus secondary AppImage; existing macOS DMG/ZIP | Proposed P4 packaging | OS versions/architectures remain provisional until device inventory and build evidence |

P0 support evidence may change package targets, prerequisite instructions or a proposed default. Record the reason and affected work IDs instead of silently expanding the scope. A new AWS service, credential mechanism or executable capability needs its own reviewed contract.

## How completion is recorded

For each work ID, record: source revision/diff, intended behavior, acceptance cases and observed results, remaining device evidence, and any compatibility change. Use distinct states: **planned → implementing → automated checks passed → native evidence pending/verified**. “Plan complete” never means “feature shipped.”

Phase completion uses the exit criteria in that phase's document. A known context leak, forbidden execution, false save success or lost/hidden failure keeps its unit open. A pending native result remains attached to the candidate and blocks the corresponding platform/performance claim.

P4 hands a candidate to P5. P5 establishes device evidence. P6 packages the portfolio explanation, recorded decisions, demo and release evidence. A GitHub action, push, tag, draft release or public publication remains a later explicit action governed by [AGENTS.md](../../AGENTS.md). The authorized local P1 commits do not authorize any of those remote actions.
