# P4-06 — Preserve unsigned macOS artifacts

Implemented locally on 2026-09-04. Both architecture-specific DMG and ZIP names
are retained. The macOS overlay explicitly sets the chosen 13.0 candidate floor
and leaves publisher identity and entitlements unset. Hardened-runtime defaults
are retained. No signing, notarization, updater, version or application-identity
change was introduced.

The build environment now clears the inherited Finder-formatting override in
addition to Apple/signing inputs. Cached Tauri CLI 2.11.4 / bundler 2.9.4 source
confirms CI mode skips the optional Finder AppleScript. Its DMG helper is embedded
in the bundler; no helper-download path was found in that script.

Validation: 4 shared build-helper fixtures pass, including removal of the override
and preservation of spaces/Unicode in path-remapping arguments. Actual paired
artifact builds and inspections are recorded in the final P4 evidence after the
complete inspection/provenance tools are committed. No app launch occurred.

See [macOS build and unsigned policy](../packaging/macos.md).
