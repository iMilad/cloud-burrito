# Read-only Policy Allowlist Historical Plan

This file is retained as a historical marker for the read-only policy work that
started on 2026-06-28. The detailed step-by-step implementation checklist was
removed because it described an older backend shape and AWS call surface that no
longer matches the app.

Current source of truth:

- `src-tauri/src/aws/policy.rs` owns the compiled AWS call registry, policy
  parsing, default policy generation, and fail-closed gate.
- `src-tauri/src/widgets/mod.rs` owns widget-level preflight checks and denied
  render payloads.
- `src-tauri/src/commands.rs` owns command-level AWS service-call checks.
- `docs/superpowers/specs/2026-06-28-readonly-policy-allowlist-design.md`
  describes the current design.

Current behavior:

- Every AWS service call must be present in the compiled registry.
- Every registered operation must pass the structural read-only guard.
- Every allowed operation must also be allowed by `policy.yaml`.
- Denied calls are audited as blocked calls before the app returns a missing
  permission response.
- Unreadable or invalid policy state fails closed.
