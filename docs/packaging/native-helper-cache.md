# Native helper cache: offline preflight

`scripts/check-native-helper-cache.py` checks a previously reviewed local tool
inventory. It reads file metadata and hashes; it does not execute, acquire,
repair or approve helpers. Missing or changed inputs fail before the native
bundler runs. No real helper inventory is shipped yet: native Windows/Linux
packages remain pending until the necessary reviewed inputs and hosts exist.

## Exact cache route

Both platform overlays require `bundle.useLocalToolsDir:true`. In the pinned
Tauri CLI 2.11.4, `src/interface/mod.rs` derives the local tools directory from
Cargo metadata's `target_directory`. Bundler 2.9.4 appends `.tauri` in both its
Windows NSIS and Linux AppImage modules. The effective route must therefore be:

```text
absolute CARGO_TARGET_DIR
└── .tauri
    ├── NSIS/                         Windows
    └── AppImage helper files         Linux
```

The build wrapper must set an absolute `CARGO_TARGET_DIR`, reject overrides
such as a conflicting `--target-dir`, and verify that Cargo metadata returns
that same directory before preflight. It must keep the same environment and
target directory for the build. Without `useLocalToolsDir`, upstream uses an
OS cache directory, so checking an unrelated cache would not protect the build.
The preflight rejects symlinks and Windows reparse points in the cache route.

```sh
python3 scripts/check-native-helper-cache.py \
  --target x86_64-pc-windows-msvc \
  --cargo-target-dir /absolute/cargo-target-directory \
  --inventory /absolute/reviewed-inventory.json --json
```

Use the native platform's absolute path syntax and a matrix target ID or triple.
This example describes an invocation, not an existing inventory or a download
instruction. The output contains counts and target identity, not local absolute
paths. It explicitly reports `executed:false` and `network_enforced:false`.

## Reviewed inventory contract

The JSON object must contain `schema_version:1`, `tauri_cli:"2.11.4"`,
`tauri_bundler:"2.9.4"`, the exact `target` triple from the matrix, and a nonempty
`files` list. Each entry has exactly `path`, positive integer `size` in bytes,
and lowercase 64-character `sha256`. Paths are relative to `.tauri`, use `/`,
and cannot include redirects, traversal, streams or duplicate spellings.

The complete inventory is limited to 10,000 files, 1 GiB of helper bytes and a
2 MiB JSON input. The checker streams a bounded number of bytes from each
regular file, checks identity before and after reading, and verifies the exact
size and digest. All entries need prior provenance review; hashing arbitrary
local bytes does not establish their source or trust. The checker deliberately
has no inventory-generation or auto-approval mode.

Windows uses the NSIS 3.11 distribution plus `nsis-tauri-utils` 0.5.3. Include
**every regular file** in `.tauri/NSIS`, including its includes, language files,
Modern UI assets, compressor stubs and plugins. The checker requires upstream's
13-file existence gate plus the English/Modern UI includes and installer
plugins. Any additional unreviewed file fails. It also checks the plugin's
upstream SHA-1 `75197FEE3C6A814FE035788D1C34EAD39349B860`, in addition to its
reviewed SHA-256, because a mismatch causes this bundler to redownload it.
Missing required files cause upstream to recreate the NSIS directory; the
preflight prevents entering that path with a knowingly incomplete cache.

Linux x64 requires these five files, each in the reviewed inventory:

- `AppRun-x86_64`
- `linuxdeploy-x86_64.AppImage`
- `linuxdeploy-plugin-gtk.sh`
- `linuxdeploy-plugin-gstreamer.sh`
- `linuxdeploy-plugin-appimage.AppImage`

The pinned bundler prepares the GStreamer script even when media bundling is
disabled. It also attempts to acquire the AppImage plugin when absent, although
upstream permits fallback after that download fails. This project's preflight
requires both files to avoid those attempts and an implicit fallback. Upstream
helper sources include moving branches or release aliases; pinning the Rust
crate alone does not pin all helper bytes.

The bundler overwrites three bytes at offsets 8–10 in cached `linuxdeploy` to
avoid AppImage integration detection. Acquisition provenance and the effective
build-input hash must be distinguished. Record any transformation and review
the effective digest; a later preflight must fail if a cached file no longer
matches its approved inventory. This checker does not mutate the helper or
silently update an inventory after a build.

## Limits of the check

`CARGO_NET_OFFLINE=true` and Cargo's `--offline` cover Cargo dependency lookup.
They are not a network boundary for Tauri, NSIS, linuxdeploy or nested plugins.
Even a complete top-level helper inventory cannot establish what a child tool
will try to access. Use a separately enforced no-network build environment and
record that control if the build claims network isolation. Without it, report
only “Cargo offline; reviewed local helper inputs; no acquisition authorized.”

The cache must not be modified concurrently between verification and use. This
preflight is a read-only consistency check, not a filesystem lock or sandbox.
All external acquisition, signature/provenance review and helper installation
remain outside this task. Runtime WebView2 acquisition on a user's Windows
device is a separate documented installer behavior, not a build-time download.

## Source of the checks

The implementation was inspected from the already cached Tauri CLI 2.11.4 and
bundler 2.9.4 crates. Relevant files are CLI `src/interface/mod.rs`, bundler
`src/bundle/windows/nsis/mod.rs`, its `installer.nsi`, and
`src/bundle/linux/appimage/linuxdeploy.rs`. The projected schema records the
full upstream schema's SHA-256 and CLI crate SHA-256. It contains the exact
constraints for approved overlay fields and their references; it is not a
replacement for Tauri's complete schema. Changing the CLI pin requires a fresh
review. [Tauri configuration reference](https://v2.tauri.app/reference/config/)

## Post-build byte verification

The builder retains the preflight report in memory and checks the same reviewed
inventory after native packaging. Every helper must remain unchanged except the
pinned Linuxdeploy transformation: bytes 8 through 10 become zero. Preflight
computes that exact expected digest; postflight also restores the original three
bytes in a hash stream and requires the original reviewed digest. Unrelated edits,
changed inventory, missing tools and new Windows helper files fail acceptance.
Post-build relative file hashes are retained in the manifest. The checker neither
repairs nor approves the cache. Later builds require reviewed effective inputs or
restoration of the original reviewed bytes. Observations cannot detect transient
changes undone between checks, or replace separate network isolation.
