# Build the desktop candidates

The source has packaging paths for all three desktop operating systems. A full
candidate contains four targets and seven files. Build each target from the same
clean committed source after the final design is selected.

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

Transfer the four target directories through the agreed route, then follow
[candidate assembly and verification](provenance.md). Only after all seven
artifacts and their matching receipts are present is the complete candidate
ready for the owner's [device tests](p6-handoff.md).

## Current evidence

As of 2026-09-04, earlier macOS ARM64 and Intel builds were inspected in P4;
those files predate Studio and the new logo. The current Mac has both Rust
targets and the pinned Rust/Tauri versions. Windows and Ubuntu packaging policy
checks pass locally, but their native build machines and reviewed helper
inventories have not been supplied in this task. No device acceptance has been
claimed. Rebuild the final selected design before testing it.

The existing GitHub Release workflow remains macOS-only. The four-target
[CI example](ci-orchestration.md) is inactive; these local instructions do not
enable a workflow, push code, release files or connect to AWS.
