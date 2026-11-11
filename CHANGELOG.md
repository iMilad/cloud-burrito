# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Target version: **0.3.0**, a local candidate for device testing. No release or
tag has been published for these changes.

### Added

- Mandatory, version-pinned Gitleaks scanning of full Git history in CI.
- Bounded table rendering and explicit partial, unavailable and stale result
  states.
- A keyboard-first watchlist of specific things from many accounts, each pinned
  to its own profile, account and region, with an inspector that drills from a
  failed run to its build log line.
- A vim-style command line (`:pipe`, `:pkg`, `:logs`, `:stack`, `:insights`,
  `:aws`, ...) whose results can be saved with `s`.
- SSO token time for the selected thing's own profile.
- Native candidate build paths and inspection receipts for macOS ARM64/Intel,
  Windows x64 and Ubuntu 22.04 x64. Windows/Linux device acceptance is pending.

### Changed

- The default-account dashboard is replaced. Existing dashboard widgets are
  converted once into watchlist things; `dashboard.json` is read and never
  written.
- The synthetic browser benchmark measures the watchlist (fixture revision
  p301-browser-v2). It no longer includes the old widget add/remove memory-cycle
  series.

### Removed

- GridStack layout.
- The top-bar account and region pickers. Each thing now keeps its own
  profile, account and region.

### Fixed

- Patch locked HTTP/2 and TLS dependencies and remove the old transitive `rkyv`
  dependency to resolve the three vulnerabilities found by the fresh RustSec audit.
- Reject hidden AWS request parameters in CLI pagination tokens while retaining
  supported service cursors and CLI truncation offsets.
- Create private app storage on macOS/Linux and repair permissions on existing
  app files and preserved audit archives at startup without changing contents.
- Keep result feedback compact with optional technical details and local display
  preferences for tips and expanded results. Limited, stale and failed results
  retain visible status and recovery information.
- Keep newer policy drafts when asynchronous reads or saves finish, and report
  rejected saves without claiming success.
- Accept the pinned stock NSIS template while rejecting custom installer hooks.
- Provision locked application dependencies before offline release builds and
  transfer the inspected candidate archives with their original checksums.

### Security

- Verify account contexts and fence stale work; constrain CLI handoff and bound
  background work, outputs and retained diagnostics.
- Replace policy files atomically, preserving the active policy on partial
  writes, sync failures and rename failures.
- Update `anyhow` to 1.0.103 for RUSTSEC-2026-0190. Remaining upstream dependency
  warnings are tracked separately; this is not a clean-advisory claim.
- Patch locked TLS dependencies (`rustls` 0.23.46, `rustls-webpki` 0.103.16,
  `ctutils` 0.4.8) to resolve six vulnerabilities found by the fresh RustSec
  audit. RUSTSEC-2026-0387 in `crypto-bigint`, used by `aws-sigv4` for SigV4a
  signing, has no compatible fix yet. `src-tauri/.cargo/audit.toml` ignores
  only that advisory, with the reason and the condition for removing it.
- Update `tauri` to 2.12.5 (`tauri-build` 2.7.4, `tauri-plugin-log` 2.10.2),
  `serde_with` to 3.24.0 and drop `rand` 0.7.3 from the lock. The Tauri origin
  confusion advisory (GHSA-7gmj-67g7-phm9) affects Windows and Android when a
  remote page runs in the webview. The app's own pages are local under a
  `script-src 'self'` CSP, and its only outside links go to the AWS console, so
  exposure was very limited; the update removes it. `glib` 0.18.5 stays, because
  the Linux GTK stack in Tauri still requires it.
- Limit the only secret-scanner exception for source to the exact public
  preference key in one removed file that Git history still holds, with positive
  and negative detector checks.

## [0.2.9] - 2026-07-15

### Added

- Switch expanded CloudFormation stacks between resource and recent-event
  tables without leaving the widget.

### Changed

- Give expanded CloudFormation rows a stronger visual treatment and rebalance
  detail columns so long logical and physical IDs wrap readably.

## [0.2.8] - 2026-07-15

### Added

- Expand CodeArtifact package rows into a cached, newest-first history of up to
  ten complete versions, each with its publication date and copy action.

### Changed

- Compact the CodeArtifact package filters, use the header refresh control as
  the single load action, and improve normal and full-screen version layouts.

## [0.2.7] - 2026-07-13

### Fixed

- Supersede the tagged but unpublished 0.2.6 build by ignoring only all-zero
  12-digit sentinels extracted from compiled binaries in the privacy gate,
  while continuing to reject nonzero account IDs and hashed private markers.

## [0.2.6] - 2026-07-12

### Added

- Browser-mode Playwright coverage for startup, region selection, CloudFormation
  filtering, and persisted widget configuration.
- Dependency automation and community contribution templates.
- Explicitly unsigned, identity-free macOS convenience artifacts with SHA-256
  checksums and source-build instructions.
- A repository-level release follow-up policy and read-only release-status
  checker.

### Changed

- Pin the Rust toolchain, Tauri CLI, Node setup, and GitHub Actions dependencies.
- Use one lockfile-constrained, path-remapped release build script locally and
  in GitHub Actions.
- Share pinned release-tool versions between local and CI builds.

### Fixed

- Report the backend version from Cargo metadata instead of a stale constant.
- Persist browser-mode region changes and update widget context immediately.

### Security

- Add a restrictive Tauri content security policy and explicit per-command IPC
  permissions.
- Remove unnecessary hardened-runtime exception entitlements.
- Add automated Rust dependency audits and immutable action commit pins.
- Pin release builds to the immutable tag-trigger commit and refuse moved tags
  or attempts to overwrite already published release assets.
- Force release builds through Tauri's `--no-sign` path, clear inherited Apple
  identity variables, and reject publisher identities in generated artifacts.
- Remove persisted checkout credentials and minimize workflow token permissions.
- Remove personal identity references from first-party source and documentation.
