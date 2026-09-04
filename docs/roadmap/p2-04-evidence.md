# P2-04 — Result state, freshness and coverage

Status: complete locally; automated checks passed, native evidence pending. Base: `5aad323`, version `0.2.9`.

## Scope

CB-J02/03/05: preserve the origin and age of evidence, distinguish an empty
successful response from an unsuccessful request, and show known result limits
and failed subrequests. Existing request counts, concurrency and query-cleanup
behavior remain the P3 baseline.

## Response contract

Widget coverage is additive: `completeness` is `complete`, `limited` or `unknown`;
`has_more` can be unknown. Named counts, limits, fixed reasons and optional
section coverage describe what was actually observed. Coverage describes the
requested scope, not every resource or historical event in an account.
Intentional truncation does not make a successful bounded read an execution
failure. Failed pages or enrichment remain explicit failures with any evidence
already obtained. Logical cancellation does not establish that a remote query
or local process tree stopped.

## Validation

- 235 locked, offline Rust library tests pass. Actual producers consume queued
  synthetic JSON/XML/REST responses; unexpected requests and any endpoint outside
  the fixed invalid test origin panic. Default provider/HTTP fences are unchanged.
  No real connector is constructed by the fixture.
- The new producer cases cover later-page failures, exact and exceeded limits,
  enrichment denial/failure, retained CFN resources when events fail, query
  discovery/aggregation limits, CLI projection/cell clipping, and pipeline-name
  discovery's existing 200-result boundary. Request-count assertions prevent
  retention from introducing extra calls.
- All-target Clippy with warnings denied passes; 17 Node ownership/auth cases
  and the exact 15-command registry check pass.
- All 75 production-frontend browser cases pass in one installed-Chrome worker (1.2 minutes, exit 0), including the 13 new result-state cases. Existing ownership, persistence, settings and hostile-rendering assertions remain intact.
- Tracked/new-file privacy and current-tree Gitleaks scans pass.

## Frontend behavior

All 18 widget-fetch paths plus pipeline-name discovery share result-state
settlement. Custom arrays, selectors and CodeArtifact history preserve the same
coverage contract as tables, logs and detail views. Invalid response shapes or
coverage metadata cannot become usable evidence. Existing context/request
ownership checks still run before rendering.

Same-context, same-input refreshes retain the prior view with its original
receipt time and coverage. A subsequent denial, cancellation or failure reports
the refresh outcome separately from the stale evidence. Changing context,
configuration, inputs or owner lifetime discards the old view. The existing
CodeArtifact history cache retains its original receipt time when reopened.

Partial producer results keep available rows or sections. Empty failed/limited
responses do not claim the account has no resources or errors. Query-count
lower bounds, unknown stream coverage, unvisited pages and clipped CLI cells
remain visible. Query polling failures now state that cleanup was not attempted
and remote execution may continue; this adds no stop call or new cleanup logic.

Only synthetic identities, disposable storage and localhost browser bridges were used. No AWS, credential inspection, actual CLI execution, native app launch, remote Git or publication action was performed.

## Limits / next

The existing CodeBuild event threshold is checked between whole pages; the
returned event count can exceed that threshold. P2 reports the observed count
and page-stop threshold honestly. Strict work budgets, query cleanup and
performance measurements remain P3. Real-provider and native-device acceptance
remain pending. P2-05 next connects the flagship investigation using explicit
resource identifiers and honest unknown associations.
