# P4-09 — Native candidate handoff and final inspection fixes

> Phase numbering note (2026-09-04): this historical record uses P5 for device validation and P6 for the portfolio. Those future gates are now [P6](phase-6.md) and [P7](phase-7.md), following the inserted [P5 design phase](phase-5.md). Recorded results, source identities and work IDs are unchanged.

Implementation and handoff preparation completed locally on 2026-09-04. Real
candidate acceptance remains conditional on the complete four-target artifact
set. [Final evidence](p4-exit-evidence.md) records actual build outcomes and exact
remaining gates; no fixture report is a native pass.

The first ARM64 build from `297c9fc` compiled and emitted both distributables, but
the inspector correctly rejected their differing payload inventories. The ZIP
contained generated AppleDouble host metadata and retained group-write permission
on an icon where the DMG did not. The ZIP adapter now normalizes a copy to the
same write-bit policy and omits host metadata. The comparison remains strict;
original built app files are unchanged. A dedicated regression covers this policy.

Post-build native-helper verification now records effective helper hashes and
accepts only the exact pinned Linuxdeploy three-byte transformation. Nine added
fixtures reject unrelated mutation, changed inventory and skipped byte patches.
Two reviewed JSON checksum annotations were corrected for scanner syntax; the
public values themselves did not change. Temporary source paths are canonicalized
before compiler path remapping.

The [P5 handoff](../packaging/p6-handoff.md) supplies filename/checksum/installation
instructions. [P5](phase-6.md) has five ordered sections and a shared acceptance
checklist covering runtime/first launch, five product journeys, persistence,
upgrade/reinstall and removal with data retained. No device result is passed.

Both final macOS architecture pairs and a repeat ARM64 build passed inspection.
The final follow-up also disables rustup automatic installation for the optional
shell-wrapper target probe, before Python starts.

Validation: 113 Python helper fixtures pass. Full
native build, repeat comparison and scanner results are in the final evidence.
