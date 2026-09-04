# P3 exit — Local performance work complete

P3-01 through P3-08 are implemented and separately committed locally. The language and architecture remain Rust, Tauri and vanilla JavaScript. P4-01 is next: freeze the unsigned artifact/compatibility contract before preparing platform packages. Native support and native performance remain unverified.

## Review and rollback map

| Unit | Local commit | Reviewable outcome |
| --- | --- | --- |
| [P3-01](p3-01-evidence.md) | `d29396a` | Reproducible unchanged synthetic baseline |
| [P3-02](p3-02-evidence.md) | `121fe27` | Bounded scheduling, coalescing and subscriber-owned cancellation |
| [P3-03](p3-03-evidence.md) | `e43f8cf` | Package identities before bounded enrichment |
| [P3-04](p3-04-evidence.md) | `273b9b0` | Query lifecycle supervision and explicit uncertain-cleanup recovery |
| [P3-05](p3-05-evidence.md) | `fa16d57` | Short-lived detail reuse within verified identity and policy |
| [P3-06](p3-06-evidence.md) | `99307e9` | CLI parser, retained-value and aggregate admission bounds |
| [P3-07](p3-07-evidence.md) | `8bcb606` | Bounded audit reads, ordered writes and explicit retention |
| [P3-08](p3-08-evidence.md) | Containing commit (`P3-08:` subject) | Bounded tables, interruptible filtering and final phase evidence |

These commits are ordered dependencies. A unit gives a precise review/bisection reference; reverting one in isolation can require adjustments to dependent units. Earlier phase commits were not rewritten.

## Measurement method

All timing comparisons use the same local machine and synthetic fixtures: debug Rust producers, disposable files, fake AWS identity/HTTP/process boundaries, and the unbundled production frontend in installed headless Chrome. There are five warmups and 30 recorded trials per scenario. Failures, maxima and raw trials are retained. Each artifact records revision, dirty-tree status, source fingerprints, machine/toolchain metadata and method limits. Dirty snapshots are identified by fingerprints; a HEAD value alone is not the measured source. The final browser and CodeArtifact fingerprints were rechecked against the production files in this containing commit, with no mismatches.

The browser runner now waits for asynchronous matching to finish before counting two paint frames. Immediate handler time remains separate. The first after-run browser warmups began as the last regression process finished; recorded trials followed regression completion. No concurrent Cargo build was used during recorded timing trials. Host background activity and power mode were not controlled, so small differences are observations, not stable performance guarantees.

Browser heap samples are CDP JavaScript heap, not total RSS or a proven peak; three ten-cycle series have no forced GC and include fixed fixture arrays. Worker termination and bounded DOM tests establish lifecycle behavior, not a complete absence of retained objects. The backend harness high-water mark includes Cargo/test-process costs and cannot establish native process memory.

## Before/after observations

Times below are median / empirical p95 in milliseconds. Each browser row has 30 successful recorded trials; all 270 browser trials and all three ten-cycle series completed without fixture or page errors.

| Synthetic path | Before | After | Meaning |
| --- | --- | --- | --- |
| 100-row first response → useful table | 37.60 / 38.70 | 40.45 / 41.40 | Small-table rendering slightly slower |
| 1,000-row first response → useful table | 223.70 / 227.20 | 41.30 / 42.60 | First 100 rows mounted; remaining returned rows pageable/searchable |
| 10,000-row first response → useful table | 2,114.60 / 2,155.60 | 56.20 / 57.90 | Same returned dataset, bounded initial DOM |
| 100-row refresh → result | 89.60 / 101.80 | 106.40 / 122.50 | Small refresh has additional interaction/resource lifecycle work |
| 1,000-row refresh → result | 309.50 / 325.50 | 106.25 / 107.70 | Bounded replacement rendering |
| 10,000-row refresh → result | 2,541.15 / 2,590.20 | 138.70 / 140.80 | Bounded replacement rendering |
| 10,000-row normal filter handler | 19.20 / 19.80 | 0.10 / 0.20 | Matching moved off the main thread |
| 10,000-row normal filter → painted matches | 32.35 / 33.70 | 82.05 / 83.10 | Matching takes longer overall due to debounce/worker; pathological regex is interruptible |
| 300 audit rows from 100 MiB history | 2,479.11 / 2,568.88 | 14.13 / 14.32 | Bounded filesystem reader; 327,808 bytes read including anchors |
| 50 fake CLI jobs, all completed | 99.20 / 124.48 | 447.84 / 454.47 | Deliberate admission tradeoff: peak children 50 → 2 |
| Fresh Chrome process → interactive shell, immediate six-tile fixture | 635.33 / 653.34 | 646.68 / 673.04 | No startup speedup established; this is not native application launch |

