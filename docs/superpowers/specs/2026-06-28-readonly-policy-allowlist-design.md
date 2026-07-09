# Read-only policy allowlist — design

**Date:** 2026-06-28
**Status:** approved (design)
**Branch:** phase-1-shell-sidecar

## Problem

The app is read-only by construction, but the SSO credentials it runs with are
admin-scoped. The user wants a self-serve, file-based way to *narrow* what the
app may call — an IAM-style YAML policy they edit themselves — so they don't
have to ask Claude to adjust the Rust allowlist each time. Removing a service
from the policy must make the corresponding widget fail with a clear
"missing permission" message.

## Invariant (non-negotiable)

The YAML policy can only **narrow**, never widen. Effective decision:

```
allowed(service, op) = registry::contains(service, op)
                       AND guard::is_read_only(op)
                       AND policy::allows(service, op)
```

The structural read-only floor (`aws/guard.rs`, only read ops compiled into the
binary) remains the ultimate authority. Even `cloudformation:DeleteStack` in the
YAML does nothing — there is no delete code to run.

## Policy file

- **Path:** `~/.cloud_burrito/policy.yaml` (beside `settings.json`).
- **Shape:** IAM-statement style.

  ```yaml
  statements:
    - effect: Allow
      action:
        - cloudformation:Describe*
        - logs:*
    - effect: Deny          # overrides Allow above
      action:
        - logs:StartQuery
  ```

- **Evaluation (standard IAM):** explicit `Deny` > matching `Allow` >
  default-deny. Actions are `service:Action`; glob `*` and `?` supported.
  Matching is case-sensitive on the operation, service compared lowercase.
- **First run / missing file:** auto-write the exact-ops default — one `Allow`
  statement listing every `(service, op)` in the op registry (below). This is
  the tightest policy that still yields a working dashboard; user deletes lines
  to scope down.
- **Fail-closed:** unparseable or structurally-invalid YAML ⇒ deny everything,
  and surface the parse error in Settings. No silent fallback to last-good or
  allow-all.

## Components

### `src-tauri/src/aws/policy.rs` (new)
- `decision(service, op) -> Decision` (`Allow` | `Deny`), applying the rules above.
- Parses `policy.yaml` via a maintained YAML crate (`serde_yml` or `yaml-rust2`;
  **not** unmaintained `serde_yaml` — pin in the plan).
- Re-read on each widget fetch (file is tiny); no caching complexity, no restart.
- Validation surface: `load() -> Result<Policy, PolicyError>` where `PolicyError`
  carries a human message + line where available.

### Op registry (single source of truth)
A `&[(service, op)]` constant listing every AWS service call the app can make. Consumed by:
1. the default-policy generator, and
2. a coverage test (below).

Current contents:
`sso:GetRoleCredentials`, `sts:GetCallerIdentity`, `cloudformation:ListStacks`,
`cloudformation:DescribeStackResources`, `cloudformation:DescribeStackEvents`,
`logs:DescribeLogGroups`, `logs:StartQuery`, `logs:GetQueryResults`,
`logs:FilterLogEvents`, `logs:GetLogEvents`, `codepipeline:ListPipelines`,
`codepipeline:ListPipelineExecutions`, `codepipeline:ListActionExecutions`,
`codebuild:BatchGetBuilds`, `resourcegroupstaggingapi:GetResources`.

### Enforcement chokepoint — `widgets/mod.rs`
- Widget `fetch` signature becomes `async fn fetch(&WidgetCtx) -> Result<Value, Value>`
  (`Err` carries a render shape).
- Replace each `ctx.audit_aws(svc, op);` with `ctx.preflight(svc, op)?;`.
  `preflight` audits **and** enforces:
  - structural fail → audit `aws-blocked` (reason `not-read-only`) → `Err(render)`
  - policy deny     → audit `aws-blocked` (reason `policy`)        → `Err(render)`
  - allow           → audit `aws`                                  → `Ok(())`
- Dispatcher collapses both arms: `fetch(ctx).await.unwrap_or_else(|deny| deny)`.

**Scope tradeoff (accepted):** if a widget makes call A (allowed) then B (denied),
the whole tile shows B's denial — no partial results.

### Error surfacing — frontend
- New closed render shape `permission_denied` in `app.js` dispatcher + catalog docs.
- Renders a lock tile: `missing permission: <action> — not allowed by your
  read-only policy. Edit policy.yaml in Settings to enable it.`
- Worded to distinguish from a real AWS `AccessDenied` (IAM-side).
- Denials also show in the audit panel as `aws-blocked`.

### Settings UI ("Both")
A "Read-only policy" section:
- file path + validation status (`✓ valid — N actions` / `✗ line 4: …`),
- rendered summary of the active allow set,
- `Reload & validate` button (re-reads disk for vim edits),
- editable textarea with `Save` (validate → write; inline errors on failure).

Tauri commands: `policy_get` (raw text + status + summary) and
`policy_set(text)`. Reload is implemented by calling `policy_get`.

## Testing
- `policy.rs`: allow, default-deny, glob match, Deny-overrides-Allow,
  invalid/unreadable policy → deny-all, `service:Action` parsing.
- Coverage test: the auto-generated default allows **every** op in the registry.
- `get/save_policy` round-trip; fail-closed on invalid input.

## Out of scope
- Partial widget results across mixed allow/deny calls.
- IAM `Resource`/`Condition` fidelity (action-level only).
- Validating actions against the real AWS action catalog (only syntax + registry
  membership warnings).
