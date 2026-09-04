# AWS operation and Tauri command inventory

Status: historical source review on 2026-09-03 for v0.2.9, commit `095d1ad`. The line links and findings below describe that baseline. No AWS calls were made for this review.

Subsequent changes: **P1-01 through P1-03 are complete locally; P1-04 is next and full P1 remains incomplete.** [P1-01](p1-01-evidence.md) isolates test storage/runtime boundaries. [P1-02](p1-02-evidence.md) moves the same 21 operations to classified records in [guard.rs](../../src-tauri/src/aws/guard.rs), with exact CLI mappings for the 18 resource reads and validated argument schemas. [P1-03](p1-03-evidence.md) adds explicit SSO snapshots and STS-verified contexts. The historical findings below are retained for traceability, not presented as current behavior.

## Current boundary after P1-03

| Surface | Locally implemented behavior | Remaining proof / work |
| --- | --- | --- |
| Profile/configuration selection | Parses the selected legacy or named-session SSO profile into an owned snapshot; rejects ambiguous aliases and unsupported credentials, role chains, external processes or endpoint indirection; no process-wide AWS config mutation | Real user configuration and live provider compatibility remain unverified |
| Credential acquisition (C) | Explicit SSO `GetRoleCredentials`; named-session SDK token renewal and local cache activity remain separate provider effects. Legacy cached tokens must match the selected start URL/region and be unexpired | No live token renewal or SSO request was exercised; expired legacy tokens require external login |
| STS and resource calls (R) | Share one frozen credential set; compare STS account and retain principal; check expiry, reverify refresh and invalidate changed identity/configuration | Offline fixtures establish application behavior, not live AWS or native runtime behavior |
| Active/pinned contexts (L/R/C) | Latest connection attempt owns publication; pinned contexts verify independently; cache identity includes selected configuration and verified principal/session; stale session results are fenced | Broader tile/detail/selector ownership remains P1-05 |
| Frontend auth status | Selection/request guards reject older poll results and prevent stale cached identity-panel updates | Node handler tests are not native GUI or full browser acceptance |
| Desktop CLI | Exact P1-02 parsing/policy rules remain, but `widget_fetch` returns `CliContextUnavailable` without spawning a child | P1-04 must implement verified credential handoff, controlled environment, streaming limits and process supervision |

P1-03 validation: **97 Rust library tests and 5 Node production-handler tests passed** with synthetic boundaries. No live AWS, native GUI, Windows/Ubuntu validation or GitHub action was performed. See [P1-03 evidence](p1-03-evidence.md) for the complete proof boundary.

Remaining provider limits: the named-session token provider rereads its cache after metadata preflight, without an atomic lock against external replacement. Provider-internal OIDC requests are not individually covered by application registry audit events. Preflight records still do not establish completed wire calls.

The baseline registers **14 Tauri commands**, dispatches **13 widget names**, and declares **21 `APP_OPS` entries**. Twenty are explicit first-party SDK operations; `sso:GetRoleCredentials` represents delegated credential acquisition. At that baseline, the generic CLI and SDK credential provider were outside the proof offered by the finite registry.

## Effect classes

| Class | Meaning |
| --- | --- |
| R | Reads AWS resource data, identity metadata, query results, or local files. A read may still expose sensitive data or incur costs. |
| Q | Starts or stops a Logs Insights query. Changes query execution state and may incur scanning costs; this is not infrastructure/resource mutation. |
| C | Obtains or refreshes temporary credentials or authentication sessions. |
| L | Changes local application state, environment, settings, policy, dashboard, audit, or potentially CLI output files. |
| U | User-selected CLI operation and arguments; the compiled registry does not exhaustively describe this surface. |

## Historical Tauri command surface (P0)

