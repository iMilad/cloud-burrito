# P3-01 — Reproducible synthetic baseline

> Phase numbering note (2026-09-04): this historical record uses P5 for device validation and P6 for the portfolio. Those future gates are now [P6](phase-6.md) and [P7](phase-7.md), following the inserted [P5 design phase](phase-5.md). Recorded results, source identities and work IDs are unchanged.

**Complete locally.** Local instrumentation and measurement only. Production behavior is unchanged in this unit. The source baseline is `6c0377e` (version `0.2.9`); each artifact records the source revision, production-file hashes and the uncommitted instrumentation state used for its run.

## Method and reproducibility

- Backend: real Rust widget producers and audit reader, debug profile, locked offline dependencies, fixed fixture revision/seed, five warm-ups and thirty recorded trials per scenario. SDK transport and CLI runner are synthetic; unexpected real provider, HTTP or executable access fails closed.
- Browser: the production frontend in installed headless Chrome, a localhost-only server and synthetic IPC. Fresh browser processes measure startup; other scenarios use fresh contexts in a reused browser. The runner blocks external requests. No browser is downloaded.
- Scenarios include six dashboard tiles, fifty pins, CodeArtifact sizes of fifty and one thousand, tables of one hundred/one thousand/ten thousand rows, long cells and keyboard details, delayed responses, one recorded five-second outlier, denied/throttled/failed pages, and one/ten/one hundred MiB audit histories.
- Raw warm-up and recorded trials, failures, median, empirical nearest-rank p95 and maximum are retained. Floating metrics are serialized to six decimal places, below the useful timing resolution; integer counts are unchanged. Table add/refresh/remove memory observations form a separate ten-cycle series. No OS cache flush or forced browser GC is performed.

Replay with the same installed toolchain and browser:

```sh
python3 scripts/performance-baseline.py --suite backend --output /private/tmp/burrito-backend.json
python3 scripts/performance-baseline.py --suite browser --output /private/tmp/burrito-browser.json \
  --chrome '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
```

The browser command uses local binding and Chrome automation; it does not launch Tauri. Output paths are arbitrary local artifacts, not application data. The ignored Rust measurement entry points are separate from ordinary regression tests.

## Recorded baseline

[Backend raw measurements](benchmarks/p3-before-backend.json) include thirteen scenarios with thirty measured trials each. Representative debug-harness timings:

| Scenario | Median | p95 | Interpretation |
| --- | ---: | ---: | --- |
| CodeArtifact, 50 packages, immediate fake responses | 51.73 ms | 53.09 ms | One final result after 101 HTTP attempts |
| CodeArtifact, 1,000 packages, immediate fake responses | 1,013.65 ms | 1,044.11 ms | One final result after 2,001 HTTP attempts |
| Audit, latest 300 entries from 1 MiB | 24.42 ms | 24.68 ms | Existing forward scan |
| Audit, latest 300 entries from 10 MiB | 247.67 ms | 249.29 ms | Existing forward scan |
| Audit, latest 300 entries from 100 MiB | 2,479.11 ms | 2,568.88 ms | Existing forward scan |
| CLI, valid JSON near the stdout cap | 59.83 ms | 60.48 ms | Actual parser/model with a fake runner |
| CLI, 50 concurrent synthetic jobs | 99.20 ms | 124.48 ms | Peak 50 fake runner jobs; no aggregate gate yet |

The CLI fifty-job fixture returns 12,500 total rows and approximately 2.96 MB of encoded responses. That is retained response data, not an RSS measurement. Failure fixtures intentionally produce partial results and remain in the report; they are not discarded as measurement failures.

[Browser raw measurements](benchmarks/p3-before-browser.json) contain nine scenarios, each with five warm-ups and thirty successful recorded trials, using Chrome `152.0.7977.82`. All three separate ten-cycle table series complete successfully.

| Browser workload | First response to useful result, median / p95 | Refresh completion, median / p95 |
| --- | ---: | ---: |
| 100 table rows | 37.60 / 38.70 ms | 89.60 / 101.80 ms |
| 1,000 table rows | 223.70 / 227.20 ms | 309.50 / 325.50 ms |
| 10,000 table rows | 2,114.60 / 2,155.60 ms | 2,541.15 / 2,590.20 ms |

Normal filter paint p95 is 32.50 / 33.20 / 33.70 ms respectively. This supports reducing full-table construction in P3-08; it does not show that ordinary filtering is currently slow. Fresh-browser launch to the interactive six-tile shell has median 635.33 ms and p95 653.34 ms with immediate fake responses; this is not native application startup or verified connection time.

## Verification

- 249 Rust library tests pass; two opt-in benchmark entry points remain ignored during ordinary tests and pass when explicitly replayed.
- All-target Clippy passes with warnings denied. The unchanged frontend's 17 Node tests pass and the exact 15-command IPC registry remains aligned.
- All 270 recorded browser trials pass, plus the separate ten-cycle series at each table size. The earlier smoke run also passed all nine scenarios.
- New source/artifacts pass the scoped release-privacy scan and the redacted current-tree Gitleaks scan. No production behavior or dependency has changed.

The serialized backend and browser artifacts retain 660 measured trials across twenty-two scenarios, alongside warm-ups and the separate browser cycle series. Native evidence remains outside this count.

## Measurement boundaries

These are small-sample local synthetic observations, not live AWS latency, quota, installability or native release-performance claims. Resource preflight counts are not SDK wire attempts. SDK retries are disabled in the fixed transport fixture; the recorded throttled response tests a failure path, not a production retry policy. Fake runner concurrency is not actual process count.

Native Tauri launch-to-interactive, real connect-to-verified time, installed WebView versions, total backend-plus-webview RSS, actual child/pipe memory, native idle CPU and native retained memory remain **unmeasured**. CPU/RAM/model fields unavailable to the metadata probe and power mode remain explicitly unavailable. Cumulative child peak RSS can include Cargo, the compiler and fixture construction; it is not an application memory budget. Browser CDP heap samples exclude native allocations and cannot prove a memory leak or its absence.

The comparable synthetic baseline permits P3-02 through P3-08 implementation. P4 packages and P5 device journeys must provide the missing native evidence before product-level support or performance claims.

No AWS connection, real AWS CLI, credential-source inspection, native Tauri launch, dependency installation, push, tag or release is part of this unit.
