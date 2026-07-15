# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
