# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.6] - 2026-07-12

### Added

- Browser-mode Playwright coverage for startup, region selection, CloudFormation
  filtering, and persisted widget configuration.
- Dependency automation and community contribution templates.
- Explicitly unsigned, identity-free macOS convenience artifacts with SHA-256
  checksums and source-build instructions.

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
