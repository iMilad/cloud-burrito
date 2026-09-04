# P3-07 — Bounded activity history and explicit retention

Audit reads now seek from the end for the latest page, then follow an append cursor. A read consumes at most 2 MiB including continuity anchors, retains at most 1,000 records (UI: 300), limits each record to 16 KiB and returns at most 512 KiB. Partial final lines wait for a newline. Malformed/oversized entries, omitted history and forward backlog are reported separately. Ordinary replacement/truncation resets the page; cursor fingerprints are sampled continuity hints, not universal tamper detection.

Native writes use one ordered worker with a bounded 512-record queue. Overflow, write failure and unconfirmed flush set sticky audit diagnostics. Final command diagnostics follow an ordered flush barrier, with a two-second waiting bound. Potentially slow reads and retention work run off async request workers. The UI prevents overlapping polls, pauses while hidden/closed and retains earlier rows on read failure.

Retention is an explicit saved choice. The default preserves existing history; choosing bounded retention enables five known files of 10 MiB each, including the active file (50 MiB total), with oldest-file expiry. Settings displays effective mode, location, size and expiry before saving. Oversized legacy files require the explicit preservation action first. That action moves only known history into a unique sibling directory; preserved copies do not expire automatically and are outside the rotating budget. No real user history was read, rotated or moved during this work.

Settings persistence and retention activation share one transaction lock, so a failed save cannot activate pruning. Append, rotation, preservation and flush share file-operation ordering. Reader/writer opens reject symlinks/reparse points and verify file identity; parent directories remain trusted. Windows identity fallback and sampled cursors are not cryptographic replacement detection. Native platform/filesystem behavior remains pending device validation.

## Validation

339 Rust library tests pass with two opt-in benchmark entries ignored; 26 Node and 27 focused browser tests pass. All-target Clippy and the exact 17-command IPC registry pass. Fixtures cover 1/10/100 MiB history, oversized/malformed/partial records, append cursors, replacement races, symlinks, bounded rotation, preservation, queue overflow, disk failure, conflicting saves and visible-only polling.

[Raw after measurements](benchmarks/p3-audit-cli-after.json) retain five scenarios with five warmups and 30 successful recorded trials each. Reading the latest 300 records took median/p95 **14.24/14.50 ms** at 1 MiB, **14.25/14.38 ms** at 10 MiB and **14.13/14.32 ms** at 100 MiB. Every read consumed **327,808 bytes** including anchors. The unchanged 100 MiB baseline was **2,479.11/2,568.88 ms**; both use the same local debug fixture and warm filesystem method. These are reader timings, not native panel-open latency.

The same report replays CLI work after P3-06: 50 fake jobs peak at two active children (before: 50) and complete at median/p95 **447.84/454.47 ms** (before: **99.20/124.48 ms**). This deliberately exchanges burst completion speed for bounded concurrency; it is not a throughput speedup. The near-cap output retains 500 rows with an explicit limit, versus the earlier larger table.

Next: P3-08 bounded rendering, final comparison and the P4 handoff. No AWS, real CLI, native app, GitHub or release action was performed.
