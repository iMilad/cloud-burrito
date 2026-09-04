# P2-05 — Evidence-bound investigation

Status: complete locally; automated checks passed, native evidence pending. Base: `c529384`, version `0.2.9`.

## Scope

CB-J03 with CB-J02 context ownership: connect pipeline actions, build logs,
CloudFormation resources/events and CloudWatch evidence without inventing a
relationship from similar resource names.

Navigation metadata carries a closed status, provenance, target widget, reviewed
inputs and fixed explanatory text. It is an explicit user action, never authority
to switch accounts, regions or policies. Pipeline action targets must belong to
the requested execution and the verified context. An AWS CodeBuild Build action
can identify a build; a CloudFormation action's stack configuration identifies a
configured target, without proving its existence or ownership.

A matching build's actual log group/stream and a CloudFormation log-group
resource's physical identifier can lead to logs. Lambda's configured log group
is distinct from its unverified default naming convention. No build-to-stack
relationship is inferred. Unknown relationships require same-context lookup or
an explicitly labelled manual selection.

Resource lookup confirms ownership only for unqualified Lambda functions,
CloudWatch log groups and EC2 instances with an exact physical-resource
identifier/type and one consistent stack identifier in the same account/region.
An arbitrary ARN suffix or the first returned stack row
cannot establish ownership. Unsupported, ambiguous, denied and unattempted
associations remain unknown. Existing page/enrichment budgets remain unchanged.

## Validation

- 248 locked, offline Rust library tests pass, including 13 new producer cases
  for handoff provenance, exact ownership, wrong or foreign identifiers,
  unsupported actions, partial/denied results and omitted configuration fields.
  All-target Clippy passes with warnings denied.
- 17 Node ownership/auth cases and the exact 15-command registry check pass.
- All 84 production-frontend browser cases pass in one installed-Chrome worker (1.3 minutes, exit 0), including nine new investigation scenarios. They cover the failed execution/build/stack/log journey, manual and confirmed targets, denied/partial data, stale controls, close/reopen and removed sources, and independent pinned contexts.
- Tracked/new-file privacy and current-tree Gitleaks scans pass.

Only synthetic identities, disposable storage and localhost browser bridges were used. No AWS, credential inspection, actual CLI execution, native app launch, remote Git or publication action was performed.

## Limits / next

These checks do not establish native-device or live-provider acceptance.
Ownership proof deliberately supports a small reviewed set of resource types;
unknown does not mean no stack exists. P2-06 next aligns visible beta controls and
help with implemented behavior.
