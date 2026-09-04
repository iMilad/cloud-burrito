# P0 — Establish the truth

Status: **In progress; local source inventory and development baseline recorded. Device checks deferred while planning P1–P4.**

Reviewed 2026-09-03 against `0.2.9` at `095d1ad`. This phase adds planning evidence, not application behavior. Windows/Ubuntu attempts, native runtime measurements and the final support matrix remain open.

Subsequent implementation: [P1-01](p1-01-evidence.md) isolated the formerly excluded settings-reading test and removed HOME mutation from persistence tests. Its full 63-test run has no filtered tests. The baseline below remains the historical P0 snapshot, including its original limitations and command; use the P1-01 command for the current tree.

## Purpose and concrete outputs

Agree what the beta must do, identify every first-party execution path, define observable acceptance, and expose platform blockers before implementation grows.

| Output | Current state |
| --- | --- |
| Beta scope and canonical P0–P6 sequence | Recorded in [roadmap](README.md) |
| Tauri/AWS/widget operation inventory | Recorded: 14 commands, 21 registry entries, 13 dispatch paths; provider and CLI proof limits explicit |
| Five release-critical journeys | Contracts below; none verified end to end |
| Local development baseline | Static checks, helper tests, 45 Rust tests and Clippy pass; limitations below |
| Native startup/memory/workflow baseline | Not measured |
| Windows/Ubuntu inventory and build/launch spike | Awaiting device information and guided attempts |
| Frozen beta support matrix | Proposed architectures below; version/architecture decisions remain open |

## Acceptance fixtures

Use synthetic Demo A/Demo B identities and fictional resources. Exercise production frontend paths through a fake Tauri bridge with controllable delays and failures. Rust identity, SDK, filesystem and process tests need explicit injected boundaries. Unexpected real AWS or CLI execution must fail closed in automated acceptance tests.

The following are target contracts, not descriptions of behavior already proved. Preserve their IDs in future tests and device evidence.

### CB-J01 — First launch and verified identity

- **Given** missing or malformed AWS configuration, **when** the app starts, **then** it shows the configuration problem and a Settings/retry route; account-dependent work remains unavailable.
- **Given** a configured synthetic profile and matching STS account, **when** the user connects, **then** the UI transitions from verification to the verified profile/account/region before enabling work.
- **Given** expired credentials or an identity mismatch, **when** connection fails, **then** the recovery action is visible and no mismatched account is accepted. An absent optional CLI executable blocks CLI features while valid SDK-backed work can continue.

Current evidence: profile discovery and auth UI exist, but `aws_set_account` accepts credential resolution before account verification. The later STS path does not compare the returned account against the claimed account. The browser startup test covers demo mode only.

Sources: [context.rs](../../src-tauri/src/aws/context.rs), [commands.rs](../../src-tauri/src/commands.rs), [config_file.rs](../../src-tauri/src/aws/config_file.rs), [browser tests](../../tests/frontend/browser-mode.spec.js) at line 20.

### CB-J02 — Context switching and pinned comparison

- **Given** Demo A data and a pending A request, **when** the user selects Demo B and A completes late, **then** inherited A data is cleared during verification and cannot overwrite B's active context or results. Repeat with reversed authentication completion order and region changes.
- **Given** pinned Demo A and Demo B widgets/cards, **when** the default context changes or a detail view is expanded, **then** each item and nested request retains its own verified context.
- **Given** a failed B authentication, **when** a switch is attempted, **then** the failure is explicit; A data never appears as verified B data.

Current evidence: frontend selection IDs protect some selection responses, and some nested routes snapshot context. Backend credential completion can still overwrite newer context; many widget results lack generation guards. Three Rust context-selection tests exist; they do not test authentication order. Two passed in this baseline and one was excluded to avoid reading personal settings. Browser region persistence is not account-switch coverage.

Sources: [commands.rs](../../src-tauri/src/commands.rs), particularly active writes near line 203 and tests near line 335; [app.js](../../frontend/app.js), selection near line 930 and nested pipeline handling near line 1534.

### CB-J03 — Pipeline → Build → Stack → Logs