The normal filter's p95 matching-plus-paint times are 81.90, 82.30 and 83.10 ms for 100/1,000/10,000 rows. Those synthetic cases meet the provisional 100 ms filter target; this does not establish all-input refresh/cancel latency. The three table first-useful cases meet the synthetic 250 ms target. No hardware input or native IPC latency was measured.

Page JavaScript heap did **not** show a general improvement. The settled-sample median for 100/1,000/10,000 rows changed from **3.09/6.02/5.91 MiB** to **3.38/8.57/29.18 MiB**. The new worker path retains searchable source data while DOM is bounded. GC timing was not controlled, CDP page samples do not establish total worker/native memory, and these numbers do not attribute every allocation. In the ten-cycle 10,000-row series, post-removal samples ranged **19.73–42.86 MiB** (first 42.69, last 42.76); the smaller series also fluctuated. The native/retained-object memory gate remains open and no memory-reduction claim is made.

The final CodeArtifact replay retains ten scenarios × 30 recorded trials. Deliberate denial, throttling and page-failure cases remain partial (90 trials); the other 210 trials succeeded. The selected five-second outlier remains in the raw data (after maximum 5,005.47 ms). SDK retries are disabled in this timing fixture; bounded retries are covered separately by correctness tests.

| CodeArtifact producer work | Before median / p95 ms | After median / p95 ms |
| --- | --- | --- |
| Legacy full enrichment, 50 packages / 101 calls | 51.73 / 53.09 | 89.58 / 98.59 |
| Legacy full enrichment, 1,000 packages / 2,001 calls | 1,013.65 / 1,044.11 | 1,929.88 / 2,270.64 |
| Progressive first 50 identities from 50-package dataset | No progressive path | 2.39 / 2.97 |
| Progressive first 50 identities from 1,000-package dataset | No progressive path | 2.24 / 2.65 |
| First 25 metadata rows complete, 50-package dataset / 51 calls total | Different initial workload | 40.54 / 43.61 |
| First 25 metadata rows complete, 1,000-package dataset / 51 calls total | Different initial workload | 39.79 / 41.97 |

Legacy full enrichment is slower in this fixture after the added scheduling/authorization/accounting checks. The timing includes synthetic policy-file preflight writes; it is not isolated AWS or network cost, and no single cause was independently profiled. The normal frontend now delivers identities first and enriches only its bounded initial set. This reduces initial work and latency; it does **not** demonstrate faster completion of the same fully enriched 1,000-package workload. Backend and browser timings must not be added into a native end-to-end claim.

Raw sources: [unchanged backend](benchmarks/p3-before-backend.json), [unchanged browser](benchmarks/p3-before-browser.json), [after browser](benchmarks/p3-after-browser.json), [after CodeArtifact](benchmarks/p3-after-codeartifact.json), [after audit and CLI](benchmarks/p3-audit-cli-after.json), and the earlier [P3-03 checkpoint](benchmarks/p3-progressive-backend.json).

## Resource and correctness contracts

