# Cloud Burrito — Design Spec

**Date:** 2026-04-28
**Author:** mk + Claude
**Status:** Superseded draft. The current app is pure Rust, has no Python
sidecar, and no longer includes the retired experimental widgets from this
draft as active widgets.
**Mockup:** `index.html` (vanilla HTML/CSS/JS click-through, in repo root)

---

## 1. Problem

The team operates ~10 AWS accounts daily. Every routine task — checking a pipeline failure, finding which CloudFormation stack owns a resource, tailing a Lambda's logs — currently takes 5 to 15 clicks across 3 to 4 AWS console pages. The console is **organized by service** (Lambda, CloudWatch, CFN, CodePipeline). Real work is **organized by workflow** ("a pipeline failed → trace it"). Every navigation step is the user translating between the two models.

The console is also slow on company-managed laptops because of corporate proxies, EDR/MDM tooling, and the per-page API call volume. A native client cannot avoid the proxy, but it can make far fewer, more targeted API calls and cache aggressively.

## 2. Solution

A **portable desktop app** (macOS + Windows 11) where:

- The unit of UI is a **widget**, not a service page.
- Each user composes their own dashboard from widgets — drag, resize, fullscreen-in-app.
- Widgets are **small Python files** that compose existing AWS tooling (boto3, Steampipe, saw, AWS CLI) and return a **declarative render spec** that the host renders.
- The library grows two ways: prebuilt widgets shipped with the app, and **AI-generated widgets** the user describes in natural language and accepts after a dry-run preview.
- All AWS access is **strictly read-only** — the app assumes a read-only role on every call. It cannot create, modify, or delete resources.

## 3. Scope and non-goals

### In scope (MVP)

- macOS and Windows 11 portable app, single binary, no install dependencies.
- 8 prebuilt widgets covering the team's daily scenarios (listed in §10).
- Drag-to-rearrange, resize, fullscreen-in-app, layout persisted per user.
- Multi-account, multi-region picker reusing existing `~/.aws/config` SSO profiles.
- Read-only-role enforcement on every adapter call.
- Per-user widget storage in OS-appropriate user data folder.

### In scope (v2)

- AI widget generation with dry-run preview.
- Team widget library backed by a git repo.
- Widget settings UI (per-tile inputs editable inline).

### Explicit non-goals

- **No write capability** — ever. Not in v1, not in v2. If a future widget needs writes, it is a separate, opt-in feature gated by a different role.
- **Not a full console replacement** — only covers operational diagnosis. Not service configuration.
- **Not a monitoring/alerting platform** — no background polling, no notifications, no on-call routing.
- **Not multi-tenant SaaS** — runs locally per user; no central server.

## 4. Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Tauri shell  (Rust + WebView2/WKWebView)                    │
│  — UI: HTML/CSS/JS frontend                                   │
│  — Layout, dashboard grid, widget host, render layer          │
│  — Account/region picker, theme, side panel                   │
│  — IPC bridge to Python sidecar                               │
└────────────────────────┬─────────────────────────────────────┘
                         │  JSON over stdio / local socket
┌────────────────────────▼─────────────────────────────────────┐
│  Python sidecar (bundled CPython 3.12)                        │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Widget runtime                                          │  │
│  │  — Discovers widget files, runs fetch(ctx)              │  │
│  │  — Schedules refresh, handles named actions             │  │
│  │  — Pipes render output to the host                       │  │
│  └─────────────┬───────────────────────────────────────────┘  │
│  ┌─────────────▼───────────────────────────────────────────┐  │
│  │  Tool adapters (the only way widgets touch AWS)          │  │
│  │   ctx.boto3            — read-only-role-scoped session   │  │
│  │   ctx.steampipe.query  — local Steampipe service         │  │
│  │   ctx.shell.run        — bundled saw / aws cli           │  │
│  │   ctx.cw_insights      — CloudWatch Logs Insights        │  │
│  │   ctx.resource_explorer — AWS Resource Explorer          │  │
│  │   ctx.cache            — host-managed TTL cache          │  │
│  └──────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
                         │
                         ▼
              Local AWS credential chain
              (SSO cache + assumed read-only role)
