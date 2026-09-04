# P4-01 — Versioned artifact and compatibility contract

> Phase numbering note (2026-09-04): this historical record uses P5 for device validation and P6 for the portfolio. Those future gates are now [P6](phase-6.md) and [P7](phase-7.md), following the inserted [P5 design phase](phase-5.md). Recorded results, source identities and work IDs are unchanged.

`packaging/targets.json` is the shared `p4-v1` contract: two macOS architectures with DMG/app ZIP pairs, Windows x64 MSVC with one NSIS installer, and Ubuntu 22.04 x64 with Debian/AppImage packages. It preserves all seven artifacts and the application identity. The contract distinguishes planned, build-verified and device-validated evidence; every row currently remains planned, with no validated OS entry.

Candidate compatibility floors are macOS 13, Windows 11 24H2 x64 and Ubuntu 22.04 x64. These are conservative project test targets, not verified support or claims about upstream minimum requirements. macOS previously inherited Tauri's lower default; explicitly narrowing the candidate floor avoids promising old webview behavior without tests. Windows 10, Windows ARM64, Linux ARM64 and other distributions are outside this first candidate. Actual laptop inventory may revise the matrix with a recorded reason before release.

Ubuntu 22.04 is a documented example of the oldest suitable WebKitGTK 4.1 build base; a newer build host can raise runtime library requirements. The Windows path uses a native MSVC build host and an explicit WebView2 acquisition policy. See [Tauri AppImage guidance](https://v2.tauri.app/distribute/appimage/), [prerequisites](https://v2.tauri.app/start/prerequisites/) and [Windows packaging](https://v2.tauri.app/distribute/windows-installer/), checked 2026-09-04.

The local host is macOS ARM64 with Rust 1.91.1, Tauri CLI 2.11.4 and Xcode SDK 26.5. Only the ARM64 Rust target is installed. The local container engine is Linux ARM64 and has no cached dedicated Ubuntu packaging image. Windows tooling/PowerShell is unavailable. This inventory is not a native app launch or installation test. Build-time dependencies may need explicit pre-provisioning; no AWS/GitHub access is permitted in this task.

The shared contract loader rejects unknown/dropped targets, duplicate or unsafe artifact names, missing formats and unsupported status values. Later units consume it for build selection, inspection, provenance and P5 instructions. Default application data remains the per-user `.cloud_burrito`; no migration, updater, signing identity, release or publication was introduced.
