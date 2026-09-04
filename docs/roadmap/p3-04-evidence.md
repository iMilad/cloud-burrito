# P3-04 — Supervise remote queries and preserve cleanup truth

Logs Insights and Errors by Stack now share one lifecycle supervisor. It holds a two-slot query permit through start, polling and cleanup; StartQuery has one attempt and a bounded start wait. Cancellation retains the owned worker long enough to receive a late query ID and request StopQuery using the original verified context. Cleanup has reserved scheduler capacity and an independent 10-second budget.

Start/Get/Stop must all be allowed before admission and again under a bounded fresh policy read immediately before dispatch. Policy deletion fails closed. Polling failure, deadline and local cancellation attempt cleanup when an ID exists. Missing/lost start responses never invent an ID or silently retry. Stop accepted, denied, failed, unconfirmed and unknown are separate outcomes; only accepted stop sets the confirmation flag. Final policy/context errors preserve cleanup and recovery evidence instead of erasing it.

Unknown remote outcomes retain their query slot and a bounded recovery record. Automatic replacement is blocked. The UI offers explicit review with one selected opaque recovery token, restricted to the same verified principal/profile/configuration/account/region. Acknowledging one record releases exactly that hold; it does not prove the old query stopped and can permit overlapping remote work. Unknown records are never silently evicted. Query text and remote IDs are excluded from recovery summaries and diagnostics.

The UI keeps recovery warnings through failed, denied or mismatched recovery attempts. Ordinary Refresh cannot bypass that warning. Error counts retain successful groups and coverage, cap selected groups at 20 and ranges at seven days, and never turn a failed group into a successful zero count.

## Validation

The offline library suite passes **297 tests** with two opt-in benchmark entries ignored. Shared-query fixtures cover slow/lost start, late ID after cancellation, poll failure, failed/denied stop, queued policy revocation, unknown capacity, selected recovery and cross-principal rejection. A real command-worker test revokes policy during delayed StartQuery and verifies that its final error retains unknown cleanup and recovery. The 33 focused browser tests and 22 Node tests pass. All-target Clippy runs with warnings denied.

Native provider latency, actual scan costs and device cleanup remain unmeasured. Next: P3-05 bounded verified result reuse. No live AWS, native app, GitHub or release action was performed.
