# P3-02 — Bound shared work and cancellation

## Result

One app-wide scheduler now admits at most eight ordinary SDK operations, four per verified account/region/service, two CLI children and two active query lifecycles. Two separately reserved cleanup slots avoid starvation by ordinary work. These are app resource limits, not AWS quota claims. Interactive requests receive priority with an ageing/fairness rule; pending ordinary work is bounded to 128 entries.

Every widget owns a deadline, operation budget and encoded result ceiling. Native SDK calls allow at most two attempts, with 10-second attempt and 20-second operation timeouts; StartQuery is single-attempt. Retries stay inside the same occupied permit. Default whole-widget time is 30 seconds (45 for Insights, 60 for Errors by Stack). Query cleanup integration follows in P3-04.

The request registry bounds outstanding clients/jobs, coalesces only identical work under the full verified authority/configuration/policy key, and retains a separate response envelope for each subscriber. Detaching one subscriber does not cancel another's work. The last detach signals cancellation while an owned worker remains alive to finish CLI/query cleanup. The 85-second UI fallback reports unknown cleanup rather than claiming an operation was stopped. Cancel acknowledgements only confirm a local cancellation signal.

Cold pinned contexts are shared and reservation-limited; dropped/denied work releases reservations. Verified pinned context reuse is capped at 64 entries. A policy/configuration change invalidates the relevant work before new dispatch. Independent pinned requests survive unrelated top-bar selections.

Page scans retain at most 25 successful pages, including empty pages, reject repeated/oversized continuation tokens, and enforce per-producer item limits plus a 2 MiB encoded retained-data ceiling. SDK response decoding and whole native RSS are not covered by this encoded-data limit. Reaching a bound preserves earlier complete items with visible partial coverage.

Cold identity verification also has one owned 30-second worker: cancelling its first subscriber cannot restart another subscriber's SSO/STS resolution. Fresh policy is rechecked before each verification stage; deletion fails closed.

## Validation

The Rust library suite passes **276 tests**, with two opt-in benchmark entries intentionally ignored. Synthetic tests exercise 50 identical cold pins, 50 independently scheduled/retried operations, cancellation before dispatch, queued policy changes, shared-subscriber races, reservation exhaustion/recovery, repeated tokens and oversized results. CLI termination ownership and identity-attribution regressions remain required. The production frontend suite passes 97 browser cases; 22 Node production-handler tests pass. No real provider, process, AWS request or native app was used.

## Next

P3-03 separates package identity pages from bounded visible-row enrichment. Native memory, process-tree cleanup and real-device acceptance remain unmeasured.
