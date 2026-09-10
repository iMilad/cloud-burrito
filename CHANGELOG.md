# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Target version: **0.3.0**, a local candidate for device testing. No release or
tag has been published for these changes.

### Added

- Studio with four selectable appearances: Original, Precision, Paper and
  Night Shift. Paper is the default; Classic remains available.
- Per-widget header treatments with a preview, a collapse/expand action that
  remembers widget height, and local contrast adjustment in Appearance.
- Investigation workspaces, bounded table rendering and explicit partial,
  unavailable and stale result states.
- Native candidate build paths and inspection receipts for macOS ARM64/Intel,
  Windows x64 and Ubuntu 22.04 x64. Windows/Linux device acceptance is pending.

### Fixed

- Use the selected Cloud Fold mark in Classic and keep the Studio return control
  reachable from the topbar, including fullscreen widgets and pending startup.
- Allow connection details to close and reopen without reconnecting, preserving
  the choice across polls and restart while surfacing a new connection failure.
- Show the selected cached SSO access token's remaining lifetime in the topbar.
  Retrying the connection or obtaining new account credentials does not reset
  that timer; genuine token renewal updates its actual expiry. Keep account
  credential expiration separately in Identity details.
- Keep result feedback compact with optional technical details and local display
  preferences for tips and expanded results. Limited, stale and failed results
  retain visible status and recovery information.
- Preserve an unavailable pinned account when saving unrelated widget settings.
- Preserve results and unfinished inputs when saving header-only changes.
- Keep newer policy drafts when asynchronous reads or saves finish, and report
  rejected saves without claiming success.
- Restore scrolling, grid controls and keyboard focus after a fullscreen widget
  is removed.
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
- Limit the secret-scanner exception to the exact public Studio preference key
  in its source file, with positive and negative detector checks.

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
