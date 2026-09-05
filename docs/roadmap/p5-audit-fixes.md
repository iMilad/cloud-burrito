# P5 audit repairs before device testing

The owner authorized implementation of the pre-P6 audit findings on 2026-09-05.
Changes are local, with separate P5-07 onward commits. No AWS connection, push,
tag, release, application installation or native application launch is part of
this repair work. The candidate version is **0.3.0**; the existing 0.2.9 tag and
history remain untouched.

## Corrected behavior

| Audit item | Change | Regression evidence |
| --- | --- | --- |
| F01 — unavailable saved profile | Keep the exact pinned context visible through unrelated edits; reject incomplete pins | Missing-profile, empty-discovery and explicit unpin/change browser cases |
| F02 — partial policy write | Validate and bound complete text; write a private temporary file, sync, then replace atomically; serialize app transactions | Partial valid-Allow prefix, sync/rename failure, failed creation/migration, bounded reads and concurrent ordering |
| F03 — stock NSIS rejected | Accept only the four exact dormant stock hook guards and pinned generated includes | Pinned template guards/includes positive case; injected hooks/includes remain rejected |
| F04 — policy draft overwritten | Track draft revisions and request ownership; serialize saves; distinguish rejected writes | Delayed read/save, newer edits, reopen, reload and rejected-save browser cases; explicit backend response tests |
| F05 — fullscreen removal | Restore body scrolling, previous grid controls and usable focus, including indirect removal | Button removal, final-tile removal and GridStack removal browser cases |
| F06 — browser runner shutdown | Require Playwright's matching Chromium; bound the complete gate, including teardown | System Chrome reproduced 152 successful assertions followed by a shutdown timeout; the canonical browser passed all 152 and exited normally in 49.1 seconds |
| F07 — public key scanner false positive | Match only the exact public preference value, source path and generic-key rule together | The exact key is exempt; another path and a secret-like extension are detected |
| F08 — omitted unit suite | Run frontend units in active CI and Release workflows | Workflow contract checks |
| F09 — cold-cache offline build | Fetch locked target dependencies before entering the offline build wrapper | Workflow ordering checks; offline compilation locally |
| F10 — rebuilt release ZIP | Stage and upload original inspected archives plus build receipts; verify transferred bytes | Mutation, missing/extra file, source/version and checksum rejection tests |

Policy replacement shares the existing storage primitive. It orders requests
within this app process and preserves the destination on write/sync/rename
failure. It does not claim cross-process conflict resolution or durability after
sudden power loss. Native Windows filesystem behavior remains a device test.

Release workflow edits are authored and locally checked, not executed on GitHub.
The active workflow remains macOS-only and draft-only; the four-platform example
remains inactive. Full candidate assembly still requires all four targets.

## Dependency disposition

`anyhow` is updated from 1.0.102 to **1.0.103**, meeting the fix threshold for
[RUSTSEC-2026-0190](https://rustsec.org/advisories/RUSTSEC-2026-0190.html).
The refreshed local audit of the lock against the cached advisory database
reports zero vulnerability-category entries, **two unsoundness notices and 17
maintenance notices**. This is scoped evidence, not a claim of a fresh, clean
advisory database.

- **rand 0.7.3:** used only for build-time PHF generation through selectors,
  phf_codegen and phf_generator. All four target graphs omit its optional `log`
  feature; the generator uses a fixed-seed `SmallRng`. The activation conditions
  in [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html) are
  absent in this locked graph. The version remains flagged; changes to features
  or consumers require a new review.
- **glib 0.18.5:** present only in the Linux GTK3/WebKit/Tauri stack. A source
  search across 453 resolved registry package roots and application code found
  the affected `VariantStrIter`/`array_iter_str` API only in GLib itself, with no
  production consumer call. This is static evidence, not proof of safety. The
  fix in [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)
  requires GLib 0.20+, while the locked GTK/GIO/WebKit crates require 0.18.
  **This upstream issue is not patched.** Adding a direct newer GLib dependency
  would retain the affected transitive copy. A compatible upstream backport or
  a separately reviewed desktop-stack change is still needed to close it.
- **Maintenance:** the GTK3 family, fxhash, proc-macro-error and unic notices
  remain tracked. They do not by themselves demonstrate an exploitable defect.

These dispositions permit local candidate preparation and isolated device
testing. They do not support saying that all dependency advisories are fixed or
that public-release review is complete.

## Validation and next step

The complete offline Rust suite passes **357 tests**, with two opt-in benchmarks
ignored. Node unit tests pass **30 tests**. The release/helper suite passes
**132 tests**, including the installed Gitleaks regression. The targeted frontend
fixes add **13 browser regressions**. The complete canonical browser gate passes
**152 tests** and exits successfully in **49.1 seconds**, including teardown.
All-target Clippy with warnings denied passes. Source privacy and final package
inspection results belong to the final local verification record.

Build native candidates from one clean repair commit using the
[packaging commands](../packaging/README.md). The build receipts name the actual
source, version, architecture, inspected files and hashes. A missing target
receipt means that target has not been built and accepted. macOS can build both
Mac architectures here; Windows and Ubuntu require their declared native hosts
and reviewed helper inventories. Those hosts are not available in this task.

After the target artifacts exist, follow [P6](phase-6.md) for installation,
startup, settings persistence, upgrade and removal, initially with synthetic
data. Public portfolio/release preparation remains P7. No device acceptance or
live AWS validation is implied by successful source tests or package builds.
