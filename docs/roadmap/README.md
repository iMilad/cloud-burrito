# Cloud Burrito execution roadmap

Status: **P1-01 through P1-05 complete locally. P1-06 is next. Full P1 remains incomplete. P2–P4 remain planned; P0 device evidence remains open.** Original source baseline: `0.2.9`, commit `095d1ad`, reviewed 2026-09-03.

Latest completed unit: [P1-05](p1-05-evidence.md) adds verified response envelopes and request ownership across production frontend tiles, pins, selectors and nested details. Its Rust, Node and synthetic browser checks pass locally.

Previous completed unit: P1-04 re-enables the constrained desktop CLI on the committed P1-01–03 baseline `a879851`. The full Rust library suite passes **128 tests**, with no failed, ignored or filtered tests; **5 Node production-handler tests** and **13 release-helper tests** also pass. See [P1-04 evidence](p1-04-evidence.md) and the [execution plan](execution-plan.md). The historical [P1-03 evidence](p1-03-evidence.md) records 97 Rust library tests and 5 Node tests; [P1-02](p1-02-evidence.md) and [P1-01 evidence](p1-01-evidence.md) retain their original results. Live AWS and native platform acceptance remain pending.

P1-03 adds explicit SSO snapshots, STS account/principal verification, frozen resource credentials, refresh/expiry checks, independent pinned contexts and configuration/auth-status guards. P1-04 hands those exact verified temporary credentials to an isolated CLI child, caps both output streams and supervises cancellation and direct-child cleanup. Cleanup failures survive a superseded context as the stable `CliCleanupFailed` UI/audit error. The P1-03 temporary CLI block is removed. Broader tile/detail ownership is now covered by P1-05; process-tree and native OS behavior remain unverified.

Executable support includes native installers and a narrow Unix absolute-Python wrapper launched through its validated native interpreter with isolated Python mode. Shell, environment-relative and batch wrappers remain unsupported. AWS CLI v2 version and publisher trust remain local-installation requirements; no version probe was run. The execution deadline is not a guaranteed cleanup-return bound: waiting for OS-confirmed exit can take longer, and a nonempty isolated directory may remain.

## Product direction

Cloud Burrito keeps an AWS investigation connected across accounts, services and evidence. The first public beta should let an engineer install the desktop app, establish a verified AWS context, inspect a failed deployment and compare accounts without reconstructing context at every step.

Keep Rust + Tauri. Improve the existing execution boundary, context correctness, daily usability and measured responsiveness. Claims about safety, speed and platform support must match the evidence recorded below.

## Canonical phases

Use **P0–P6** for work items and acceptance evidence. The presentation's five cards are summary themes; their card numbers are not execution-phase IDs.

| Phase | Outcome | Exit evidence | Planning / delivery |
| --- | --- | --- | --- |
| [P0 — Establish the truth](phase-0.md) | Scope, operation inventory, five journey contracts, baseline and early platform checks | First-party execution paths mapped; baseline and gaps recorded; Windows/Ubuntu build-and-launch attempts documented; initial support matrix frozen | Scope recorded / device and native evidence pending |
| [P1 — Build the trust boundary](phase-1.md) | Exact execution allowlist, verified active/pinned identities and stale-response protection | Forbidden actions denied before execution; mismatched identity blocks work; delayed responses cannot cross contexts | P1-01–05 complete locally / P1-06 next |
| [P2 — Make the core dependable](phase-2.md) | Coherent investigation workflow, onboarding, durable settings and explicit errors | Journey contracts pass through production paths with synthetic dependencies; unfinished controls handled honestly | Plan complete / implementation not started |
| [P3 — Earn the performance claim](phase-3.md) | Bounded work, cancellation, caching and repeatable measurements | Startup, result latency, memory and request counts measured; published claims reproducible | Plan complete / implementation not started |
| [P4 — Produce native artifacts](phase-4.md) | Unsigned packages from the same version and commit | Checksums, artifact contents and platform instructions reviewed; package installation proven in P5 | Plan complete / implementation not started |
| P5 — Validate on real machines | Packaged release candidate tested on the available macOS, Windows and Ubuntu laptops | Required journeys and install/restart/upgrade/uninstall pass on each declared OS/architecture; no unresolved critical/high defects | Outline only / not started |
| P6 — Build the portfolio launch package | English documentation, demo, decisions and release evidence | Privacy/history and dependency review complete; independent feedback recorded; explicit publication decision | Outline only / not started |

P0 probes platform compatibility early when devices become available. Pending device evidence does not block planning or future isolated P1 source work. It does block final support claims; comparable performance measurements must precede optimization claims. P4 produces candidate packages after trust, product and performance work. P5 tests those packages; a source build or Chromium test cannot substitute for this gate.

```mermaid
flowchart LR
    P1["P1 · Trust<br/>Verified identity + exact operations"] --> P2["P2 · Dependability<br/>Settings + connected investigation"]
    P2 --> P3["P3 · Performance<br/>Measure + bound + optimize"]
    P3 --> P4["P4 · Native packages<br/>macOS · Windows · Ubuntu"]
    P4 --> P5["P5 · Real device evidence"]
    P0["P0 · Device inventory<br/>Deferred until available"] -.-> P4
    P5 --> P6["P6 · Portfolio + publication decision"]
```

## Beta scope

- Explicit legacy and named-session SSO profiles; named sessions retain supported SDK token renewal, while expired legacy tokens require external SSO login. Live provider validation remains pending.
- Resource and identity inspection, plus bounded Logs Insights query control. Starting/stopping a query is a separate capability with cost and cleanup implications.
- One flagship investigation: Pipeline → Build → Stack → Logs, with explicit lookup when the association is unknown.
- Inherited and pinned account/region contexts.
- An optional CLI path for the 18 reviewed resource reads, using frozen verified credentials, isolated child configuration and bounded execution. P1-04 offline checks pass; native AWS CLI v2 compatibility remains unverified.
- Existing widgets retained where they meet their release criteria; no expansion to every AWS service.

Deferred: Go/Wails rewrite, plugin SDK, full AI assistant, infrastructure mutations, signing/notarization and publisher identity. Releases remain unsigned and identity-free under [AGENTS.md](../../AGENTS.md).

## Start here

1. Continue with P1-06 command/rendering/audit boundaries using the [P1-05 evidence](p1-05-evidence.md) and [execution plan](execution-plan.md).
2. Use the phase plans in order: [P1](phase-1.md) → [P2](phase-2.md) → [P3](phase-3.md) → [P4](phase-4.md).
3. Keep evidence against the [journey contracts](phase-0.md) and [operation inventory](aws-operation-inventory.md).
4. Resume [device checks](device-checks.md) when the laptops are available; final support and native acceptance remain conditional until then.

P1-01–03 are recorded in local commit `a879851`; P1-04 is a separate locally validated implementation unit. No live AWS call, native GUI acceptance, Windows/Ubuntu validation, install, push, tag, release or GitHub action was performed. Publication remains a separate decision after the reviewable release package exists.
