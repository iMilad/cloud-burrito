# "Bring your own service" widgets — design

**Date:** 2026-07-07
**Status:** approved (user selected option 3 in `docs/widget-ideas.md`)
**Branch:** dev

## Problem

The widget set mirrors one team's CDK/CodePipeline workflow. Option 3 from
`docs/widget-ideas.md` adds genericity without a plugin SDK: two widgets where
the *user* supplies the payload — a Logs Insights query, or a read-only AWS
CLI command — and the app supplies safety, credentials, and rendering.

## Widget 1: Logs Insights query (`logs-insights`)

User writes a CloudWatch Logs Insights query; the widget runs it and renders
the result rows as a table.

**Config (persisted per widget instance):**
- Log group name (one, with the same search/picker UX as the CloudWatch Logs
  widget — reuses `fetch_groups`).
- Query text (textarea; default `fields @timestamp, @message | sort @timestamp desc | limit 20`).
- Time range preset: 15m / 1h / 3h / 12h / 24h / 3d / 7d (default 1h).

**Backend (`widgets/logs_insights.rs`), mode-dispatched like `cloudwatch_logs.rs`:**
- `groups` mode → delegate to `cloudwatch_logs::fetch_groups` (preflight
  `logs:DescribeLogGroups`).
- `query` mode → preflight `logs:StartQuery` + `logs:GetQueryResults`, then
  `StartQuery` (start = now − range, end = now), poll `GetQueryResults` every
  800 ms until status `Complete` (or `Failed`/`Cancelled` → error render), hard
  cap ~25 s. On timeout: best-effort `StopQuery` (preflighted; new registry op)
  and an error render telling the user to narrow the range.
- Result mapping: columns = union of field names across rows in first-seen
  order, `@ptr` dropped; rows keep server order. Also surface the
  `statistics` (records scanned / matched) as a footer line.

**Registry change:** add `("logs", "StopQuery")` to `APP_OPS`; snapshot the
current default as `PRE_LOGS_INSIGHTS_DEFAULT_OPS` and extend the
unedited-default upgrade chain, per the established migration pattern.

## Widget 2: Generic AWS CLI table (`aws-cli`)

User supplies a read-only `aws …` command; the app runs it (no shell) and
renders the JSON output as a table. This is the escape hatch for services the
app has no SDK crate for.

**Config (persisted):** the command string, e.g.
`aws ec2 describe-instances --query 'Reservations[].Instances[]' --region eu-west-1`.

**Validation — pure function `parse_cli_command(&str)` (TDD'd):**
1. Tokenize with `shell-words` (quotes/escapes handled; no shell ever runs).
2. `tokens[0]` must be `aws`; `tokens[1]` (service) and `tokens[2]` (operation)
   must match `[a-z0-9-]+` and not start with `-` (global options go after the
   operation, not before the service).
3. Forbidden flags: `--profile` (the app controls identity). A user-supplied
   `--output` is rejected too; the app appends `--output json`.
4. Derive the IAM-style action: kebab→Pascal (`describe-instances` →
   `DescribeInstances`), service as-is → `ec2:DescribeInstances`.

**Gate — `gate_cli(policy, service, op)` in `aws/policy.rs`:**
`guard::is_read_only(op) AND policy.decision(service, op)`. The compiled
registry layer is replaced by the structural guard as the floor — the registry
only lists SDK calls compiled into the binary, which by definition cannot cover
user-supplied commands. The YAML still only *narrows*: nothing non-read-only
can run regardless of policy content. Because the default policy is exact-ops,
every CLI action outside the registry is **deny by default**; the denial render
shows the exact line to add to `policy.yaml` (e.g. `- ec2:Describe*`). Denials
are audited as `aws-blocked` like SDK denials; allowed runs are audited too.

**Execution (backend `widgets/aws_cli.rs`):**
- Spawn the `aws` binary directly with the parsed argv — no shell involved.
- Environment: the app holds no credentials of its own — it reads the same
  `~/.aws/config` + SSO token cache as the CLI. The child gets
  `AWS_PROFILE=<ctx.profile>` and `AWS_REGION=<ctx.region>` from the widget's
  resolved context (pinned or topbar; `WidgetCtx` gains a `profile` field),
  inherits `HOME`, and `AWS_CONFIG_FILE` is already set process-wide when a
  custom config path is configured. A user `--region` flag wins over the env
  (CLI precedence).
- Timeout 30 s then kill; stdout capped at 2 MB; missing `aws` binary and
  non-zero exits render stderr's tail as the error.

**JSON → table heuristics (pure function, TDD'd):**
- Object with exactly one key whose value is an array → unwrap to that array.
- Array of objects → columns = union of keys in first-seen order; scalar cells
  verbatim, nested objects/arrays JSON-stringified and truncated (~120 chars).
- Array of scalars → single `value` column.
- Flat object → two-column key/value table.
- Anything else → pretty-printed JSON block.
- Users who want precise columns use the CLI's own `--query` (JMESPath) — no
  bespoke column-mapping UI.

## Frontend

Both widgets follow the existing 5 registration points (catalog entry,
`buildTileForWidget`, `addWidgetToGrid`, `renderWidgetTile`, `refreshHandlers`)
and the widget-config form/toolbar CSS already in place. Config is persisted
via the same mechanism the other configurable widgets use. Refresh: manual +
global refresh button, same as every other widget (no separate scheduler in
this slice). The `aws-cli` tile shows the derived action chip (e.g.
`ec2:DescribeInstances`) next to the command so the safety mapping is visible.

## Testing

- `parse_cli_command`: valid command, quoted args, forbidden flags, non-aws
  prefix, option-before-service, kebab→Pascal (incl. multi-dash), bad tokens.
- `gate_cli`: read-only + policy allow → Ok; write op denied even with `*`
  policy; policy default-deny for unregistered read-only op; invalid policy
  fails closed.
- JSON mapping: each heuristic branch.
- Registry migration: default-policy upgrade test extended for `logs:StopQuery`.

## Update (2026-07-07 evening): pinned commands for `aws-cli`

The widget gained the Pipeline Runs pin system, per user request: a Live/Pinned
tab bar (same `pipeline-*` CSS, `cli-*` JS hooks), and pin cards that save the
command **together with the profile/account/region** active when it was pinned
(`pinned_cli_commands` in the tile's persisted inputs, capped at 50, deduped by
`profile|account|region|command`). Each card reruns under its own saved context
via a per-fetch context override, shows a status badge (`N rows` / OK / Denied /
Error) plus a local run timestamp, expands on demand (first expand runs the
command), and supports drag-to-reorder with document-level mouse events.
Refresh follows the active tab; an account/region switch re-renders but never
refetches pinned-tab tiles, since pins carry their own context.

## Out of scope

- Per-widget auto-refresh interval / cron scheduling.
- Column mapping UI (JMESPath via `--query` covers it).
- Non-JSON CLI output (`--output text|table` stays rejected).
- Pagination of huge CLI outputs beyond the 2 MB cap.
