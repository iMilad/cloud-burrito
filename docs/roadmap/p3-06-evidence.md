# P3-06 — Bound CLI decoding and retained results

The existing P1 process boundary remains authoritative: the exact 18 reviewed reads, frozen verified credentials, isolated child environment, 2 MiB stdout and 256 KiB stderr, cancellation and owned terminate/reap completion. This phase adds limits after collection without launching any real CLI.

CLI stdout must be valid UTF-8 and complete JSON. A bounded deserialization visitor stops at 50,000 nodes/key slots and depth 32. Tables retain at most 500 rows, 32 columns, 256 characters per column name and 4,096 characters per string preview. Nested previews remain compact. Permitted full detail values are keyed by original row/column; larger withheld values direct the user to narrow the command.

The encoded widget payload is capped at 512 KiB. Row/column/display omissions are explicit in coverage. Mixed or raw JSON is returned whole only if it fits; sliced invalid JSON is never presented as complete data. Parsing and display bounds are separate from the scheduler's two active CLI children and from native process RSS, which remains unmeasured.

## Validation

The Rust library suite passes **311 tests**, with two opt-in benchmark entries ignored; all-target Clippy passes with warnings denied. The offline suite covers invalid UTF-8, structural budgets, wide tables, row/byte ceilings, full versus withheld values, and ten rounds of 50 fake jobs mixing overflow, parse failures and cancellation. It checks exact dispatch counts, a maximum of two active fake children and recovered permits after each round. Existing identity, exact-parser and process-cleanup tests remain required. No real executable, AWS or user credential source is involved.

Next: P3-07 bounded audit reading, ordered writes and explicit retention choice. P3-08 adds the intentional full-value UI and bounded table rendering.
