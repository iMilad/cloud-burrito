# P1 — Build the trust boundary

Status: **P1 in progress. P1-01 through P1-03 complete locally; P1-04 is next; P1-05/06 not started. Native validation pending.**

Original source reference: `0.2.9` / `095d1ad`, reviewed 2026-09-03. [P1-01](p1-01-evidence.md), [P1-02](p1-02-evidence.md), and [P1-03 evidence](p1-03-evidence.md) record subsequent local implementation. Remaining work is planned; this document does not certify the current app. Read the [operation inventory](aws-operation-inventory.md) and [journey contracts](phase-0.md) alongside it.

## Outcome

Every supported request runs under a verified context and an exact application capability. A broad user policy cannot enable an unsupported operation. Late requests cannot change the account the user is currently investigating. SDK calls, credential acquisition, query control and subprocess execution have distinct contracts.

P0 device checks may remain pending while this work is planned. Implementation can begin with isolated source-level work when requested; device-specific results remain open until measured. No rewrite or new AWS service expansion is required.

## Current evidence and intended decisions

| Source finding | Current response / remaining work |
| --- | --- |
| Original prefix guard and CLI registry bypass | P1-02 replaced them with exact service/operation/argument records; policy only narrows the supported set |
| Original CLI process inherits its environment | P1-02 rejects unknown/context/endpoint options; P1-03 blocks desktop CLI execution with `CliContextUnavailable` until P1-04 supplies verified credentials and a controlled environment |
| The same runner buffers output before applying its 2 MiB stdout limit | Bound both streams during reading; terminate and reap on limit, timeout or cancellation |
| Original [context.rs](../../src-tauri/src/aws/context.rs) resolved credentials without account verification and changed global config environment | P1-03 uses explicit SSO snapshots and frozen credentials shared by STS verification and resource clients; expiry, refresh and principal checks precede reuse |
| Original [commands.rs](../../src-tauri/src/commands.rs) published the last completion; [state.rs](../../src-tauri/src/state.rs) cached only profile/account/region | P1-03 versions connection attempts, verifies pinned contexts independently, keys caches by verified identity/configuration/session and invalidates changed contexts; broader tile ownership remains P1-05 |
| [widgets/mod.rs](../../src-tauri/src/widgets/mod.rs) records preflight, which may cover multiple subsequent requests | Distinguish permission decisions, provider activity and completed operations |
| The existing [Tauri capability](../../src-tauri/capabilities/default.json) is limited to named commands and a local main window | Preserve that boundary and enforce validation inside each exposed command |

## Ordered work packages

### P1-01 — Make the critical boundaries testable

**Complete locally, 2026-09-03:** explicit storage/runtime boundaries, scripted credential/identity/process responses and a fixed clock are in place. All 63 library tests pass without filtering; see [implementation evidence](p1-01-evidence.md). The contract below is retained as the acceptance reference.

**Depends on:** P0 inventory and CB-J01/02/04. **Touchpoints:** [paths.rs](../../src-tauri/src/paths.rs), settings/dashboard/policy/audit stores, `AwsContext`, `AppState`, and the CLI runner.

- Introduce small injectable interfaces for storage location, credential/identity resolution, process execution and clock/request completion. Keep production defaults unchanged in this slice.
- Replace tests that consult personal settings or change global HOME with temporary stores passed explicitly. Re-enable the previously excluded pinned-context test once isolated.
- Build synthetic Demo A/Demo B responses and controllable completions. An unexpected SDK transport or subprocess call must fail the test immediately.
- **Done when:** existing behavior tests use isolated inputs, the former exclusion is removed, and denial tests can assert zero credential resolutions, network requests and process spawns where the decision precedes those boundaries.

### P1-02 — Define the exact application capability set

**Complete locally, 2026-09-04:** the classified registry, exact CLI mappings/argument schemas and query cleanup precondition are implemented. See [implementation evidence](p1-02-evidence.md). The CLI beta subset is the 18 existing resource reads; query start/stop and credential acquisition are excluded from the CLI table.

**Depends on:** P1-01. **Touchpoints:** `guard.rs`, `policy.rs`, `widgets/mod.rs`, `aws_cli.rs`, command preflights.

- Replace prefix-based authorization with explicit operation records. Keep SDK service/action names, CLI service/subcommand names and any documented IAM mapping separate; string capitalization is not proof of equivalence.
- Classify resource reads, query start/stop and credential acquisition separately. Begin from the existing inventory; every new entry needs a purpose, effect and acceptance case.
- CLI beta scope: only the 18 existing resource reads with reviewed CLI mappings and argument schemas. Unknown services, operations, switches, structured shorthand and values with file-loading semantics are rejected. Credential issuance is not a general CLI table feature.
- User policy remains `explicit deny > matching allow > default deny` within that supported set. Invalid policy fails closed. A forbidden action never receives an “add this permission” hint.
- Treat query start and stop as a coupled workflow capability: prevent starting a query if the app's cleanup capability is disabled; a later AWS cleanup denial remains a visible outcome.
- **Done when:** wildcard policy still rejects forbidden operations before execution, each registered call has a classified record, and allowed requests retain their documented arguments and result shapes.

