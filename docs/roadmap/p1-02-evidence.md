# P1-02 — Exact application capabilities

Status: **implemented and locally validated**, 2026-09-04. Local working-tree
change on top of P1-01, version `0.2.9`, original commit `095d1ad`. Nothing has
been committed, pushed, released or published. No live AWS calls were performed.

## Result

The application admits exact reviewed operations. A broad local policy cannot
turn an unsupported command into an executable capability. The existing 21
operations now have explicit records: 18 resource reads, query start, query stop,
and credential acquisition. SDK action names and CLI command spellings are
mapped explicitly; operation-name prefixes and PascalCase conversion no longer
authorize anything.

The CLI table supports only those 18 resource reads, with per-command argument
schemas. It rejects unsupported operations, abbreviations, unknown or repeated
options, identity/region/endpoint/TLS/output overrides, file/URL-loading values,
and unreviewed structured inputs. Allowed commands retain the existing table
rendering and injected process boundary. Resource reads still require a policy
allow; explicit deny takes precedence, and invalid policy fails closed.

Both Logs Insights workflows require local permission to stop a query before
starting one. Errors by Stack checks its complete query capability before log
group discovery. This is an authorization prerequisite, not a remote cleanup
implementation or proof of IAM permission.

The UI reports a blocked request and its reason without suggesting that every
denial can be fixed by widening policy. The command preview displays the typed
CLI service/subcommand; only the backend establishes the approved operation.

## Regression and checks

Before implementation, the production CLI fetch path received a wildcard
policy and this synthetic request:

```text
aws ecr batch-delete-image --repository-name demo-repository --image-ids imageTag=demo
```

The injected fake runner recorded **one attempted process call**, failing the
expected zero-spawn assertion. No real process or AWS request was used. The same
test is retained to establish denial after the change.

| Check | Observed result |
| --- | --- |
| Full Rust library suite, locked/offline, normal parallel run | **73 passed; 0 failed; 0 ignored; 0 filtered** |
| Original wildcard deletion regression | Pass; zero fake process calls after the fix |
| All 18 reviewed CLI reads through the real widget fetch path | Each produces a table using exactly one scripted process response |
| Wildcard operation/argument denial matrix | All rejected before process execution, without a permission-widening hint |
| Invalid policy, default/explicit deny, argument types/bounds and relationships | Pass |
| Both query widgets without StopQuery permission (absent or explicit deny) | Denied with zero SDK credential and transport work |
| Default policy text/order and legacy migration compatibility | Pass |
| All-target Clippy with warnings denied | Pass |
| Rust formatting, JavaScript syntax, whitespace checks | Pass |
| Release version and Tauri command registry | Version 0.2.9; the same 14 commands |
| Privacy scan of Rust source, frontend, README and roadmap | Pass; 51 files |
| Focused read-only implementation review | No remaining concrete P1-02 findings |

The unit tests use isolated temporary stores, scripted process output, and SDK
credential/transport fences that panic on unexpected boundary work. Native AWS
and process adapters also reject use in unit tests. No real AWS CLI executable,
provider discovery, credentials, or service transport was used.

Core reproduction commands, from `src-tauri`:

```sh
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  -u AWS_PROFILE -u AWS_DEFAULT_PROFILE AWS_EC2_METADATA_DISABLED=true \
  cargo test -p cloud-burrito --locked --offline --lib
cargo clippy -p cloud-burrito --locked --offline --all-targets -- -D warnings
cargo fmt --all --check
```

Frontend changes received syntax/static review; a browser or native GUI
acceptance run was not performed in this unit.

## CLI contract sources

Schemas were reviewed against the official AWS CLI references for the commands
listed in `guard::APP_OPS`; the accepted subset is implemented in
[`aws_cli.rs`](../../src-tauri/src/widgets/aws_cli.rs). These are deliberately
smaller than the full AWS CLI schemas. The review covers required targets,
pagination differences, value bounds and mutually exclusive options.

Examples of important distinctions:

- CloudFormation's stack list/event commands support starting-token/max-items,
  while describe-stack-resources uses a stack or physical-resource target.
  [List stacks](https://docs.aws.amazon.com/cli/latest/reference/cloudformation/list-stacks.html),
  [Describe stack resources](https://docs.aws.amazon.com/cli/latest/reference/cloudformation/describe-stack-resources.html).
- Logs get-log-events uses next-token/limit; get-query-results has its own
  service pagination inputs. The app does not infer universal paginator flags.
  [Get log events](https://docs.aws.amazon.com/cli/latest/reference/logs/get-log-events.html),
  [Get query results](https://docs.aws.amazon.com/cli/latest/reference/logs/get-query-results.html).
- Logs unmask requires an additional permission and is excluded. Structured
  pipeline filters and tagging filters are also outside this CLI subset.
  [Filter log events](https://docs.aws.amazon.com/cli/latest/reference/logs/filter-log-events.html),
  [List pipeline executions](https://docs.aws.amazon.com/cli/latest/reference/codepipeline/list-pipeline-executions.html),
  [Get resources](https://docs.aws.amazon.com/cli/latest/reference/resourcegroupstaggingapi/get-resources.html).
- File parameter loading and CLI v1 URL parameter loading require explicit
  rejection when the installed executable version is not pinned.
  [File parameters](https://docs.aws.amazon.com/cli/latest/userguide/cli-usage-parameters-file.html),
  [CLI v2 migration](https://docs.aws.amazon.com/cli/latest/userguide/cliv2-migration-changes.html#cliv2-migration-paramfile).

## Compatibility and remaining work

The CLI surface is intentionally narrower. Existing EC2, S3 or other unsupported
pins remain saved but are rejected when run, even under wildcard policy. Default
policy action order and recognized legacy-policy migrations are preserved.

P1-03 still owns verified active/pinned identity, authentication ordering and
configuration isolation. P1-04 owns the child environment and streaming output,
timeout and cancellation limits. Provider internals are not intercepted by this
registry. Audit entries still represent preflights rather than per-wire-call
outcomes. Full query cancellation/cleanup remains P3 work. Native/provider tests
on macOS, Windows and Ubuntu are still pending.

Next: **P1-03 — verify active and pinned identities and isolate configuration.**