- **Given** a synthetic failed execution with known build, stack and log evidence, **when** the user follows the investigation, **then** expected evidence appears at every step under the same verified account/region.
- **Given** an unknown association, **when** the user continues, **then** an explicit lookup/selection preserves context and states the relationship is unknown rather than inventing a link.
- **Given** a denied request, later-page failure or log limit, **when** the detail opens, **then** empty, denied, failed and partial/truncated results are distinct and retry or an appropriate console link is available.

Current evidence: Pipeline → execution detail → inline CodeBuild logs is implemented. Stack resources/events and CloudWatch views exist separately. The full connected route is not proved. The live resource-lookup handler retains a replaced input node. The browser CloudFormation test covers mock resources/events; no production flagship-chain acceptance test exists.

Sources: [app.js](../../frontend/app.js), pipeline route near line 1534 and lookup near line 3372; [codebuild_log.rs](../../src-tauri/src/widgets/codebuild_log.rs); [browser tests](../../tests/frontend/browser-mode.spec.js) at line 81.

### CB-J04 — Approved CLI execution

- **Given** verified context, an exact approved operation and a narrowing policy, **when** a fake executable returns synthetic JSON, **then** the displayed result and audit outcome identify that context and operation without credentials.
- **Given** wildcard user policy, **when** a forbidden/unknown operation such as `BatchDeleteImage` or a prohibited identity/endpoint/TLS override is supplied, **then** it is rejected before process creation. The UI must not suggest granting a forbidden action. No shell is invoked.
- **Given** a missing executable, oversized stdout/stderr, invalid JSON, cancellation or timeout, **when** execution fails, **then** output and memory are bounded, the child terminates where appropriate, and the UI presents a current actionable failure.

Current evidence: argv execution, selected argument rejection, timeout and JSON rendering exist. The CLI deliberately bypasses the finite registry; `Batch*` safety is heuristic. Region/endpoint/TLS and inherited environment remain insufficiently constrained. Size is checked after buffering. Existing Rust tests cover parsing/rendering and selected policy decisions, not these full execution guarantees; one currently expects a region override to pass.

Sources: [aws_cli.rs](../../src-tauri/src/widgets/aws_cli.rs), [guard.rs](../../src-tauri/src/aws/guard.rs), [policy.rs](../../src-tauri/src/aws/policy.rs).

### CB-J05 — Durable settings and honest recovery

- **Given** valid settings, theme and layout/pinned choices, **when** the user saves and restarts, **then** accepted values return and success was reported only after persistence succeeded.
- **Given** invalid input or a failing filesystem, **when** saving fails, **then** the error is visible, previous durable settings remain intact, unsaved input is recoverable, and retry is possible.
- **Given** a previous result, **when** refresh fails or returns partial data, **then** the state is explicit; retained data keeps its original context and freshness label. Recovery replaces it once and clears the failure.

Current evidence: region/layout browser persistence and native settings/dashboard storage exist. Theme is not durable and native save paths suppress filesystem failures. Existing tests cover successful normalization/filtering and browser persistence, not injected save failures or full recovery.

Sources: [settings.rs](../../src-tauri/src/settings.rs), [dashboard.rs](../../src-tauri/src/dashboard.rs), [app.js](../../frontend/app.js), [browser tests](../../tests/frontend/browser-mode.spec.js) at lines 42 and 356.

## Baseline recorded in this phase

| Check | Result | What it establishes |
| --- | --- | --- |
| Version consistency | Pass: 0.2.9 | Release metadata agrees |
| Tauri binding checker | Pass: 14 commands | Registered command names align |
| Existing tracked-source privacy rules | Pass: 145 files | No findings under these rules; not a full history/secret audit |
| JavaScript/shell syntax and whitespace | Pass | Syntax/static hygiene |
| Python release helper tests | 13 passed | Disposable local fixtures; Git transport restricted to local files, global hooks/config disabled |
| Rust formatting | Pass | Current formatting matches rustfmt |
| Rust library tests, locked/offline, single-threaded | 45 passed, 0 failed, 1 filtered out | Existing unit coverage; no live SDK or CLI execution |
| Rust Clippy, locked/offline, warnings denied | Pass | Current lint baseline |
| Frontend suite discovery | 6 tests found; not executed | Five demo tests and one production CodeArtifact path with a fake Tauri bridge |
| Native app/install tests and full journey acceptance | Not run | No platform or end-to-end acceptance claim |