### P1-03 — Verify and isolate active and pinned identities

**Complete locally, 2026-09-04:** explicit SSO snapshots, account/principal verification, frozen credentials, expiry/refresh fences, latest-attempt connection state, independent pinned contexts, configuration invalidation and frontend auth-poll guards are implemented. The full Rust library suite passes **97 tests**; **5 Node tests** exercise production frontend handlers. See [implementation evidence](p1-03-evidence.md). Live AWS, native GUI and Windows/Ubuntu validation remain pending.

**Depends on:** P1-01/02. **Touchpoints:** `context.rs`, `commands.rs`, `state.rs`, [config_file.rs](../../src-tauri/src/aws/config_file.rs).

- Resolve the chosen profile from an explicit configuration snapshot without changing process-wide AWS variables or loading the default provider chain. Reject ambiguous profile/session aliases and unsupported provider or endpoint settings before provider work.
- Supported source paths: legacy inline SSO with a matching unexpired cached token, and named sessions using the locked SDK's supported token renewal. Expired legacy tokens require external SSO login. External processes, static credentials, role chains, environment fallback and endpoint indirection are unsupported. Source review and synthetic tests do not establish live renewal behavior.
- Verify the selected account and retain the STS principal using the same frozen credentials that resource clients receive. Check expiry and reverify refreshed credentials; a changed principal invalidates the context. Credential resolution alone is insufficient.
- Make connection an attempt-scoped state transition: disconnected → verifying → verified/failed. Only the latest attempt may publish success, failure, timestamps or last-attempt details.
- Verify pinned contexts independently. Cache keys include configuration revision, profile, actual identity, region and session/provider revision. Reconfiguration or provider identity changes invalidate affected contexts and pending results.
- Fence auth-status polling and cached identity-panel updates by selection/request generation. This does not complete P1-05's broader tile, selector and detail-owner lifecycle.
- Keep desktop CLI execution unavailable with `CliContextUnavailable` until P1-04 can hand the verified credentials to an isolated child; SDK widgets continue through their verified contexts.
- **Done when:** mismatched identities never become active, pinned contexts cannot borrow the topbar identity, and an older success or failure cannot replace a newer connection outcome.

