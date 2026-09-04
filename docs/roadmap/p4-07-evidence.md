# P4-07 — Inspectable candidate provenance

Implemented locally on 2026-09-04. Real native artifact/repeat-build outcomes are
recorded in the final P4 handoff; fixture reports are never candidate evidence.

## Changes

- Source/input identity uses immutable Git bytes, including on CRLF checkouts.
  The exporter supports Python 3.10 without relying on newer tar filter APIs.
- All tool probes receive the controlled build environment; rustup automatic
  installation and inherited compiler/target/signing overrides are disabled.
  macOS build children run inside a network-denying sandbox.
- Native payload inspection covers ZIP/DMG, PE/NSIS, Debian and AppImage. It checks
  identity, architecture/version, permissions/links, publisher policy and privacy.
  AppImage is extracted externally; Debian/AppImage executables must match.
- Four per-target manifests retain source, lock/config fingerprints, tools/SDK,
  helper inventory, dependency/license declarations and inspection evidence.
- Complete assembly requires all seven distributables from the same source and
  preserves the four build manifests. Independent verification checks the complete
  set and SHA-256 sums. Missing/extra/changed or mixed-source inputs fail.

Validation: 101 Python helper tests pass, including 15 format-inspection fixtures,
26 candidate assembly fixtures, 6 build-helper fixtures and 6 dependency inventory
fixtures. Both actual non-macOS overlays pass policy validation. Current tracked
source privacy scan passed (254 files before this unit was staged).

The offline resolved macOS metadata has 426 Cargo packages and 4 npm development
dependencies; 30 license declarations require further review by the deliberately
bounded inventory parser. This is a declaration inventory, not a shipped-binary
SBOM or proof of license compliance. Native/runtime/helper notices remain a
release prerequisite. No dependency was updated.

The scanner initially flagged four public build-input checksum literals and one
literal synthetic signing fixture. Cached public crate/schema digests were
verified, then only those exact reviewed lines were annotated. No secret-value,
path, or whole-file exclusion was added for those findings. Full final gates are
recorded with P4-09.

See [provenance, assembly and limits](../packaging/provenance.md). Recorded unsigned
metadata describes checks; it does not authenticate a publisher or repeat native
installer behavior on another machine.
