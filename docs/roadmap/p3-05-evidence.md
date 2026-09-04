# P3-05 — Reuse only verified, bounded detail results

Result reuse is memory-only, opt-in and limited to stack detail, pipeline execution detail and package version history. It stores at most 16 entries, 4 MiB of encoded keys/data total, 512 KiB per result and 15 seconds from insertion. Hits do not extend the TTL. Partial/limited/error results, request envelopes and cleanup outcomes are not stored; transient budget counters are not replayed as new work.

Every lookup follows current configuration checks, credential/session verification and operation-policy gates. Its key includes verified account/principal, profile, region, canonical configuration path, context/provider/settings/policy generations, operation and normalized inputs. Configuration, policy and invalidated identity clear old entries. A cached result does not prove a session remains valid. The owned verification worker also rechecks context invalidation immediately before dispatch after awaiting policy.

The frontend requests reuse only for newly opened reviewed details. Manual parent Refresh performs fresh work. A hit receives the current subscriber envelope, retains the original capture time and visibly says “Cached evidence”. Invalid cache metadata cannot turn old data into a new successful result. Audit records distinguish a cache hit from execution; no keys or resource values are logged or persisted.

## Validation

305 Rust library tests pass, with two opt-in benchmark entries ignored. Actual command tests exercise account/profile/region isolation, changed principal, current deny policy and configuration invalidation; cache tests verify fixed expiry and concurrent encoded-memory/entry ceilings. All 12 focused browser tests and 22 Node tests pass; all-target Clippy passes with warnings denied. The browser tests check original timestamps, rejected freshness metadata and explicit Refresh behavior alongside the existing investigation journey. Whole native allocator/RSS use remains unmeasured; the 4 MiB limit is an encoded retained-data budget.

Next: P3-06 CLI parsing and retained-result limits. No real AWS or CLI execution, native launch, push or release action was performed.
