# P3-03 — Package identities before metadata

## Result

CodeArtifact now returns one bounded page of up to 50 package identities before requesting any versions. The frontend paints that list, then enriches the first 25 visible rows with concurrency four through the shared scheduler. Remaining details and the next page require explicit actions. A 1,000-package input remains a total browsing limit, not permission to eagerly enrich all packages.

Each row keeps its package key and DOM identity as metadata arrives. Pending, completed and failed enrichment remain distinct. Failed details do not erase versions that already arrived; retry sends only failed identities. Open version history remains lazy and shares its pending promise. Changing inputs/context or removing the tile cancels obsolete owners and rejects late updates.

The list request has a 50-name page bound and token guard. Enrichment accepts 1–25 unique, exact package identities under the same domain/repository/prefix/owner; mixed list-only fields are rejected. The frontend stops at 20 pages and detects repeated cursors. The legacy final-table mode remains for benchmark comparison and older callers.

## Request and measurement contract

A first page uses one ListPackages request. Enriching 25 non-empty rows needs at most 50 additional version/detail calls. Earlier eager behavior required 101 calls for 50 packages and 2,001 for 1,000. This changes the amount of initial work: first-page latency is useful-first-result evidence, not an equal-work claim that all 1,000 packages finished sooner.

The progressive synthetic runner records the list-return timestamp separately from completion of the first 25 metadata rows, with 5 warmups and 30 recorded trials at each dataset size. Its fake transport, inputs and source hashes are retained with the report. Browser and backend measurements are separate; neither establishes native end-to-end or AWS latency.

[Raw measurements](benchmarks/p3-progressive-backend.json) record 30/30 successful trials per progressive size. First identity-page return measured median/p95 **2.43/2.91 ms** for 50 packages and **2.41/2.59 ms** for 1,000. Completing the first 25 enriched rows measured **38.20/41.51 ms** and **37.24/38.97 ms** respectively. These debug fake-service measurements include local producer work; they exclude frontend paint and live network/provider latency. The report also retains the CLI scheduler replay (two scenarios, 30 trials each).

## Validation

282 Rust library tests pass (two explicit benchmark entries ignored by the normal suite), 22 Node tests pass, and 26 focused browser regressions pass, including five progressive package cases. All-target Clippy passes with warnings denied. Test cases preserve row identity, failed-row retry, continuation bounds, history deduplication and obsolete-input cancellation.

Next: P3-04 shared query lifecycle and honest cleanup/recovery outcomes. No AWS, native launch, push or release was performed.
