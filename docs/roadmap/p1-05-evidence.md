# P1-05 — Request and result ownership

Status: complete locally, 2026-09-04. Parent: `43a168c` (P1-04), version `0.2.9`. No version or release change.

## Contract

Each `widget_fetch`, `aws_list_pipelines` and account-selection response carries an additive `_request` envelope. It echoes a bounded client request ID and, only after verification, the captured context ID, provider revision, settings revision, profile, actual account and region. IDs and revisions are opaque strings. Preverification errors never borrow metadata from the currently active account. Auth status carries the same verified context fields without a client ID. Existing result render models remain intact.

A frontend request belongs to a mounted owner, its ancestor lifetime, configuration generation and selection attempt. Explicit pinned cards/tiles have independent account lifetimes. An inherited request remains inherited through the backend; capturing a label does not turn it into a pin. Account verification clears inherited results and selectors immediately. Removed, reconfigured, superseded and replaced owners reject late success, error, cache and finalization callbacks. A→B→A is three distinct attempts even when labels match.

## Local evidence

- **130 Rust library tests pass**, with none failed, ignored or filtered. New command assertions cover ID rejection before provider work, connection/auth/widget envelope agreement, credential refresh revisions and original pinned/cleanup identity.
- **15 Node production-handler tests pass** (6 auth status, 9 request ownership).
- **17 Playwright tests pass**, including 11 deterministic ownership journeys plus the 6 existing frontend journeys. Scenarios include A→B→A, reversed same-owner responses, incorrect/missing verification metadata, failed connection, remove/reconfigure, nested detail reopen, selector races, independent pins and post-verification reload of saved CLI/previously loaded CodeArtifact tiles.
- All-target Clippy with warnings denied, Rust format, frontend syntax, whitespace and the 14-command registry pass. The repository security gate exits successfully; 13 release-helper tests pass.
- Scoped privacy scans, Gitleaks tree/history and detect-secrets report no findings. TruffleHog reports zero verified/unverified findings, with sandbox temporary-artifact PID cleanup warnings. The cached dependency audit retains **20 allowed warnings** and cannot open the package-index lock; this is not fresh release-advisory certification.

All provider/process fixtures are synthetic and fail closed for unexpected native work. The new ownership browser suite routes only localhost. Chrome was the existing local installation, selected through the optional `CLOUD_BURRITO_CHROME_PATH` test setting; no browser or package was installed. No native Tauri/AWS backend ran.

## Boundaries retained

Discarding a frontend response is **logical cancellation**, not confirmation that remote SDK/query work stopped. P1-04 supervises a CLI child when its backend context is invalidated or its caller is dropped; removing a tile alone does not send a backend cancellation request. P3 owns broader cancellation propagation and resource budgets. No AWS connection, real CLI process, personal credential source, native application launch, installer test, push, tag, release or GitHub action is part of this work.

Focused review covered the backend envelope and frontend callbacks. It found a clear/hidden-row reload regression in saved CLI and previously loaded CodeArtifact tiles; the fix preserves reload intent without retaining old data and both regression journeys pass. No remaining concrete ownership issue was found in the changed paths. P1-06 remains the next separate unit: strict input validation, safe external links, redacted diagnostics and explicit audit outcomes. Windows, Ubuntu and native macOS/provider acceptance remain open in the validation ledger.