STS returns the account and principal associated with the calling credentials and does not require an IAM permission grant for this operation. The app may still apply its own local verification policy. An IAM grant hint is therefore not a general solution to verification failures. [AWS STS reference](https://docs.aws.amazon.com/STS/latest/APIReference/API_GetCallerIdentity.html)

The SDK's default credential chain can discover credentials beyond a profile label. P1-03 bypasses that chain with an explicit SSO provider and freezes each STS-verified credential set for resource calls. Locked-SDK source review and offline tests are recorded in [P1-03 evidence](p1-03-evidence.md); live provider and native-runtime behavior remain separate validation gates. [AWS Rust credential providers](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credproviders.html)

### P1-04 — Constrain and supervise the CLI child

**Next implementation unit.** The desktop CLI is temporarily blocked; completing this handoff and process contract is required before enabling it again.

**Depends on:** P1-02/03. **Touchpoints:** `aws_cli.rs` and its injected runner.

- Resolve an absolute executable path and execute argv directly. Platform discovery is completed in P4; never add a shell fallback.
- Construct a documented environment allowlist and controlled AWS configuration. Preserve required OS runtime variables and deliberately supported proxy/certificate settings; remove competing credentials, profile/provider, endpoint and debug settings.
- Preferred design: pass the same verified temporary credential snapshot to the child without persisting it; bind region/configuration explicitly. Revalidate on credential refresh. If a supported provider cannot be handed off safely, report CLI unavailable for that context while retaining supported SDK features.
- Reject user overrides for identity, region, endpoint, signing, TLS verification, pager/output and arbitrary local input files. Review both `--name value` and `--name=value`, repeats, abbreviated switches and nested file-value forms.
- Keep the existing 30-second deadline and 2 MiB stdout cap as initial limits; propose a separate 256 KiB stderr cap. Enforce streaming caps before accumulation, preserve only bounded sanitized error text, and await child termination. These are design limits, not measured behavior.
- **Done when:** fake-child tests prove exact argv/environment, no secret persistence/logging, bounded streams, and termination/reaping on timeout, cancellation and overflow. Real executable discovery and OS process behavior await native validation.

### P1-05 — Bind every result to the request that produced it

**Depends on:** P1-03 and the constrained runner contract. **Touchpoints:** `commands.rs`, `state.rs`, [app.js](../../frontend/app.js), nested widget fetch routes.

- Carry context revision and request ID through a small response envelope, adapting existing render models rather than rewriting the frontend.
- Bind inherited tiles, pinned tiles, detail cards, selectors and auth status to the captured context. Dispose or ignore responses when their owner is removed, reconfigured or superseded.
- Clear inherited data during account verification. Retained data after a same-context refresh failure must keep its original identity and freshness label; P2 defines the presentation.
- Record logical cancellation now; P1-04 handles subprocess termination, while P3 implements propagation to remaining SDK/query work. Ignoring a result must never be described as stopping its remote query.
- **Done when:** deterministic A→B→A switching, reversed credential completion, failed connection and late nested-detail responses cannot cross identities or resurrect removed UI.

### P1-06 — Close command, rendering and audit gaps

**Depends on:** P1-02–05. **Touchpoints:** Tauri capability/build manifest, command input validation, `audit.rs`, widget preflights and frontend rendering.

- Bound exposed JSON inputs, array/page limits and audit-tail requests; reject unknown widget/command shapes before provider work. Preserve local-only command access and the existing CSP.
- Exercise hostile synthetic AWS strings and links through production rendering. Insert text safely and permit only deliberately supported external URL schemes/hosts; never treat resource data as HTML or command authority.
- Use request IDs and distinct denied/started/succeeded/failed/cancelled events. Identify app intent separately from SDK retries/provider activity; do not claim comprehensive per-wire-call auditing without instrumentation.
- Redact tokens, credentials, raw command arguments, private configuration and raw service-error payloads from diagnostics. Keep local account context only where needed; future shared support bundles must redact it.
- Surface audit-write failures without inventing success. Proposed beta behavior: inspection remains usable with a visible diagnostics warning; operation authorization never depends on an audit write succeeding. P3 owns bounded reading/retention.
- **Done when:** command capability checks, malicious-render fixtures and secret-marker tests pass; logs and UI distinguish authorization, execution and cleanup outcomes.

Tauri capabilities govern which windows/webviews may reach commands and permissions. This supports the existing IPC boundary; application-specific AWS authorization remains backend work. [Tauri capabilities](https://v2.tauri.app/security/capabilities/)

## Planned acceptance evidence

P1-02's operation/argument denial cases and P1-03's identity, connection-ordering, configuration/refresh and auth-poll cases pass locally; see [P1-02](p1-02-evidence.md) and [P1-03 evidence](p1-03-evidence.md). CLI lifetime/environment, broader tile ownership and rendering/audit contracts remain pending. The table retains the phase-wide acceptance contract, including already covered cases.

| Case | Required observable result | Contract |
| --- | --- | --- |
| Wildcard policy + synthetic forbidden `ecr batch-delete-image` | Denied; zero process spawns; no grant hint | CB-J04 |
| Allowed operation with profile/region/endpoint/TLS/file override | Rejected before spawn, including alternate syntax | CB-J04 |
| Malformed policy + resource request | Denied before provider/transport work | CB-J04 |
| Profile A resolves identity B | No active or pinned verified context created | CB-J01/02 |
| Older A succeeds or fails after B connects | B and its status stay current | CB-J02 |
| Custom config changes while pinned work is pending | Old context/result invalidated; no global environment race | CB-J02 |
| Credential refresh changes principal | Affected work requires renewed verification; old cache never reused | CB-J01/02 |
| Child stdout/stderr overflow or timeout | Bounded retained bytes; child terminated/reaped; explicit failure | CB-J04 |
| Unmount/cancel with late detail response | No stale UI update; remote cleanup status remains honest | CB-J02/04 |
| Synthetic HTML/URL and secret markers | No executable rendering or leaked diagnostic secrets | CB-J03/04/05 |

## Exit and handoff

P1 is complete only after the implementation exists, the deterministic cases pass through production boundaries, and a focused security review covers the changed paths. Preserve native process/provider checks in the validation ledger until real OS evidence exists; do not describe a synthetic pass as full platform verification.

P2 receives verified context/result/error contracts. P3 receives cancellation ownership, process byte limits and cache identity rules. P4 receives executable/environment/storage portability requirements. **P1-01–03 are complete locally. Next is P1-04:** verified credential handoff and a constrained CLI child. Full P1 remains incomplete; P1-05 tile ownership, P1-06 command/rendering/audit work and deferred validation stay open.
