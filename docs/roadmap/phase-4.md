# P4 — Native artifacts from one source candidate

**P4-01–02 implemented locally; P4-03 is next. Native artifact and device evidence remain pending.**

The original plan was source-reviewed on 2026-09-03 against v0.2.9 (`095d1ad`). Implementation follows the completed P3 source at `e82dcb5`; each unit has a separate local commit and evidence. [P4-01](p4-01-evidence.md) defines the versioned candidate matrix. Rust/Tauri remains the application stack.

P4 produces inspected native artifacts and the evidence needed for P5. P5 performs actual installation, first launch, upgrade, uninstall, and journey acceptance on declared devices. Outstanding [P0 device evidence](phase-0.md#platform-decision-still-open) remains a release prerequisite, not a blocker to finishing this plan. No device checks are requested during planning.

## Historical source baseline at planning

| Area | Current evidence | Implication |
| --- | --- | --- |
| Bundle configuration | [`tauri.conf.json`](../../src-tauri/tauri.conf.json#L32) targets `app` and `dmg`; includes `.icns` and `.ico` icons | Windows and Linux artifact targets are not configured |
| Application identity | Same file defines product name, version `0.2.9`, and identifier `app.cloudburrito.desktop` | Preserve stable identity; version changes need the existing release process |
| Build helper | [`build-release.sh`](../../scripts/build-release.sh#L22) remaps paths, clears Apple variables, uses `--no-sign` and `--locked`, then assumes a macOS `.app` | Retain its safeguards while separating platform-specific staging/inspection |
| Build inputs | [`rust-toolchain.toml`](../../rust-toolchain.toml), [`tool-versions.env`](../../scripts/tool-versions.env), and both lockfiles | Rust 1.91.1 and Tauri CLI 2.11.4 are current pins, not a claim that every native tool is pinned |
| Frontend tooling | [`package.json`](../../package.json) declares Node >=22 and npm 10.9.8; [`CI`](../../.github/workflows/ci.yml#L30) selects Node 22 | Align future documented build/test environments and record exact resolved versions |
| CI coverage | [`ci.yml`](../../.github/workflows/ci.yml#L18): Ubuntu browser job, macOS Rust job | An Ubuntu browser pass is not a Linux native build |
| Release artifacts | [`release.yml`](../../.github/workflows/release.yml#L146): ARM64 and Intel macOS builds, unsigned DMG + app ZIP, checksums, draft release | Preserve both macOS artifact pairs; add platforms without weakening existing checks |
| Local paths/executable | [`paths.rs`](../../src-tauri/src/paths.rs#L5) uses a per-user `.cloud_burrito`; [`aws_cli.rs`](../../src-tauri/src/widgets/aws_cli.rs#L118) searches for `aws` and macOS fallback paths | Windows executable discovery and path behavior require explicit implementation |

## Decisions and dependencies

| Decision | Planned position | What must settle before a release candidate is accepted |
| --- | --- | --- |
| macOS artifacts | Preserve separate ARM64 and Intel unsigned `.dmg` and `.app.zip` | Declared minimum OS and evidence for each architecture; one ARM64 laptop does not validate Intel |
| Windows primary artifact | One unsigned NSIS setup `.exe`; provisional x64 MSVC target | Actual device architecture/Windows version and installer behavior; add ARM64 only with evidence |
| Windows alternative | MSI deferred unless a concrete managed-install requirement appears | Additional packaging/runtime/elevation validation if brought into scope |
| Ubuntu artifacts | `.deb` primary, AppImage secondary; provisional x64 | Actual Ubuntu release/architecture, native library baseline, and both formats' compatibility |
| Linux build baseline | Oldest declared supported base that provides required Tauri native dependencies | Select after inventory; no blanket “all Ubuntu/Linux” support claim |
| Data location | Provisionally retain the current per-user data directory for the first cross-platform release | A move to OS-specific locations requires an explicit versioned migration design and P5 coverage |
| Updating | Manual replacement/installer upgrade for the first release | Do not introduce an updater, update endpoint, or signing key in this phase |

The NSIS/MSI options are supported by [Tauri's Windows installer documentation](https://v2.tauri.app/distribute/windows-installer/); NSIS is the proposed first format to keep one Windows acceptance path. Tauri recommends building on Windows rather than relying on its less-tested cross-compilation path. The choices above remain project decisions, not claims of completed compatibility.

Implementation depends on P1's command, credential, process, and filesystem boundaries; P2's durable settings and recovery contracts; and P3's measurement method and responsiveness limits. Packaging adapters can be prepared earlier, but a release candidate must include the accepted P1–P3 work. P0 inventory may change target labels and package prerequisites without changing this sequence.

## Ordered work packages

### P4-01 — Freeze the artifact and compatibility contract

- Record target triple, CPU architecture, minimum/validated OS versions, package formats, WebView/runtime assumptions, and validation owner for every matrix row.
- Distinguish `planned`, `build-verified`, and `device-validated`; do not promote a row merely because its compiler target exists.
- Retain both existing macOS artifact pairs. Keep Windows x64 and Ubuntu x64 provisional until device evidence arrives.
- Output: one versioned matrix used by build scripts, manifest generation, download documentation, and P5.
- Acceptance: no artifact or support claim exists outside the matrix; unresolved rows are visibly pending rather than silently dropped.

### P4-02 — Close runtime portability gaps

- Extend executable discovery for Windows `aws.exe`; resolve and execute the selected path directly under P1's argument/environment controls. Preserve the no-shell boundary.
- Cover GUI-launch PATH differences, spaces/Unicode, drive-letter paths, custom AWS config locations, missing home directories, and inaccessible data directories through injected fixtures.
- Keep application data separate from installation files and user-owned AWS configuration. Packaging and uninstall policy must not delete AWS profiles, credentials, or SSO caches.
- Preserve existing settings/layout data; if migration becomes necessary, define backup, conflict, failure, and rollback behavior before moving files.
- Separate Rust/Node/native build tools from end-user runtime requirements. Document AWS CLI needs for the CLI widget and supported authentication setup; do not bundle credentials or silently install the CLI.
- Acceptance: platform-specific path/executable behavior has deterministic coverage; actual desktop launch and persistence remain P5 checks.

### P4-03 — Separate shared build logic from platform packaging

- Keep shared product/version/CSP/capability configuration; add explicit platform bundle selection without widening IPC or introducing remote frontend assets.
- Refactor the macOS-only helper into shared input/version/privacy checks and platform-specific build/staging adapters; use a native Windows wrapper where shell semantics differ.
- Keep Cargo lock enforcement and Tauri CLI version checking. Preserve path remapping using platform-correct paths and record the final build flags.
- Fail explicitly on unsupported targets, missing expected artifacts, unexpected duplicate outputs, or version/architecture mismatches.
- Acceptance: each adapter consumes the same source candidate and emits a declared artifact list; no adapter tags, uploads, or publishes anything.

### P4-04 — Define the Windows installer and WebView2 policy

- Build the provisional x64 MSVC target on a Windows build host using the pinned Rust/Tauri inputs and recorded native tool versions.
- Use NSIS as the first installer and propose a per-user app installation. Inspect generated behavior; do not promise zero elevation for every runtime/setup condition.
- Select Evergreen WebView2 with an explicit missing-runtime check and bootstrapper policy. State that runtime acquisition can require network access; do not advertise an offline installer.
- If offline installation becomes required, assess a separate standalone-runtime variant, including size, redistribution, update, and validation costs, before adding it.
- Keep application executables/installers unsigned and free of personal publisher metadata. Record expected OS warning behavior for P5 rather than weakening OS protections.
- Acceptance: installer contents, target architecture, version, install scope, runtime policy, and absence of app publisher certificates are inspected; install/upgrade/uninstall results are still pending P5.

Tauri documents the available [WebView2 installation modes](https://v2.tauri.app/distribute/windows-installer/#webview2-installation-options). Microsoft recommends checking for the runtime even when it is commonly preinstalled; its [distribution guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution) distinguishes online bootstrapper and offline standalone deployment. Evergreen receives runtime updates; a fixed runtime transfers update maintenance to the application distributor.

### P4-05 — Define Ubuntu packages and the native-library baseline

- Build `.deb` and AppImage from the same Linux target and declared baseline; keep package metadata, desktop entry, icons, and executable identity consistent.
- Inspect generated Debian dependencies against the chosen Ubuntu release. Do not copy development-package requirements into end-user instructions unchanged.
- Build against the oldest declared supported base with Tauri v2's required WebKitGTK availability; record glibc/native-library requirements. Do not choose a moving `ubuntu-latest` label as the compatibility contract.
- Treat AppImage as an additional distribution format, not proof of compatibility with every Linux system. Record remaining host/runtime requirements and defer actual desktop execution to P5.
- Keep application/runtime permissions narrow; do not add root-running application behavior, background services, or repository installation hooks for convenience.
- Acceptance: both payloads have the declared architecture/version, expected files/dependencies, and no unexplained maintainer scripts or private data.

Tauri's [Debian packaging guidance](https://v2.tauri.app/distribute/debian/) covers generated runtime dependencies and desktop integration. Its [AppImage limitations](https://v2.tauri.app/distribute/appimage/#limitations) explain why the build base affects glibc compatibility. The documented baseline examples are inputs to selection, not Cloud Burrito support promises.

### P4-06 — Preserve macOS packaging and unsigned guarantees

- Retain `aarch64-apple-darwin` and `x86_64-apple-darwin`, each with the current unsigned DMG and app ZIP naming convention.
- Preserve executable architecture checks, plist validation, DMG integrity checks, and app/binary/DMG publisher-identity inspection from the existing release workflow.
- Preserve app bundle structure, permissions, and links when creating the ZIP; ensure DMG and ZIP contain the same candidate payload.
- Keep the explicit no-sign build path and cleared Apple build environment. Do not add Developer ID, notarization, stapling, certificates, team/account IDs, or personal publisher fields.
- Acceptance: both artifact pairs pass build-time inspection; Gatekeeper behavior and actual ARM64/Intel operation are recorded separately in P5.

The governing boundary is [AGENTS.md](../../AGENTS.md), and the existing inspection/staging behavior is in [release.yml](../../.github/workflows/release.yml#L216). “Unsigned” must not be presented as trusted-publisher authentication.

### P4-07 — Make build inputs, privacy, and provenance reviewable

- Record source commit, version, source cleanliness, lockfile digests, Rust/Tauri/native tool versions, build image/SDK, target triple, and relevant flags in a non-personal build manifest.
- Use locked dependencies and pinned helper versions; record external native/bundler inputs and approved acquisition sources. Cached inputs must match target/toolchain/lockfile keys.
- Attempt a clean repeat build when implementation reaches this gate. Compare payloads and explain timestamp/container differences; do not claim byte-for-byte reproducibility without evidence.
- Generate SHA-256 for each final distributable and a manifest mapping filename, platform, architecture, size, and source commit; verify the combined checksum set independently of build jobs.
- Scan tracked source and extracted package payloads, including binary strings and metadata. The current privacy scanner walks files but does not unpack archives; add format-aware staging instead of scanning compressed bytes alone.
- Exclude credentials, user configuration, real account/resource data, absolute personal paths, signing identities, and unintended development/debug artifacts. Document scanner limits and include dependency/license evidence.
- Acceptance: each payload can be traced to one candidate; missing/extra artifacts fail the manifest gate. Checksums establish integrity relative to the manifest, not publisher authenticity; unsigned provenance is descriptive evidence.

Existing foundations: [version checks](../../scripts/check-release-version.py#L132), [privacy traversal](../../scripts/check-release-privacy.py#L113), and [macOS checksums](../../.github/workflows/release.yml#L252).

### P4-08 — Prepare CI job orchestration without enabling publication

- Define the following job graph locally; enabling or running remote workflows is a later authorized action.
- Keep build jobs read-only with respect to repository contents, without AWS credentials or signing secrets; use synthetic fixtures and fail-closed AWS boundaries.
- Preserve immutable source selection, pinned action references, and target-aware cache keys. Keep failed matrix rows visible and prevent partial artifact promotion.

| Sequence | Proposed job | Required output / dependency |
| --- | --- | --- |
| 1 | `validate-source` | Version/privacy/registry checks and accepted P1–P3 validation; one source commit |
| 2 | `build-macos` matrix | ARM64 and Intel DMG + app ZIP; depends on validation |
| 2 | `build-windows` | Declared MSVC target and NSIS installer; depends on validation |
| 2 | `build-ubuntu` | Declared baseline/target, `.deb` + AppImage; depends on validation |
| 3 | `inspect-artifacts` | Per-format payload/metadata/identity/privacy checks; depends on all required builds |
| 4 | `assemble-candidate` | Complete manifest, verified checksums, redacted build evidence; depends on inspection |
| 5 | P5 handoff | Artifact transfer and test instructions; no automatic release/publication |

Acceptance: a local or future CI candidate can be assembled without a release tag. The current tag-triggered Release workflow remains a separate publication path; any future extension must require all declared platform artifacts and preserve its immutable-tag and draft-only safeguards.

### P4-09 — Hand over one inspected candidate to P5

- Provide the exact artifact set, source commit, checksums, compatibility matrix, runtime/network prerequisites, known limitations, and redacted build logs.
- Prepare platform-specific checksum and installation guidance after the matrix is frozen; mark OS warning expectations honestly and avoid blanket security-disable instructions.
- Include P5 cases for fresh install, missing runtime, first launch without credentials, CB-J01–CB-J05, restart/persistence, upgrade from the supported prior version, and uninstall/data-retention behavior.
- Acceptance: P5 can identify the exact candidate and expected result for each case. No device result is marked passed by this handoff.

## Completion and release boundary

- [x] P4 planning is complete: ordered work, provisional decisions, dependencies, build evidence, and P5 handoff are defined.
- [ ] P4 implementation has begun and the platform adapters/configuration changes are reviewed.
- [ ] P0 device inventory and build/launch evidence have frozen the declared matrix.
- [ ] P1–P3 acceptance evidence is attached to the candidate.
- [ ] Every declared artifact is built and inspected from one source commit; checksums/privacy/provenance gates pass.

After P4 hands over the inspected candidate, P5 completes actual native installation, operation, upgrade, and uninstall acceptance. That is the next phase's gate, not a prerequisite for building the P4 candidate; it remains required before the corresponding platform-support claim or public release.

No tag, release draft, upload, repository publication, signing, or device execution is authorized by this planning document. Follow [AGENTS.md](../../AGENTS.md) for the later approved draft-release sequence; public draft publication remains a separate explicit decision. Unresolved device evidence affects readiness to release, while this planning work remains complete.

Official documentation was checked read-only on 2026-09-03. Platform prerequisites must be reconfirmed against the chosen toolchain and OS matrix when implementation begins; [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) distinguishes native development dependencies from runtime distribution.
