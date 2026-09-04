# P4 — Local implementation and native evidence

> Phase numbering note (2026-09-04): this historical record uses P5 for device validation and P6 for the portfolio. Those future gates are now [P6](phase-6.md) and [P7](phase-7.md), following the inserted [P5 design phase](phase-5.md). Recorded results, source identities and work IDs are unchanged.

**All nine implementation/handoff units are committed. Four macOS artifacts are
built and inspected; the complete seven-artifact candidate is not accepted yet.**
Windows and Ubuntu require their declared native build hosts and reviewed helper
inputs. P5 device execution has not started.

Recorded 2026-09-04. Actual native artifacts below come from source `4983754`,
version `0.2.9`, with a clean immutable tracked export. The final follow-up records evidence and fences the optional shell-wrapper target
probe; it does not change these already-built artifact identities. For remaining platforms, use the exact
recorded candidate source in a clean checkout, or select a new source and rebuild
all four targets. Never combine outputs from different source commits.

## Separate local work units

| Unit | Commit | Outcome |
| --- | --- | --- |
| P4-01 | `9b65df3` | Versioned four-target, seven-artifact contract |
| P4-02 | `ab08d4c` | Explicit home/data paths and native process portability |
| P4-03 | `4d10d38` | Shared immutable-source build adapter and native wrappers |
| P4-04 | `e8992d0` | Per-user unsigned Windows installer/runtime policy |
| P4-05 | `9752b40` | Ubuntu baseline, Debian and AppImage policy |
| P4-06 | `5206a55` | Both unsigned macOS architecture pairs and OS floor |
| P4-07 | `ab7dae5` | Payload inspection, provenance, licenses and complete assembly |
| P4-08 | `297c9fc` | Inactive orchestration example; existing workflows unchanged |
| P4-09 | `4983754` | Real packaging discrepancy fixed; helper postflight and P5 handoff |

This final evidence is a separate P4-09 follow-up commit. No earlier unit was
squashed, amended or rewritten.

## Actual artifacts

Local files are under the ignored `dist/candidates/p4-final/` directory. Each
architecture folder contains two distributables and its build manifest. These
files were not uploaded or attached to a GitHub release.

| Target | Format | Bytes | Build-time result |
| --- | --- | ---: | --- |
| macOS ARM64 | DMG | 11,004,308 | Passed |
| macOS ARM64 | app ZIP | 10,870,366 | Passed |
| macOS Intel | DMG | 11,310,469 | Passed |
| macOS Intel | app ZIP | 11,182,826 | Passed |
| Windows x64 | NSIS setup | — | Native Windows host unavailable; wrapper refused before building |
| Ubuntu 22.04 x64 | Debian + AppImage | — | Native baseline/helper inputs unavailable; wrapper refused before building |

Both macOS pairs pass architecture/plist/minimum-OS checks, publisher-identity
checks, extracted payload privacy, DMG integrity and exact app-file/mode/link
comparison between DMG and ZIP. No native app was launched. ARM64 compilation
and Intel cross-compilation on an ARM64 host do not prove either device journey.

`macos-SHA256SUMS.txt` covers the four actual distributables and passes an
independent `shasum -a 256 -c` verification. Full digests and normalized payload
hashes remain in each build manifest. This partial checksum list is explicitly
not the complete candidate's `SHA256SUMS`.

Passing only the two macOS input directories to the complete assembler was
rejected, and no complete candidate directory was created. Windows/Linux are not
silently dropped, substituted with fixtures or represented by renamed files.

Build environment: macOS 26.6.2 ARM64, Rust/Cargo 1.91.1, Tauri CLI 2.11.4, Xcode
SDK 26.5 and Apple clang 21.0.0. The Intel Rust standard target was provisioned
from the official Rust distribution; no dependency version/lockfile changed.
Build children used a network-denying macOS sandbox, Cargo offline mode and
rustup automatic-installation disablement. No app publisher certificate, Apple
identity, notarization, updater or signing service was used.

## Repeat build

The ARM64 candidate was rebuilt from the same clean source and protected input
digest into a separate output directory. Both repeated artifacts passed native
inspection. Fresh tracked exports and fresh bundle outputs were used; the compiler
cache directory was reused. Both normalized app payload hashes match exactly.

The DMG and ZIP container hashes differ. The repeated DMG is 11,004,303 bytes;
the ZIP remains 10,870,366 bytes. ZIP entries retain identical payload CRCs with
changed timestamps. DMG container metadata/layout was not fully normalized or
attributed. This establishes matching application payloads for this one repeated
build, not byte-for-byte reproducibility or a general reproducible-build guarantee.
Machine-readable comparison is in local `dist/candidates/p4-final/repeat-comparison.json`.

## Validation and limits

- 350 Rust tests passed; two opt-in performance benches remain intentionally
  ignored. Formatting and all-target Clippy passed after the runtime changes.
  Subsequent P4 changes only affect build tooling, packaging policy and docs.
- 113 Python helper fixtures and 30 Node production-handler tests passed.
  Both Windows/Linux overlays pass static policy validation. Native Windows
  conditional Rust tests still require Windows; fixtures do not validate it.
- Source privacy and Gitleaks source/history checks passed. Detect-secrets
  verified 272 public benchmark fingerprints with no unverified findings after
  the exact reviewed public checksum/synthetic annotations. TruffleHog scanned
  an exported tracked source tree without verification or updates: zero findings.
- Cached Cargo audit passed with the existing 20 allowed warnings. No advisory
  database refresh occurred; this is not a current public-release security review.
- The offline dependency inventory records 426 Cargo packages and four npm
  development packages; 30 declared license entries need review by the bounded
  parser. Native/helper/system notices and full redistribution review remain open.
- No browser/frontend production source changed from P3; its 126-case browser
  evidence remains historical and was not substituted for native device tests.

Scans are bounded evidence, not proof that every possible secret or malicious
behavior is absent. Unsigned checksums/manifests establish consistency rather
than publisher authenticity. Raw compiler logs and generated NSIS scripts can
contain build paths and remain local; they are not approved publication material.

## Next gate

Provision the dedicated Windows x64 and Ubuntu 22.04 x64 build environments and
reviewed native helper inventories, then build/inspect those missing artifacts
from the selected common source. Only a complete verified set can pass assembly.
The current matrix remains the candidate policy; per-source build evidence is
recorded here and in target manifests, without a device-support claim.

Then start [P5-01](phase-6.md) with actual laptop inventory and the
[handoff instructions](../packaging/p6-handoff.md). Test installation, first
launch without credentials, persistence, upgrade and uninstall on isolated test
data. Connected AWS journeys remain separately authorized work.

No push, remote Git/GitHub action, AWS request, native installation/app launch,
release tag, draft/public release, signing identity or publication was performed.