All 14 commands are registered in [lib.rs:33](../../src-tauri/src/lib.rs#L33).

| Command | Classes and effects | Source |
| --- | --- | --- |
| `ping` | R: version and pong; no explicit I/O | [commands.rs:88](../../src-tauri/src/commands.rs#L88) |
| `aws_set_account` | C/L: reads settings; changes `AWS_CONFIG_FILE`; loads policy; resolves credentials; updates active context and audit | [commands.rs:93](../../src-tauri/src/commands.rs#L93) |
| `aws_list_profiles` | R: reads configured AWS config and returns profile metadata and allowed regions | [commands.rs:458](../../src-tauri/src/commands.rs#L458) |
| `aws_list_pipelines` | R, conditional C/L: `ListPipelines`, up to 200 results; provider, policy, and audit effects | [commands.rs:466](../../src-tauri/src/commands.rs#L466) |
| `aws_auth_status` | R, conditional C/L: `GetCallerIdentity`; reads cached token expiry; provider, policy, and audit effects | [commands.rs:540](../../src-tauri/src/commands.rs#L540) |
| `widget_fetch` | R/Q/C/L/U by widget: resolves inherited or pinned context, initializes SDK, loads policy, dispatches | [commands.rs:383](../../src-tauri/src/commands.rs#L383) |
| `widget_get_source` | R: returns compiled-in widget descriptions and source strings | [commands.rs:422](../../src-tauri/src/commands.rs#L422) |
| `settings_get` | R: reads settings; silently falls back to defaults on failures | [commands.rs:431](../../src-tauri/src/commands.rs#L431) |
| `settings_set` | L: writes `settings.json`; filesystem errors are currently discarded | [settings.rs:44](../../src-tauri/src/settings.rs#L44) |
| `dashboard_get` | R: reads saved dashboard; silently falls back on failures | [commands.rs:441](../../src-tauri/src/commands.rs#L441) |
| `dashboard_set` | L: writes temporary dashboard then renames; failures are currently discarded | [dashboard.rs:35](../../src-tauri/src/dashboard.rs#L35) |
| `audit_tail` | R: caller chooses limit; implementation reads the entire audit file before selecting the tail | [audit.rs:64](../../src-tauri/src/audit.rs#L64) |
| `policy_get` | R, conditional L: can create missing policy or rewrite a recognized legacy default | [policy.rs:378](../../src-tauri/src/aws/policy.rs#L378) |
| `policy_set` | L: validates YAML then writes policy; validation and I/O errors are returned | [policy.rs:413](../../src-tauri/src/aws/policy.rs#L413) |

Cross-cutting effects: every `policy::load()` may create or upgrade `policy.yaml`; preflight and lifecycle calls append audit entries. App files use the per-user `.cloud_burrito` directory. See [policy.rs:408](../../src-tauri/src/aws/policy.rs#L408), [paths.rs:5](../../src-tauri/src/paths.rs#L5), and [audit.rs:38](../../src-tauri/src/audit.rs#L38).

## Complete finite AWS registry

Baseline source: [policy.rs:163](../../src-tauri/src/aws/policy.rs#L163); current classified source: [guard.rs](../../src-tauri/src/aws/guard.rs). The same 21 operations remain. These are the app's policy namespaces; this table is not a ready-to-attach IAM policy and requires service-prefix mapping before IAM use.

| Source namespace | Registered operations | Class | Count |
| --- | --- | --- | --- |
| `sso` | `GetRoleCredentials` | C; delegated/provider-mediated | 1 |
| `sts` | `GetCallerIdentity` | R; identity metadata | 1 |
| `cloudformation` | `ListStacks`, `DescribeStackResources`, `DescribeStackEvents` | R | 3 |
| `logs` | `DescribeLogGroups`, `GetQueryResults`, `FilterLogEvents`, `GetLogEvents`, `DescribeLogStreams` | R | 5 |
| `logs` | `StartQuery`, `StopQuery` | Q | 2 |
| `codepipeline` | `ListPipelines`, `ListPipelineExecutions`, `ListActionExecutions` | R | 3 |
| `codebuild` | `BatchGetBuilds` | R; this particular Batch operation is a read | 1 |
| `codeartifact` | `ListPackages`, `ListPackageVersions`, `DescribePackageVersion` | R | 3 |
| `lambda` | `ListFunctions` | R | 1 |
| `resourcegroupstaggingapi` | `GetResources` | R | 1 |
| **Total** | **18 reads, 2 query-control operations, 1 credential operation** | | **21** |

Legacy policy migration arrays are not additional active compiled operations.

## Historical widget and drill-down dispatch surface (P0)

All 13 names are dispatched in [widgets/mod.rs:196](../../src-tauri/src/widgets/mod.rs#L196). SDK paths also inherit credential, policy, and audit effects described above.

| Dispatch name | Operations or execution path | Class | Source |
| --- | --- | --- | --- |
| `aws-cli` | User-selected service/operation -> `preflight_cli` -> local AWS executable | U; potentially R/Q/C/L or an unsafe resource mutation if the classifier admits it | [aws_cli.rs:26](../../src-tauri/src/widgets/aws_cli.rs#L26) |
| `cfn-stacks` | `ListStacks`; `DescribeStackResources` enrichment | R | [cfn_stacks.rs:34](../../src-tauri/src/widgets/cfn_stacks.rs#L34), [89](../../src-tauri/src/widgets/cfn_stacks.rs#L89) |
| `cfn-stack-detail` | `DescribeStackResources`, `DescribeStackEvents` | R | [cfn_stack_detail.rs:18](../../src-tauri/src/widgets/cfn_stack_detail.rs#L18) |
| `cloudwatch-logs` | Groups: `DescribeLogGroups`; streams/events delegate to `log_tail`: `DescribeLogStreams`, `GetLogEvents` | R | [cloudwatch_logs.rs:12](../../src-tauri/src/widgets/cloudwatch_logs.rs#L12) |
| `logs-insights` | Groups: `DescribeLogGroups`; query: `StartQuery`, `GetQueryResults`; poll-budget expiry: best-effort `StopQuery` | R/Q | [logs_insights.rs:19](../../src-tauri/src/widgets/logs_insights.rs#L19), [124](../../src-tauri/src/widgets/logs_insights.rs#L124) |
| `log-tail` | List: `ListFunctions`; streams: `DescribeLogStreams`; events: `GetLogEvents`; tail: `FilterLogEvents` | R | [log_tail.rs:34](../../src-tauri/src/widgets/log_tail.rs#L34), [143](../../src-tauri/src/widgets/log_tail.rs#L143), [206](../../src-tauri/src/widgets/log_tail.rs#L206), [265](../../src-tauri/src/widgets/log_tail.rs#L265) |
| `errors-by-stack` | `DescribeLogGroups`; up to 20 concurrent `StartQuery` / `GetQueryResults` loops; no `StopQuery` cleanup | R/Q | [errors_by_stack.rs:22](../../src-tauri/src/widgets/errors_by_stack.rs#L22), [55](../../src-tauri/src/widgets/errors_by_stack.rs#L55), [142](../../src-tauri/src/widgets/errors_by_stack.rs#L142) |
| `resource-lookup` | `GetResources`; `DescribeStackResources` enrichment | R | [resource_lookup.rs:97](../../src-tauri/src/widgets/resource_lookup.rs#L97), [150](../../src-tauri/src/widgets/resource_lookup.rs#L150) |
| `pipeline-runs` | `ListPipelineExecutions` | R | [pipeline_runs.rs:15](../../src-tauri/src/widgets/pipeline_runs.rs#L15) |
| `codeartifact-packages` | `ListPackages`; per package `ListPackageVersions`; latest-version `DescribePackageVersion` | R | [codeartifact_packages.rs:267](../../src-tauri/src/widgets/codeartifact_packages.rs#L267), [311](../../src-tauri/src/widgets/codeartifact_packages.rs#L311), [353](../../src-tauri/src/widgets/codeartifact_packages.rs#L353) |
| `codeartifact-package-version-history` | `DescribePackageVersion` for supplied versions lacking dates; concurrency bounded to 4 | R | [codeartifact_packages.rs:121](../../src-tauri/src/widgets/codeartifact_packages.rs#L121) |
| `pipeline-execution-detail` | `ListActionExecutions` | R | [pipeline_execution_detail.rs:16](../../src-tauri/src/widgets/pipeline_execution_detail.rs#L16) |
| `codebuild-log` | `BatchGetBuilds`, `GetLogEvents` | R | [codebuild_log.rs:28](../../src-tauri/src/widgets/codebuild_log.rs#L28) |

## Historical provider and CLI findings (P0)

Findings 1–5 describe the original baseline. P1-02 closed the registry/argument bypasses, and P1-03 replaced the default-provider/account-label path and disabled desktop CLI execution pending P1-04. Preflight audit events still do not prove individual completed requests; live-provider and process-runtime validation remain open.

1. **Account labels are not verified identities.** `AwsContext` accepts a supplied account ID. `aws_set_account` publishes a context after credential resolution, without STS verification. The later STS check records ARN and login status but does not compare the returned account with the selected account. Pinned contexts likewise accept supplied account/profile/region. See [context.rs:68](../../src-tauri/src/aws/context.rs#L68), [commands.rs:203](../../src-tauri/src/commands.rs#L203), [255](../../src-tauri/src/commands.rs#L255), and [626](../../src-tauri/src/commands.rs#L626).
2. **Credential preflight is a proxy, not network interception.** It requires `sso:GetRoleCredentials` before non-credential operations regardless of the actual provider. The standard `aws-config` loader delegates provider behavior; the lockfile includes SSO, SSO OIDC, and STS dependencies. Role assumption, token refresh, local cache activity, and provider subprocess behavior are configuration-dependent and not exhaustively verified here. Do not label every resolution as an observed SSO call. See [policy.rs:302](../../src-tauri/src/aws/policy.rs#L302), [context.rs:47](../../src-tauri/src/aws/context.rs#L47), and [Cargo.lock:129](../../src-tauri/Cargo.lock#L129).
3. **CLI intentionally bypasses the finite registry.** It retains an operation-name classifier and user policy. Default policy denies additional operations, but a broad user Allow can admit a resource mutation such as `ecr:BatchDeleteImage`: `Batch` is permitted and that operation is absent from the exception list. This is a conditional static finding, not an executed exploit. See [policy.rs:444](../../src-tauri/src/aws/policy.rs#L444) and [guard.rs:8](../../src-tauri/src/aws/guard.rs#L8).
4. **Credential issuance is accepted by the classifier.** `AssumeRole`, `AssumeRoleWithSAML`, `AssumeRoleWithWebIdentity`, `GetSessionToken`, and `GetFederationToken` are explicitly accepted by name. They are not registered compiled calls, but broad CLI policy can admit them. Classify them as C, not ordinary resource reads. See [guard.rs:29](../../src-tauri/src/aws/guard.rs#L29).
5. **CLI arguments and environment do not enforce the displayed context.** Only `--profile` and `--output` are rejected. Region, endpoint, TLS, signing, debug, local-file arguments, and other CLI options pass through. The child inherits the environment except for five overrides. No shell is invoked, an existing protection. The 2 MB stdout limit is checked only after stdout and stderr have been buffered. See [aws_cli.rs:61](../../src-tauri/src/widgets/aws_cli.rs#L61) and [148](../../src-tauri/src/widgets/aws_cli.rs#L148).
6. **Audit entries represent preflight decisions, not necessarily completed calls.** Some loops preflight once and issue multiple requests. Polling and credential refreshes are not individually represented. Avoid claiming one audit event per AWS API call. See [widgets/mod.rs:71](../../src-tauri/src/widgets/mod.rs#L71) and [errors_by_stack.rs:55](../../src-tauri/src/widgets/errors_by_stack.rs#L55).

## Phase 1 acceptance priorities

| Order | Required acceptance / current status |
| --- | --- |
| 1. Execution boundary | Complete locally in P1-02: wildcard policy cannot add unknown operations; exact CLI mappings/argument schemas reject misleading-name mutations and unsupported overrides. R/Q/C are classified separately. |
| 2. Verified identity | Complete locally for SDK contexts in P1-03: STS account/principal verification, shared frozen credentials, expiry/refresh checks, independent pinned identities and latest-attempt connection publication. CLI credential handoff remains P1-04; broader frontend ownership remains P1-05. |
| 3. CLI authority — next | P1-04: retain P1-02 schemas and replace the temporary `CliContextUnavailable` response only after verified credential handoff, controlled environment, resolved executable, bounded streams and supervised termination are implemented. |
| 4. Provider behavior | P1-03 implements explicit SSO-only configuration and rejects unsupported mechanisms before provider work. Named-session renewal and legacy cached-token handling are source-reviewed; live validation is pending. Preflight versus actual provider-event auditing remains P1-06. |
| 5. Query lifecycle | Keep `StartQuery` and `StopQuery` in Q. Track query IDs; cap windows and total concurrency; attempt cleanup on cancellation, timeout, and polling failure. Report cleanup failures and partial results. Cover the missing cleanup in `errors-by-stack`. |
| 6. Local state and evidence | P1-03 removes global config mutation and adds context invalidation. Remaining work includes bounded/redacted audit and CLI errors, accurate audit outcomes and visible persistence failures. Preserve the distinction between policy reads and creation/migration. |

Next implementation slice: **P1-04 — verified credential handoff and a constrained CLI child**. P1-05/06, query lifecycle work and deferred live/native validation remain open.
