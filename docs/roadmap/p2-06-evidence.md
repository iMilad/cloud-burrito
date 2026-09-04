# P2-06 — Honest beta controls

Status: complete locally; automated checks passed, native evidence pending. Base: `97e254c`, version `0.2.9`.

## Scope

CB-J01/03/04: keep the beta's visible controls, descriptions and permission
language consistent with implemented behavior. No search engine, AI provider,
new AWS operation or streaming mechanism is added.

This unit removes the placeholder Global Search and its keyboard shortcut, the
fixed-sample AI generator, and the unwired Lambda Logs Pause control and live
indicator. Working per-widget filters and explicit refresh actions remain.
Unsupported saved widgets explain their unavailable type and can be removed;
they do not promise pending runtime wiring or offer a nonfunctional refresh.
Clipboard success is reported only when the browser confirms the write.

The visible label **Log Error Counts** replaces **Errors by Stack**. The existing
`errors-by-stack` identifier remains for saved layouts and backend routing.
Its bounded queries aggregate sampled log groups; names derived from log groups
do not prove stack ownership, and the displayed counts are not a complete
account-wide error total. Its current query matches case-sensitive `ERROR` text;
discovery uses its first page and at most 20 matching log groups. These existing
limits remain visible in coverage rather than being represented as a full scan.

## Operation language

The interface distinguishes resource reads, query execution/control and
credential acquisition/verification. Local policy narrows the compiled registry;
it cannot grant operations outside it or replace AWS IAM. Logs Insights queries
can incur charges, and stopping the local wait is distinct from confirmed remote
query cleanup. SSO credentials and their renewal are verified before use; this
is not an ordinary resource read. The documented provider-internal OIDC limitation
from P1 remains unchanged.

The CLI retains its exact 18 reviewed resource-read mappings, including
`sts get-caller-identity`. It does not provide an arbitrary shell or a path to
query/credential operations. Optional CLI availability is checked separately
from the SDK widgets' verified connection.

## Retained control inventory

All live AWS views require successfully loaded settings, a supported SSO profile,
a verified widget context and permitted operations. Saved pins retain their own
context. Common result states distinguish empty success, bounded/partial results,
denial, expiry and failed requests; unknown coverage does not prove absence.

| Widget / stable ID | Real controls | Specific prerequisite or limit | Empty / failure behavior |
| --- | --- | --- | --- |
| Lambda Logs / `log-tail` | Filter and select functions; load streams; view events; reload; copy the log group | Configured log groups and unverified default conventions have different labels; no automatic live feed | Empty functions, streams or events remain explicit; missing identifiers and denied reads do not become invented log targets |
| CloudWatch Logs / `cloudwatch-logs` | Filter loaded groups; Enter or Reload searches AWS; select group, stream and events; copy the group | Discovery and event reads retain their existing page limits | No loaded match can offer an explicit AWS search; failed/limited discovery does not establish absence |
| Pipeline Runs / `pipeline-runs` | Search/select pipeline; Load runs; Live/Pinned tabs; pin/remove/reorder; expand executions and reviewed evidence links | Pipeline selection is required; each saved pin is independently verified | Empty pipeline/run/pin lists differ from discovery failure; partial or denied details retain available evidence |
| CodeArtifact Packages / `codeartifact-packages` | Edit domain, repository, prefix and maximum; Refresh loads packages; inspect versions; copy values | Domain, repository and package prefix are required; the first load is explicit | Missing inputs stay visible; empty packages differ from partial version/enrichment failure |
| CloudFormation Stacks / `cfn-stacks` | Refresh; filter returned stacks; expand resources and recent events | Listing, enrichment and event limits remain visible | Empty/filter-empty tables differ from denied/failed requests; available resources survive an event failure |
| Resource Reverse Lookup / `resource-lookup` | Enter a resource query; inspect tagged matches; open reviewed stack evidence or choose a stack explicitly | Only reviewed exact physical-ID/type mappings establish ownership; tagged discovery and five-match enrichment are bounded | Blank query requests input; no matches explains tagging coverage; unknown/unsupported/ambiguous/denied owners keep the resource match |
| Log Error Counts / `errors-by-stack` | Refresh bounded queries for the default last 24 hours; filter returned names/counts | Sampled log groups and query aggregation limits; queries can incur charges | Empty, partial and failed queries differ; coverage reports the sample and lower bounds, with cleanup uncertainty preserved |
| Logs Insights Query / `logs-insights` | Enter log group/query/time range; Run query; Refresh; inspect rows/statistics | Log group and query required; query start/stop must be locally permitted; AWS charges can apply | Zero rows differ from failure/timeout; cancellation and remote cleanup are reported separately |
| AWS CLI Table / `aws-cli` | Enter/Run command; Live/Pinned tabs; pin/remove/reorder; Retry CLI check | Local compatible CLI required; exact supported mapping, valid arguments and verified unexpired credentials | Missing CLI disables only CLI execution; blank/unsupported commands are explained; zero rows and failed/capped output differ |

Shared controls remain: Add/remove widgets; fullscreen; widget configuration and
context pinning; layout reset and truthful save/retry status; theme preview and
explicit Settings save; policy edit/reload/validate/save; Audit diagnostics; and
Identity refresh/reconnect. Source inspection shows the compiled widget source
in the desktop app. These controls do not promise application features outside
the retained catalogue.

## Validation

The two focused browser suites pass all 23 cases in one installed-Chrome worker (16.9 seconds, exit 0). New cases exercise removed controls/shortcut, retained catalogue and filtering behavior, the display-name change with stable widget ID, explicit R/Q/C explanations, and synthetic clipboard failure followed by a successful retry. No real clipboard is used by the new failure scenario.

All 17 Node cases pass. Frontend syntax, whitespace, tracked/new-file privacy and current-tree Gitleaks checks pass. Rust is unchanged from P2-05, whose 248 offline library cases and Clippy passed. The full browser regression is repeated with P2-07 final integration.

No AWS, credential inspection, actual CLI execution, native app launch, remote Git or publication action was performed.

## Limits / next

P2-07 covers critical keyboard use, panel focus and readable desktop layouts.
Native-device, clipboard-permission and live-provider acceptance remain separate
checks; this unit does not establish platform-wide accessibility or behavior.
