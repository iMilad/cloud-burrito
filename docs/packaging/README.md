# Build the desktop candidates

The source has packaging paths for all three desktop operating systems. A full
candidate contains four targets and seven files. Build each target from the same
clean committed source. The current repair candidate is **0.3.0**; select the
source commit recorded in its build receipts. It includes the
[pre-P6 audit repairs](../roadmap/p5-audit-fixes.md), all four Studio appearances,
the Paper default and the matching packaged operating-system icon. The earlier
`5d39eed` preflight is historical evidence only.

| Build environment | Target | Files for later device testing |
| --- | --- | --- |
| macOS 15+ with the recorded Xcode SDK | Apple Silicon | Unsigned DMG and app ZIP |
| The same Mac, with the Intel Rust target installed | Intel Mac | Unsigned DMG and app ZIP |
| Windows 11 x64 with Visual Studio 2022 C++ tools and SDK | Windows x64 | Unsigned setup EXE |
| Ubuntu 22.04 x64 with GTK3/WebKitGTK 4.1 development packages | Linux x64 | Unsigned DEB and AppImage |

This repository uses matching operating-system build environments. Tauri also
describes a Windows NSIS cross-build route from macOS, but calls out additional
tools and less-tested behavior; that route is not configured here. Linux packages
use the declared Ubuntu baseline to avoid raising their glibc requirement.
[Tauri Windows build guidance](https://v2.tauri.app/distribute/windows-installer/#build-windows-apps-on-linux-and-macos),
[Tauri AppImage baseline guidance](https://v2.tauri.app/distribute/appimage/#limitations)

## Check the build machine first

All build environments need Git, Python 3.10+, the repository's pinned Rust
1.91.1 and Tauri CLI 2.11.4, their selected Rust target, and cached lock-matching
Cargo dependencies. The frontend is bundled directly from `frontend/`; Node is
needed for frontend tests, not to compile this static frontend. End users do not
need these build tools.

Windows additionally needs a Visual Studio developer shell, a reviewed NSIS
helper cache and `7zz` or `7z` for package inspection. Ubuntu needs the native
compiler/GTK/WebKit toolchain, a reviewed AppImage helper cache and `7zz` or `7z`.
See [Windows](windows.md), [Ubuntu](linux.md), and the
[helper inventory contract](native-helper-cache.md) for exact requirements.
The helper inventory must already have been reviewed; the build command does
not acquire tools or create an approved inventory from arbitrary local files.

Provision locked Rust dependencies before entering the offline build boundary,
including on a cold cache. On each already provisioned native host, use its
target (choose one line):

```sh
cargo fetch --locked --manifest-path src-tauri/Cargo.toml --target aarch64-apple-darwin
cargo fetch --locked --manifest-path src-tauri/Cargo.toml --target x86_64-apple-darwin
cargo fetch --locked --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
cargo fetch --locked --manifest-path src-tauri/Cargo.toml --target x86_64-unknown-linux-gnu
```

This preparation can download registry packages; the build wrapper remains
offline. It does not replace the native compiler, installed Rust target or
reviewed NSIS/AppImage helper inventory.

On the Mac:

```sh
./scripts/build-release.sh macos-aarch64 --check
./scripts/build-release.sh macos-x86_64 --check
```

In a Windows developer PowerShell, with an existing reviewed inventory path:

```powershell
.\scripts\build-release.ps1 -Check -HelperInventory 'C:\reviewed-inputs\windows-helpers.json'
```

On the Ubuntu build host, with its existing reviewed inventory path:

```sh
./scripts/build-release.sh ubuntu-x86_64 --check \
  --helper-inventory /reviewed-inputs/linux-helpers.json
```

The inventory paths above are placeholders. `--check` checks the native host,
clean source, pinned tool versions, installed Rust target, required native tools,
offline dependency metadata, source gates, platform policy and helper bytes.
It exports the committed source into temporary storage, then removes that copy.
It does not compile, bundle, replace an existing candidate, install or launch the
app. A passing preflight means the checked inputs are available; the actual
build and native artifact inspection can still reveal problems. `--plan` only
prints the target definition and does not check local availability.

## Build, inspect, then hand off

After a successful check, remove `--check` (PowerShell: `-Check`) from the same
command to build. The wrappers compile, package and statically inspect each
target. By default, each writes a new directory at:

```text
dist/candidates/<source-commit-prefix>/<target-id>/
```

Use `--output` or PowerShell `-Output` to select a different new directory.
Existing candidates are never overwritten. No app installation, launch or
publication is part of the build.

Each accepted target directory can begin the owner's
[device tests](p6-handoff.md) after its files are checked against its receipt.
Transfer all four target directories through the agreed route, then follow
[candidate assembly and verification](provenance.md). Complete matrix acceptance
requires all seven artifacts and their matching receipts from the same source.

## Current evidence

The 2026-09-05 repairs pass the complete 152-test canonical browser gate,
357 Rust tests, 30 Node units and 132 release-helper tests. See the
[repair evidence and remaining upstream dependency disposition](../roadmap/p5-audit-fixes.md).
Windows and Ubuntu native hosts/helper inventories are still unavailable in
this task. Target-specific build receipts, when present, are authoritative for
new 0.3.0 package outcomes; no receipt means no accepted package for that target.
No installed-device test has passed.

### Earlier source records

As of 2026-09-04, earlier macOS ARM64 and Intel builds were inspected in P4;
those files predate Studio, the appearance selector and the Paper icon. They are
historical evidence, not candidates for the current source. The current Mac has
both Rust targets and the pinned Rust/Tauri versions. Windows and Ubuntu
packaging policy checks pass locally, but their native build machines and
reviewed helper inventories have not been supplied in this task. No device
acceptance has been claimed.

Earlier, on committed source `205303c`, both `macos-aarch64 --check` and
`macos-x86_64 --check` passed on this Mac. Each reported `built: false` and
`device_validated: false`; version, offline input/tool checks, the 17-command
registry and the scoped 295-file privacy scan passed. The helper regression
suite passed 116 tests, including the new preflight cases. This remains the
historical build-input record for that earlier Studio/logo source.

On the earlier committed application source `5d39eed`, both
`macos-aarch64 --check` and `macos-x86_64 --check` passed on this Mac. The
committed source contains the integrated Studio Original, Precision, Paper and
Night Shift appearances plus the Paper default/canonical package icon. The
preflights validated their build inputs and included a scoped 301-file privacy scan.
Both results reported `built: false`, `device_validated: false` and
`publication: false`: no current binaries were produced, installed, launched or
published. Windows and Ubuntu native preflights remain pending on their declared
hosts and still require reviewed helper inventories. No AWS connection or push
was part of this evidence.

The existing GitHub Release workflow remains macOS-only. The four-target
[CI example](ci-orchestration.md) is inactive; these local instructions do not
enable a workflow, push code, release files or connect to AWS.