- Ordinary work admits eight app-wide and four per account/region/service, with bounded pending work and priority fairness. CLI children and locally owned query lifecycles each have two slots; cleanup has reserved capacity. Fake boundaries validate these limits.
- Concurrent subscribers may share equivalent work under the same complete verified context; cancelling one subscriber does not abort others. Queued last-subscriber cancellation prevents dispatch. Identity/configuration/policy changes invalidate affected work and cache entries.
- Only three detail paths opt into a 15-second, 16-entry, 4 MiB in-memory cache. Fresh verification and policy still apply, manual refresh bypasses reuse, and partial/error results are not cached.
- Encoded response/page limits, CLI parser depth/node/row/cell limits and scheduler admission constrain specific retained structures. They exclude SDK deserialization buffers, allocator overhead and whole-process RSS. SDK response bodies are not universally bounded before decoding.
- Query cancellation acknowledges local work. Late-start waiting and cleanup can extend beyond the ordinary deadline. Unknown queries retain recovery holds in memory for the current app lifetime. Explicit acknowledgement permits replacement and can allow remote overlap; it does not prove the previous query stopped. No live query cost or provider behavior was measured.
- Audit reads/writes have bounded work queues and explicit failure diagnostics. Default preservation does not cap historical disk use. Optional bounded retention expires the oldest known file; explicitly preserved archives are outside its budget. Sampled cursors are continuity hints, and parent directories remain trusted.

## Final validation and security scope

- **339 Rust library tests pass**, with two opt-in timing entry points ignored in ordinary tests; the main/doc targets also pass. All-target Clippy passes with warnings denied, and Rust formatting passes.
- **126 distinct browser cases verified:** the full suite passed 124, then all 17 affected cases passed after the two fixes, followed by 33 ownership/result-state regressions. **30 Node cases** and **22 Python helper cases** pass. Phase evidence retains each earlier checkpoint's historical totals.
- The exact **17-command** registry, frontend/worker/script syntax, release metadata (`0.2.9`), whitespace and repository security gate pass. The gate scanned 220 tracked and nine untracked source/report files for release privacy before the final documentation-only update; the staged final tree is checked again before commit.
- Gitleaks reports no findings in the current tree or local Git history. Detect-secrets reports **272 cryptographically verified public benchmark fingerprints and no remaining unverified findings**. Its new narrow report helper only recognizes the generator's canonical metadata fields, the exact scanner token hash, a real local commit and a SHA-256 match to current or bounded exact-path historical source. Measurement fields, other detector types, unknown hashes and unproved snapshots remain fatal. No broad benchmark exclusion was introduced; credential/account privacy checks are unchanged.
- TruffleHog reports zero verified/unverified findings over 6,333 chunks. It logged sandbox PID/temp-cleanup warnings but completed with exit zero. These are scoped repository scans, not proof that the machine is credential-free.
- Dependency audit used the cached advisory database with network fetching disabled. It retains **20 allowed warnings** and a sandbox crates-index lock warning; it does not certify fresh advisories. The unchanged dependencies still require the later fresh release review.

The repository security gate exited zero. No failing functional test is being deferred; native behavior, current advisories and memory claims retain the explicit later gates above.

## P4/P5 handoff

Start [P4-01](phase-4.md#p4-01--freeze-the-artifact-and-compatibility-contract) with this containing commit and version `0.2.9`. Keep unsigned, identity-free artifacts, and build every candidate from the same reviewed source. Record architecture, OS/library baseline and prerequisites before calling any target supported.

P4/P5 still need packaged worker/CSP behavior in WKWebView, WebView2 and WebKitGTK; native startup, idle CPU, backend-plus-webview memory and retained-object checks; actual process-tree/CLI cleanup; filesystem identity/rotation behavior; and install, restart, upgrade, uninstall and core journeys on each declared device. Live SSO renewal, STS attribution, throttling and query cleanup require later explicit authorization and a selected test context. Fresh advisory and final release/privacy review are also later gates.

No push, remote Git/GitHub operation, AWS call, real AWS CLI, credential-source inspection, native launch, installation, tag, release or publication was performed. Local implementation approval does not authorize those actions.
