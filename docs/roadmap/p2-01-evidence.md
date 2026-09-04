# P2-01 — Truthful persistence

Status: complete locally; automated checks passed, native evidence pending. Base: `efef107`, version `0.2.9`. No release metadata changed.

## Result

Settings and dashboard loads distinguish a missing first-run file from malformed, oversized or unreadable data. Corrupt data is preserved; merely opening the app cannot replace it with defaults. A saved empty dashboard remains intentionally empty. Supported settings and tile/pipeline/CLI pins survive reopening.

A shared injected store bounds JSON reads/writes, serializes each file's replacement, writes a unique exclusive temporary file, syncs it, and renames only after successful completion. Serialization, directory creation, create, partial write, sync and rename failures report failure and preserve the previous destination. Temporary cleanup owns only files it created. Newer save intent supersedes older pending intent, including when the newer serialization fails; that case preserves the previous durable file rather than resurrecting the older draft.

IPC returns fixed storage error categories without paths or file contents. Unreadable app settings invalidate active and pinned contexts and block all provider-entry paths; recovery requires a fresh verification. The UI keeps drafts/current layout on failure, blocks autosaves after a failed load, and provides Retry load, explicit replacement/reset, and Retry save controls. Reset writes starter descriptors; it no longer overloads an empty dashboard as a reset marker.

## Validation

- 177 locked, offline Rust library tests pass, including deterministic filesystem faults, save-order barriers, reopening/pin preservation, command error propagation and blocked provider access.
- All-target Clippy with warnings denied passes.
- 15 Node production-handler tests pass.
- 40 production-frontend browser tests pass: 13 persistence/browser-mode and 27 context-ownership/rendering-security cases. Controlled pending saves preserve drafts across reopen; newer edits never receive a false Saved message.
- Tracked/new-file privacy and current-tree Gitleaks scans pass. The full phase security/advisory gate is scheduled for P2-07.

Tests use disposable explicit paths and synthetic bridge responses; no personal storage, credentials, AWS, CLI subprocess or native application was accessed. Browser tests use the installed Chrome against localhost only.

## Limits / next

Ordering is within a shared app runtime, not a cross-process file lock. File sync plus replacement does not certify power-loss durability or parent-directory sync. A failed cleanup may leave its own temporary file. Windows/macOS/Linux native replacement and installation journeys remain P5 checks. P2-02 next establishes authoritative defaults, supported region selection, persistent theme and save feedback.