```

### Why these choices

- **Tauri over Electron:** smaller binary (~5 MB vs ~120 MB shell), faster startup, native WebView per OS. Cross-platform Windows + Mac is a first-class target.
- **Python sidecar over Rust-only:** boto3 is the best AWS SDK; the team writes Python; widgets are Python = familiar and AI-generatable.
- **JSON-over-stdio IPC:** simple, debuggable, no port management.
- **Bundled Steampipe + saw + AWS CLI:** zero-friction install. ~120 MB total bundle weight is acceptable for a dev tool.

## 5. The widget contract

A widget is a folder containing `widget.yaml` (declaration) and `widget.py` (logic). The folder lives in `~/.cloud_burrito/widgets/<name>/` (user) or `<team-repo>/widgets/<name>/` (team).

### widget.yaml

```yaml
name: "Pipeline Runs"
version: 1
description: "List recent pipeline executions and expand a run for action detail."
inputs:
  pipeline_name: { type: string, required: true }
refresh: 30s
permissions:
  - codepipeline:read
  - cloudformation:read
  - logs:read
```

`permissions` is informational for v1 (the role is the actual gate). v2 may use it to lint widgets that try to call APIs they didn't declare.

### widget.py

```python
def fetch(ctx):
    """Called on initial load and on each refresh tick.
       Returns a render spec dict."""
    runs = ctx.boto3.client("codepipeline").list_pipeline_executions(
        pipelineName=ctx.input("pipeline_name"), maxResults=10
    )["pipelineExecutionSummaries"]
    return {
        "render": "expandable_list",
        "items": [
            {"id": r["pipelineExecutionId"], "title": ..., "status": r["status"],
             "expand_action": "expand_run"}
            for r in runs
        ],
    }

def expand_run(ctx, item_id):
    """Named action — invoked when a row is expanded.
       Returns a render spec for the detail view."""
    ...
    return {"render": "composite", "sections": [...]}
