# P2-02 — Predictable settings

> Phase numbering note (2026-09-04): this historical record uses P5 for device validation and P6 for the portfolio. Those future gates are now [P6](phase-6.md) and [P7](phase-7.md), following the inserted [P5 design phase](phase-5.md). Recorded results, source identities and work IDs are unchanged.

Status: complete locally; automated checks passed, native evidence pending. Base: `65a6648`, version `0.2.9`.

## Result

Backend-resolved settings/defaults and the supported beta region catalogue drive desktop selectors. Valid older files with an unsupported default region or theme remain readable and visibly require correction; execution cannot silently substitute a different region. Blank/reset semantics are resolved once in Rust. The settings IPC is an explicit full-form replacement: all five editable fields are required, blanks request their defaults, and omitted fields are rejected before writing. This prevents a partial theme request from silently resetting a custom config path or SSO constraint. Older stored files may still omit newer fields. Theme is explicit light/dark: the topbar previews a draft and normal Settings Save makes it durable, with visible unsaved/failed feedback.

A saved SSO session name is an additional consistency constraint on the selected profile, not a replacement credential provider. It applies equally to active and pinned contexts. Config-path/session-constraint changes invalidate affected contexts; preference-only changes do not discard a valid credential context.

## Validation

- 186 locked, offline Rust library tests pass, including settings metadata/error contracts, persisted theme and blank defaults, unsupported region preservation with zero provider execution, explicit supported-region recovery, SSO constraint conflicts and active/pin revision fencing.
- All-target Clippy with warnings denied passes.
- 15 Node production-handler tests pass.
- All 48 production-frontend browser cases pass, including authoritative startup ordering, invalid preferences, theme save/reopen/failure, pinned selectors, full-form save feedback, and credential-save/account-switch ordering. The original nested pipeline/log ownership journey passes unchanged. Final run: one installed-Chrome worker, 48 passed in 45.6 seconds, exit 0. A prior two-worker run passed all assertions but timed out during runner teardown; subsequent local browser gates use one worker.
- Tracked/new-file privacy and current-tree Gitleaks scans pass.

The new save-status notice initially moved a button between mouse press and release. Pointer tracing reproduced the missed click; feedback now stays outside document flow and does not intercept pointer events.

Only synthetic identities, disposable storage paths and localhost browser bridges are used. No AWS, credential inspection, native app launch, subprocess CLI, remote Git, or release action was performed.

## Limits / next

No new region, provider, service, or credential mechanism is introduced. Native WebView/device restart and real provider behavior remain P5/later integration evidence. P2-03 next provides explicit first-run and recovery states and optional CLI availability without spawning a process.