The filtered Rust test is `commands::tests::pinned_context_resolves_without_active_topbar_context`: it reads the user's app settings through `settings::load()` without an injected path. No skip was added to repository code. Five other persistence tests use their existing temporary directories and internal HOME restoration on macOS; Windows isolation remains unverified. Replace environment-dependent test setup with explicit filesystem injection as part of trust-boundary test infrastructure.

The reviewed Rust tests needed an offline Cargo cache unpack outside the filesystem sandbox; the approved retry used `--offline`. No dependency download or AWS request occurred. Existing tests being green does not prove the known execution/context gaps are covered.

Reproducible Rust baseline from `src-tauri`:

```sh
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  -u AWS_PROFILE -u AWS_DEFAULT_PROFILE AWS_EC2_METADATA_DISABLED=true \
  cargo test -p cloud-burrito --locked --offline --lib -- \
  --test-threads=1 \
  --skip commands::tests::pinned_context_resolves_without_active_topbar_context
cargo clippy -p cloud-burrito --locked --offline -- -D warnings
```

## Environment and performance evidence

Local development host: macOS 26.6.2, ARM64; Xcode SDK 26.5; Rust/Cargo 1.91.1; Tauri CLI 2.11.4. Only `aarch64-apple-darwin` is installed. Node 24.14.1 / npm 11.11.0 differs from CI Node 22 and the repository's npm 10.9.8 pin. Record that difference rather than treating this as the CI environment.

The installed Playwright package expects Chromium revision 1228, which is absent. Browser-test execution is pending that prerequisite. No dependency/browser installation was performed in P0.

| Product metric | Baseline status / method |
| --- | --- |
| Native cold launch to usable window | Unmeasured; record OS/architecture/build mode and repeated launches |
| Idle CPU and resident memory | Unmeasured; record a stable idle interval, then repeated refresh cycles |
| First result and total/p95 refresh | Unmeasured; use a fixed synthetic dataset and repeat count; separate successful/failed/cancelled runs |
| AWS call count | Source estimate only: 50 CodeArtifact packages with versions typically mean 1 list + 2 requests per package, about 101 before retries/auth |
| Cancellation, query cleanup and large output | Unmeasured; deterministic delayed/error/oversized fixtures required |

Test/compile elapsed time is not an application performance benchmark. Record before/after measurements using the same workload and environment in P3.

## Platform decision still open

| Platform | Proposed target | P0 evidence |
| --- | --- | --- |
| macOS | ARM64 initially; Intel packaging already configured | Local library compile/tests pass on ARM64; native launch/install and Intel acceptance unverified |
| Windows | x64, subject to device inventory | OS/build/architecture and native build/launch attempt pending |
| Ubuntu | x64, explicit supported release/baseline | OS/glibc/WebKitGTK and native build/launch attempt pending |

Three laptops establish evidence only for their actual OS versions and architectures. One ARM64 Mac does not establish Intel macOS support. Freeze claims after inventory and build/launch attempts; obtain additional evidence or narrow the matrix for untested targets.

Known blockers: macOS-only bundle configuration and release jobs; `.app` assumptions in the build helper; Windows discovery looks for `aws` rather than `aws.exe`; Windows persistence-test isolation is unproved. See [device checks](device-checks.md).

## Exit gate and the first P1 work package

- [x] Scope and non-goals recorded.
- [x] First-party commands, finite registry and open/provider surfaces classified.
- [x] Five journey contracts with coverage gaps recorded.
- [x] Available local development checks and limitations recorded.
- [ ] Native timing/memory baseline collected in a controlled environment.
- [ ] Windows and Ubuntu device inventory received and build/launch attempts documented.
- [ ] Supported OS/architecture matrix frozen.

P0 remains open until the unchecked gates have evidence. The user has deferred laptop testing; this does not block completion of the P1–P4 plans or future isolated source work. Resume Windows inventory and then Ubuntu when available. Capture the relevant unchanged runtime baseline before a performance comparison, and freeze the support matrix before finalizing platform claims.

First P1 slice: establish injected filesystem/credential/process test boundaries, then enforce an exact operation/argument allowlist with denial-before-spawn tests. Follow with STS-verified active and pinned contexts and response-generation tests covering A/B completion order. Use CB-J04 and CB-J01/02 as acceptance contracts. Query control and credential acquisition remain separately classified; they are not ordinary resource reads.
