# Watchlist and command line — design

**Date:** 2026-11-02
**Status:** approved in chat, part by part (model, screen and keys, loading and migration)
**Branch:** `mk/watchlist-redesign` (merge into `main` only after the user's local check)
**Mockups:** `docs/design/redesign-options/concepts.html` (`#watchlist`, `#command`),
`concept-a-watchlist.png`, `concept-b-command-line.png`

## Problem

The user works with many AWS accounts. The app's real job is to show the
**specific things** that matter to them, each from its own account. For
example: a CodeArtifact repo from one account and pipelines from two others,
on one screen, with no account switching.

The current UI is built around a default account that widgets inherit. It
also has a left rail that only scrolls to widgets, a large hero block, and a
free grid of large tiles. That structure works against the user's need.

## User needs

1. One place for chosen things. Each thing has its own profile, account and
   region.
2. Things are specific items (this pipeline, this package repo, this log
   group, this stack), not whole tools per account. More kinds can be added
   later.
3. Three moments of use:
   - After a push: did the run pass, and where did it fail?
   - When something broke: get from the failure to the log line.
   - Look up a fact: a version, a status or an ARN, then copy it.
4. Keyboard first and dense. The user works in vim.
5. Data loads when the user looks: on open, or when they press refresh. No
   background polling.
6. Read-only and safe, exactly as today.

## Non-goals

- No per-account health, "needs attention" badges or summary strips.
- No background or timed refresh.
- No new AWS operations. The compiled registry, `policy.yaml` gates, CLI
  parser and scheduler limits stay unchanged.
- No custom folders or tags for things. Group by kind only.
- No change to login. The app still never starts `aws sso login`.

## Concept

A **watchlist** (left) and an **inspector** (right), plus a vim-style
**command line** opened with `:`.

- The watchlist is a dense list of things, grouped by kind.
- The inspector shows the selected thing and drills down step by step, with a
  breadcrumb trail.
- The command line runs one-off lookups. Its result can be saved into the
  watchlist with `s`.

```
┌ top bar: filter (/) · counts · loaded time · SSO of selected row · settings (,) ┐
├──────────────── watchlist ────────────────┬──────────── inspector ─────────────┤
│ Pipelines 4                               │ payments-api › run 0042 › Build log │
│ ▌P payments-api  prod-payments  Failed 2m │ Run summary, stages                 │
│  P web-frontend  prod-core      Deploying │ Actions (selected: failed one)      │
│ Packages 2                                │ Log with the failing line marked    │
│  A acme-core     shared-tools   4.12.0    │                                     │
│ …                                         │                                     │
├───────────────────────────────────────────┴─────────────────────────────────────┤
│ key bar: only the keys that work right now  ·  ":" opens the command line here  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

## Model

A **thing** is:

| Field | Meaning |
|---|---|
| `id` | Unique, stable string |
| `kind` | `pipeline`, `package`, `logs`, `stack`, `query`, `errors`, `cli`, `stacks-search`, `lookup` |
| `context` | `{mode: "pinned", profile, account_id, region}`, always pinned |
| `inputs` | The inputs of the existing widget for this kind |
| `order` | Position inside its kind group |

Kinds map to the widgets that already exist:

| Kind | Row load (widget, inputs) | Drill-down | Loads on open |
|---|---|---|---|
| pipeline | `pipeline-runs` `{pipeline_name, max_results: 5}` | `pipeline-execution-detail` → `codebuild-log` | yes |
| package | `codeartifact-packages` `list` then `enrich` `{domain, repository, package_prefix, domain_owner?}` | `codeartifact-package-version-history` | yes |
| logs | `log-tail` (Lambda) or `cloudwatch-logs` (any group), `streams` mode, newest first | `streams` → `events` | yes |
| stack | `cfn-stack-detail` `{stack_name}` | resources and recent events (same response) | yes |
| stacks-search | `cfn-stacks` `{name_prefix, status_filter}` | stack rows → `cfn-stack-detail` | yes |
| lookup | `resource-lookup` `{query, max_results}` | match rows | yes |
| query | `logs-insights` `{log_group, query, range_seconds}` | result rows | **no** (costs money) |
| errors | `errors-by-stack` `{hours, log_group_pattern}` | bars | **no** (costs money) |
| cli | `aws-cli` `{command}` | result table | **no** (runs a process) |

Rule: things that start Logs Insights queries or a child process never run on
their own. Everything else loads on open.

**Storage.** Things are saved in the existing `~/.cloud_burrito/dashboard.json`
as tiles: `id`, `widget`, `y` = order, `config.context` (pinned) and
`config.inputs`. They have no `x`, `w` or `h`. Existing `dashboard_set`
validation already accepts this shape. The limit is 200 tiles, so 200 things.
A tile is a thing when it has no `x`/`w`/`h` and its context is pinned. Old
grid tiles always have `x`/`y`/`w`/`h`.

## Screen

- **Top bar:**
  - filter field (`/`)
  - "N things in M accounts, loaded HH:MM"
  - the SSO session of the selected row (see backend changes)
  - settings (`,`)
- **Watchlist:**
  - groups in a fixed order: Pipelines, Packages, Logs, Stacks, Saved
    commands (stacks-search, lookup, query, errors, cli)
  - each row: kind badge, name, account tag (profile name), state, age
- **Inspector:**
  - breadcrumb trail
  - the content for the kind, reusing today's renderers for tables, execution
    detail, stack detail, log stream and result states
  - coverage text collapsed under "Details" (the existing "expand result
    details" preference still applies)
- **Key bar:** context-sensitive keys.
- **Panels:** settings, the policy editor and the audit log keep their current
  behavior and open over the inspector.
- **Themes:** one visual identity in light and dark, using the existing
  `theme` setting.

## Keys

| Keys | Action |
|---|---|
| `j` `k` / arrows, `g` `G` | Move, top, bottom |
| `Enter` / `l`, `Esc` / `h` | Drill in, go back |
| `Tab` | Focus list or inspector |
| `/` | Filter rows |
| `r`, `R` | Refresh row, refresh all (things that don't load on open are excluded) |
| `y` | Copy main value (version, run ID, ARN) |
| `o` | Open the AWS console, only for links accepted by `supportedConsoleUrl()` |
| `n` `N` | Next or previous error line in a log |
| `a` | Add a thing (opens the command line in add mode) |
| `e` | Edit a thing (account, region, inputs) |
| `dd` | Remove a thing |
| `J` `K` | Move a row down or up within its group |
| `?` | All keys |
| `:` / `⌘K` | Command line |
| `s` | In a command result: save it as a thing |

Keys never fire while a text field has focus, except `Esc`. The mouse works
everywhere.

## Command line

```text
:pipe <name> [@profile] [region]     :pkg <name-or-prefix> [repo] [@profile]
:logs <function-or-group> [@profile] :stack <name> [@profile]
:stacks [prefix] [@profile]          :lookup <text> [@profile]
:errors [pattern] [hours] [@profile] :insights <log-group> "<query>" [@profile]
:aws <cli command> [@profile]
:add  :settings  :policy  :audit  :theme  :help
```

- `@profile` completes from the discovered SSO profiles (`aws_list_profiles`).
  The region defaults to the profile's region, then the default region in
  settings. Only `ALLOWED_REGIONS` are accepted.
- `Tab` completes names. A completion that needs AWS (for example
  `aws_list_pipelines`) runs only when `Tab` is pressed, for that account.
- Results appear in the inspector as "not saved". `s` saves.
- `:errors` and `:insights` show the existing cost note before starting.
- `:aws` text goes to the existing `parse_cli_command`. Its rules are
  unchanged.

## Loading

- On open, rows load top to bottom, one `widget_fetch` per row with its own
  pinned context. Existing shared verification means one SSO and STS check per
  account, not per row.
- Existing scheduler limits apply: 8 concurrent, 4 per service, queue of 128.
- No timers. `r` and `R` only.
- Each row keeps its last good result. A failed refresh shows that result with
  a "stale" label and its original time (current behavior).

## Row states and errors

| State | Shown as |
|---|---|
| not loaded / loading | muted, spinner |
| loaded | value and age |
| not run yet | for `query`, `errors`, `cli`, until `r` |
| partial | value plus the reason, under Details |
| stale | old value, "stale", original time |
| sign in needed | `aws sso login --profile <profile>` to copy (`y`) |
| blocked by policy | the blocked action, for example `logs:StartQuery` |
| failed | short reason, `r` to retry |
| needs account | profile missing from the AWS config, or never set: `e` to fix |

Errors stay on their row. The only global message is the existing sticky
warning about failed audit writes.

## Backend changes (small, read-only)

1. **`aws_auth_status` takes an optional pinned `context`.**
   - Validation allows `context` (same shape as `widget_fetch`).
   - The handler resolves it with `resolve_widget_ctx`.
   - Without `context`, behavior is unchanged.
   - The SSO token expiry is still a local cache metadata read only.
2. **New command `dashboard_backup`.**
   - No parameters.
   - If `dashboard.json` exists and `dashboard-before-watchlist.json` does
     not, copy the current file to the backup with private permissions
     (0600, through `Store`).
   - Never overwrite an existing backup. Return the status.
   - Register it in `build.rs`, the capability and the autogenerated
     permission, and keep `check-tauri-commands.py` passing.

`aws_set_account` and the inherit context stay in the backend. The new UI does
not use them.

## Migration (one time)

When the dashboard has old grid tiles and no things:

1. Call `dashboard_backup`. If it fails, stop. Keep the old file and show the
   error.
2. Convert the tiles in memory:
   - each `pinned_pipelines` entry → pipeline thing
   - each `pinned_cli_commands` entry, and a current `command` → cli thing
   - CodeArtifact tile → package thing (`domain`, `repository`,
     `package_prefix`)
   - Lambda or CloudWatch logs tile with a selected group → logs thing
   - CloudFormation stacks tile with a prefix or status filter →
     stacks-search thing
   - resource lookup tile with a query → lookup thing
   - Logs Insights tile with a group and query → query thing
   - error counts tile → errors thing
   - A tile's pinned context is kept. An inherited context gets
     `default_profile` with its account and region. If that isn't set, the
     thing gets "needs account".
   - A tile with nothing specific selected is dropped. It was only a browser.
3. Save the things with `dashboard_set` and show once: "Converted N widgets
   into M things. Backup saved as dashboard-before-watchlist.json."

## Removed

- The GridStack dashboard and `frontend/vendor/gridstack-*`.
- Classic view, the Studio rail and hero, and the appearance presets
  (Original, Precision, Paper, Night Shift). The light and dark themes take
  their place.
- The top-bar default account and region pickers. Accounts are chosen per
  thing.

## Frontend structure

Plain scripts with no build step, so the `file://` browser demo keeps working
(ES modules don't load from `file://`). There is one file per unit and they
share one namespace:

| File | One job |
|---|---|
| `things.js` | Thing model, storage mapping, migration |
| `watchlist.js` | List rendering, groups, selection, row states |
| `inspector.js` | Drill-down stack, breadcrumb, reuse of renderers |
| `commands.js` | `:` parser, completion, command results |
| `keys.js` | Key map and focus rules |
| `renderers.js` | Existing result renderers moved out of `app.js` unchanged |
| `bridge.js` | `invoke` wrapper, mock data in browser mode |

`app.js` shrinks to boot and panels (settings, policy, audit).

## Testing

- No live AWS or credentials, as today.
- **Playwright (browser mode):**
  - new watchlist, inspector, keys, command line and migration specs, with
    mock data from several accounts
  - the 15 spec files tied to GridStack and Studio are replaced
  - the security, result-state, request-ownership, policy, settings and audit
    specs are kept and adapted
- **Node unit tests:** command parser, key map, thing mapping and migration.
- **Rust tests:**
  - `aws_auth_status` with and without a pinned context
  - `dashboard_backup`: no file, first backup, existing backup never
    overwritten, permissions 0600
- **All existing gates must pass:** `security-check.sh`, the CI job set and
  Clippy.
- The user builds the branch and checks it with real accounts before the merge.

## Build order (each step usable)

1. Watchlist and inspector for pipelines and packages, the bridge, keys and
   storage.
2. Logs and stacks, including stacks-search and lookup.
3. Command line, the saved commands (query, errors, cli) and migration with
   `dashboard_backup`.
4. `aws_auth_status` context, removal of the old dashboard, the full test
   rewrite, then the user's check, then the merge.