```

**Contract rules:**

1. `fetch(ctx)` is required. It must return a render-spec dict (never throw to the host).
2. Other functions (named actions) may be referenced from the render output by name. The host calls them when the user interacts.
3. **Graceful degradation is mandatory.** When a field is missing or a downstream call fails, the widget returns a partial render with an inline `error` field, never raises.
4. Widgets must be **idempotent and side-effect-free**. They are read-only. The host enforces this via the role; the contract enforces it in code review.

## 6. Render type catalog

The host implements exactly these render types. Widgets cannot draw arbitrary HTML.

**Atomic:**

- `key_value` — definition list
- `table` — columns + rows + optional row_actions
- `log_stream` — monospaced lines, append-only with virtualized scroll
- `status_badge` — single colored badge with text
- `chart` — small line/bar chart, fixed schema
- `tree` — nested expandable nodes
- `raw_json` — fallback for prototyping

**Container:**

- `composite` — vertical sections, each a render spec
- `expandable_list` — rows that expand inline to another render spec via a named action

**Action types** (referenced from any render):

- `spawn_widget` — opens another widget pre-configured with given inputs (this is how "Tail this lambda" works)
- `open_in_console` — deep-link to AWS console (always opens a new browser tab)
- `copy_to_clipboard` — utility

That's nine render types and three action types. Closed set; AI-generation works against it.

## 7. The `ctx` object

The complete API surface a widget can call:

| Field | Type | Purpose |
|---|---|---|
| `ctx.input(name)` | function | Read declared input |
| `ctx.account` | dict | Currently selected account (`{id, alias}`) |
| `ctx.region` | string | Currently selected region |
| `ctx.boto3` | object | Pre-credentialed boto3 session, **scoped to read-only role** |
| `ctx.steampipe.query(sql, params)` | function | Parameterized SQL against local Steampipe |
| `ctx.shell.run(argv, timeout)` | function | Run a bundled binary (saw, aws cli) with safe arg-array (no shell) |
| `ctx.cw_insights.query(group, query, hours)` | function | CloudWatch Logs Insights helper |
| `ctx.resource_explorer.search(q)` | function | AWS Resource Explorer wrapper |
| `ctx.cache(key, ttl)` | function | Decorator/context manager for host-managed cache |
| `ctx.log(level, msg)` | function | Send a log line to the host's diagnostic panel |

**No file system access. No network access outside these adapters. No subprocess spawning outside `ctx.shell.run`.** This is the read-only safety boundary.

## 8. Read-only auth model

This is the most important constraint and shapes the whole adapter layer.

1. App reads `~/.aws/config` to discover SSO profiles.
2. Account picker shows configured profiles.
3. When the user picks an account, the app:
   - Resolves SSO credentials via the local SSO cache.
   - If expired, shells out to `aws sso login` (bundled aws-cli) and shows the OS browser opening.
   - **Assumes a designated read-only role** in the target account. The role name is configurable per profile (default `AWSReadOnlyAccess`, overridable in app settings).
   - Caches the assumed-role credentials for their lifetime.
4. Every adapter call uses the assumed-role session. Never falls back to the SSO root credentials directly.
5. **Steampipe** is configured with the same assumed-role credentials. **Bundled binaries** (`saw`, `aws`) are spawned with `AWS_PROFILE` and assumed-role env vars set.

This means even a buggy or malicious widget cannot cause writes — the credentials it has are physically incapable of mutating AWS. This is a stronger guarantee than relying on the contract or code review.

## 9. AI widget generation (v2)

Default provider is **BYOK Claude** (Anthropic API), with optional **Ollama** for offline/private use and optional **company gateway** for enterprises.

- API key stored in OS keychain (macOS Keychain, Windows Credential Manager) via Tauri's keyring API. Never plain text.
- Per-user setting; not shared.
- **Ollama is not bundled.** It's a user-installed runtime; the app integrates with whatever the user already has at `localhost:11434` (default Ollama port). If not installed, the option is hidden in settings.

### Generation flow

1. User opens "+ Widget" → "Describe with AI" → types prompt.
2. Host sends to LLM:
   - Prompt
   - Widget contract reference (§5)
   - Render-type catalog (§6)
   - `ctx` API surface (§7)
   - 3–4 existing widgets as few-shot examples
   - User's currently selected account/region (for context, not for the LLM to call)
   - Steampipe schema for the relevant tables (retrieved on demand)
3. LLM returns `widget.yaml` + `widget.py`.
4. Host displays the code, read-only first.
5. **Dry run** button: runs `fetch(ctx)` once in the sandboxed sidecar, shows the actual rendered output below the code.
6. User clicks **Add to dashboard** (saves to user's widget folder + adds a tile) or **Edit** (opens in a small in-app editor) or **Discard**.
7. Once accepted, the widget refreshes on its declared schedule like any other.

**No widget is added to the dashboard without an explicit user click.** Auto-add is never implemented.

A user-accepted widget can later be **promoted** to the team git repo via a one-click action that copies the folder.

## 10. MVP widget set

Eight widgets, written by hand, covering the team's daily scenarios. They double as few-shot examples for AI generation in v2.

1. **Pipeline Runs** — list of recent pipeline executions, color-coded by status, with expandable execution detail.
2. **Lambda Log Tail** — tail one CloudWatch Logs group.
3. **CFN Stack Browser** — browse stacks and expand to resources/events.
4. **Resource Reverse Lookup** — search box, returns owning stack and resource metadata where available.
5. **Errors by Stack (24h)** — bar chart of error counts grouped by log group/stack signal.

The retired experimental widgets from this draft were removed from the active
scope.

## 11. Team library (v2)

- One git repo per team. Folder layout: `widgets/<name>/widget.yaml`, `widgets/<name>/widget.py`.
- App setting: "Team library git URL" — clone path stored under app data.
- On startup, if the team library is configured, app does `git pull` (best-effort, fails soft).
- Widgets in the team library appear in the side panel under "Team" and can be added with one click.
- Promote-from-personal: a button on any user-saved widget copies its folder into the local team-library checkout, opens an editor for the commit message, and runs `git commit`. Push is manual.

PR review of widget changes happens in the team's normal git workflow.

## 12. Bundling and distribution

- macOS: signed `.app` bundle, drag-to-Applications.
- Windows: signed `.exe` installer + portable `.zip` variant.
- Bundle contents (~120 MB total):
  - Tauri shell binary
  - Embedded CPython 3.12 + the sidecar's pinned site-packages (boto3, etc.)
  - Steampipe binary + AWS plugin
  - `saw` binary (Go, ~10 MB)
  - `aws` CLI v2 binary
  - Frontend assets (HTML/CSS/JS)
- Auto-update via Tauri's updater channel, signed releases.

First launch runs a setup-check: SSO config detected? Read-only-role accessible? Bundled binaries executable? Surfaces any issue with copy-pasteable remediation.

## 13. Storage layout (per-user)

```
macOS:    ~/Library/Application Support/AWSControlCenter/
Windows:  %APPDATA%\AWSControlCenter\
```

- `widgets/` — user-saved widgets
- `team-library/` — checked-out team widget repo (v2; not present in MVP). The app uses the system `git` and the user's existing git auth (SSH key or credential helper) — no separate auth in the app.
- `dashboards/` — JSON files, one per dashboard (default + named)
- `cache/` — host cache, safe to delete
- `settings.json` — provider config, role names, theme
- `logs/` — diagnostic logs, rotated

## 14. Error handling and degradation

- **Widget throws:** host catches, renders an `error` strip in that tile, leaves dashboard otherwise functional.
- **AWS API throttled:** adapter retries with exponential backoff; widget sees the result. If retry budget exhausted, returns an inline `error` field; widget renders what data it has.
- **SSO token expired:** host pauses all widgets, prompts user, runs `aws sso login`, resumes.
- **Steampipe service down:** host attempts restart once; if still down, marks Steampipe-using widgets as `unavailable` with a "retry" action.
- **CDN/network unreachable:** mockup falls back gracefully (no drag); production app does not depend on internet at runtime.

## 15. Testing

- **Widget contract tests:** a host-side test runner that loads each widget, runs `fetch(ctx)` against a **mocked AWS layer**, and asserts the returned render spec validates against the catalog schema. Runs in CI.
- **Integration smoke tests:** opt-in, run against a real read-only AWS account on a schedule. Detect API drift.
- **Render layer tests:** snapshot tests of every render type with representative inputs.
- **Auth layer tests:** unit-test the role-assumption code with mocked STS responses, covering token expiry, role assumption failure, region mismatch.

## 16. Risks and open questions

| Risk | Mitigation |
|---|---|
| Steampipe bundling complexity (it's a server, not a library) | Spawn-on-launch, kill-on-quit. If bundling proves brittle, fall back to "user installs Steampipe" with a setup-check warning. |
| Tauri Windows packaging gotchas | Build CI on both OSes from day one. Sign with a real cert before first internal release. |
| AWS CLI bundle size (~70 MB) | Acceptable for a dev tool. If it bloats the binary too much, replace with direct boto3 calls for the few adapter use-cases that need it. |
| AI generates code that calls things unexpectedly | Read-only role makes worst-case bounded. Dry-run before accept. Code shown verbatim before save. |
| LLM cost runs away | BYOK = user's wallet. v3 may add per-team budget caps. |
| Steampipe schema changes break widgets | Pin to a Steampipe minor version per app release. |

## 17. Out of scope (deferred)

- Multi-dashboard switching (planned, not in MVP).
- Widget versioning / rollback.
- Mobile companion.
- Sharing dashboards (vs sharing widgets).
- Integration with Linear / GitHub / Jira for ticket creation from a failure trace.
- Telemetry / usage analytics.
- Alert integration (the app stays diagnostic, not paging).

---

## Appendix A — Mockup walkthrough (current)

The repo-root mockup (`index.html`) implements:

- Top bar: app name, account picker (cycles fake accounts), region picker (cycles regions), **READ-ONLY** badge (new — reflects §8), global search (⌘K), theme toggle, reset-layout, "+ Widget".
- Dashboard grid: the active widget set, drag-to-rearrange, resize from corner, fullscreen-in-app via the ⛶ icon.
- Side panel: the active prebuilt widget choices, "Describe with AI" textarea with a mock generated `widget.yaml` + `widget.py` preview.
- Layout persists in `localStorage`.

The mockup uses gridstack.js for drag/resize and is intentionally not the real architecture — it's a felt-experience prototype to validate UX before shell choice.

## Appendix B — Decisions captured this session

- Bundle everything, portable on macOS + Windows 11 (user choice, 2026-04-28).
- Strict read-only-role assumption. No write capability anywhere in v1 or v2 (user choice, 2026-04-28).
- Team of 5 → BYOK Claude default for AI, git repo for widget sharing (user choice, 2026-04-28).
- Mockup-first, then native shell. Tauri chosen as the native shell (user choice, 2026-04-28).
- Six widgets in the felt-experience mockup; eight planned for MVP.
- Drag/resize/fullscreen-in-app implemented in mockup via gridstack + custom CSS.
