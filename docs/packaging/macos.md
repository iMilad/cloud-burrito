# macOS packaging

The candidate matrix keeps separate ARM64 and Intel DMG + app ZIP pairs. Version,
application identifier and file naming remain unchanged. The explicit candidate
floor is macOS 13.0; it is a chosen test boundary, not an assertion that older
macOS versions were tested or previously supported.

Build on macOS with the repository's pinned Rust and Tauri CLI, the selected Rust
target, and Xcode command-line tools. End users do not need those build tools.
Both architectures can be compiled on an ARM64 build host; this does not test
Intel hardware or either architecture's desktop journeys.

```sh
./scripts/build-release.sh aarch64-apple-darwin
./scripts/build-release.sh x86_64-apple-darwin
```

The wrapper requires clean committed source and the complete artifact inspector.
It builds an exported tracked tree, remaps source paths, clears Apple/signing
inputs, uses `--no-sign`, and sets the deployment floor. `CI=true` and clearing
`TAURI_BUNDLER_DMG_IGNORE_CI` skip the pinned bundler's Finder-formatting script.
System `hdiutil` still creates/verifies disk images. No application is launched.

`ditto` preserves the app bundle in a ZIP. Inspection checks its plist, executable
architecture and permissions, publisher identity, and privacy; it mounts the DMG
read-only without opening Finder and compares the same app payload. ZIP metadata
and relative links are inspected too. The DMG is detached after inspection.

No Developer ID, notarization, stapling, Apple account/team identity, signing key,
or updater is configured. A local ad-hoc integrity signature is allowed; a
publisher authority or team is forbidden. Unsigned does not mean authenticated.
Actual Gatekeeper warnings, opening the app, replacement upgrades, persistence
and removal belong to P6. Do not disable macOS security globally to test them.

The bundler's embedded helper and system SDK are build inputs. Compiler/SDK
versions are recorded in each manifest; this is not a bit-reproducibility claim.

The first actual P4 build exposed a packaging difference: `ditto` included host
AppleDouble metadata and the DMG helper removed group/other write bits. ZIP
staging now uses a copy with the same write-bit policy and omits generated host
resource-fork/xattr/ACL metadata. Executable bits, application files and relative
links remain intact. The original built app is unchanged. Native comparison still
requires every application file, mode and link to match; it does not ignore the
difference in its acceptance logic.
