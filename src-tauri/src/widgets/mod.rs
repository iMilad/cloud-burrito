//! Compiled-in widgets. Each is a free async `fetch(&WidgetCtx) -> Value` that
//! returns one of the closed render shapes the frontend's dispatcher knows.
//!
//! Widgets are part of the binary — there is no dynamic loading and no
//! arbitrary-code execution surface (originally ported from the Python sidecar's
//! runtime `importlib` widget loader). The one deliberate exception is the
//! AWS CLI Table widget (`aws_cli.rs`), which spawns the local `aws` binary —
//! never a shell — after exact operation, argument and policy checks.

mod aws_cli;
pub(crate) use aws_cli::parse_cli_command;
mod cfn_stack_detail;
mod cfn_stacks;
mod cloudwatch_logs;
mod codeartifact_packages;
mod codebuild_log;
pub(crate) mod coverage;
mod errors_by_stack;
mod handoff;
mod log_tail;
mod logs_insights;
mod pipeline_execution_detail;
mod pipeline_runs;
mod resource_lookup;

use aws_config::SdkConfig;
use aws_smithy_types::date_time::Format;
use aws_smithy_types::DateTime;
use serde_json::{json, Map, Value};

use crate::runtime::Runtime;

/// Explicit per-request authority supplied only after connection verification.
/// Do not derive Debug: AWS Credentials Debug includes the access-key ID.
pub(crate) struct CliAccess {
    pub credentials: aws_credential_types::Credentials,
    pub cancellation: crate::process::ProcessCancellation,
}

/// The per-fetch execution surface a widget is allowed to use.
pub struct WidgetCtx {
    pub runtime: Runtime,
    pub sdk: SdkConfig,
    pub account_id: String,
    pub region: String,
    pub widget_name: String,
    pub inputs: Value,
    /// Active read-only policy (Err = invalid file -> fail closed in preflight).
    pub policy: Result<crate::aws::policy::Policy, String>,
    pub(crate) cli: Option<CliAccess>,
}

impl WidgetCtx {
    pub fn input_str(&self, key: &str, default: &str) -> String {
        self.inputs
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| default.to_string())
    }

    pub fn input_i64(&self, key: &str, default: i64) -> i64 {
        match self.inputs.get(key) {
            Some(Value::Number(n)) => n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .unwrap_or(default),
            Some(Value::String(s)) => s.trim().parse::<i64>().unwrap_or(default),
            _ => default,
        }
    }

    pub fn input_value(&self, key: &str) -> Option<&Value> {
        self.inputs.get(key)
    }

    /// Preflight one AWS API call: enforce the read-only policy and audit the
    /// outcome. Returns `None` when allowed (call may proceed) or
    /// `Some(render)` — a `permission_denied` tile — when blocked.
    pub fn preflight(&self, service: &str, operation: &str) -> Option<Value> {
        if let Some(denied) = self.preflight_credentials(service, operation) {
            return Some(denied);
        }
        match crate::aws::policy::gate(&self.policy, service, operation) {
            Ok(()) => {
                self.audit_call(service, operation, "aws", None);
                None
            }
            Err(reason) => {
                self.audit_call(service, operation, "aws-blocked", Some(&reason));
                Some(permission_denied_render(service, operation, &reason))
            }
        }
    }

    /// Preflight one user-supplied CLI call: same credential check and
    /// auditing as `preflight`, plus an explicit CLI mapping in the compiled
    /// registry. The CLI parser validates the operation's arguments first.
    pub fn preflight_cli(&self, service: &str, operation: &str) -> Option<Value> {
        if let Some(denied) = self.preflight_credentials(service, operation) {
            return Some(denied);
        }
        match crate::aws::policy::gate_cli(&self.policy, service, operation) {
            Ok(()) => {
                self.audit_call(service, operation, "aws", Some("aws-cli widget"));
                None
            }
            Err(reason) => {
                self.audit_call(service, operation, "aws-blocked", Some(&reason));
                Some(permission_denied_render(service, operation, &reason))
            }
        }
    }

    fn preflight_credentials(&self, service: &str, operation: &str) -> Option<Value> {
        match crate::aws::policy::credential_preflight(&self.policy, service, operation) {
            crate::aws::policy::CredentialPreflight::NotRequired => None,
            crate::aws::policy::CredentialPreflight::Allowed {
                service,
                operation,
                reason,
            } => {
                self.audit_call(service, operation, "aws", Some(&reason));
                None
            }
            crate::aws::policy::CredentialPreflight::Denied {
                service,
                operation,
                reason,
            } => {
                self.audit_call(service, operation, "aws-blocked", Some(&reason));
                Some(permission_denied_render(service, operation, &reason))
            }
        }
    }

    fn audit_call(&self, service: &str, operation: &str, kind: &str, reason: Option<&str>) {
        let mut entry = json!({
            "kind": kind,
            "service": service,
            "operation": operation,
            "account_id": self.account_id,
            "region": self.region,
        });
        if let Some(r) = reason {
            entry["reason"] = json!(r);
        }
        self.runtime.audit(entry);
    }

    /// Structured widget log entry (kind="widget").
    pub fn log(&self, message: &str, extra: Value) {
        let mut obj = Map::new();
        obj.insert("kind".into(), json!("widget"));
        obj.insert("widget".into(), json!(self.widget_name));
        obj.insert("message".into(), json!(message));
        if let Some(m) = extra.as_object() {
            for (k, v) in m {
                obj.insert(k.clone(), v.clone());
            }
        }
        self.runtime.audit(Value::Object(obj));
    }
}

/// The closed render shape shown when a call is blocked by policy or the floor.
pub fn permission_denied_render(service: &str, operation: &str, reason: &str) -> Value {
    json!({
        "render": "permission_denied",
        "action": format!("{service}:{operation}"),
        "reason": reason,
    })
}

/// Format an AWS timestamp as an ISO-8601 string (empty when absent).
pub fn dt_iso(dt: Option<&DateTime>) -> String {
    dt.and_then(|d| d.fmt(Format::DateTime).ok())
        .unwrap_or_default()
}

/// Epoch-seconds float for an AWS timestamp (None when absent).
pub fn dt_secs(dt: Option<&DateTime>) -> Option<f64> {
    dt.map(DateTime::as_secs_f64)
}

/// Classify only exact, reviewed service codes. SDK messages, response bodies,
/// request IDs and source chains may contain request data or credentials.
/// Transport/construction failures and unknown codes use the same fixed fallback.
pub fn err_msg<E: aws_smithy_types::error::metadata::ProvideErrorMetadata>(e: E) -> String {
    match e.code() {
        Some(
            "AccessDenied"
            | "AccessDeniedException"
            | "UnauthorizedException"
            | "UnauthorizedOperation",
        ) => "AWS denied this request. Check the selected account's permissions.",
        Some(
            "ExpiredToken"
            | "ExpiredTokenException"
            | "InvalidClientTokenId"
            | "UnrecognizedClientException",
        ) => "AWS credentials could not be verified. Reconnect the selected account.",
        Some(
            "Throttling"
            | "ThrottlingException"
            | "TooManyRequestsException"
            | "RequestLimitExceeded",
        ) => "AWS throttled this request. Try again later.",
        Some("ResourceNotFound" | "ResourceNotFoundException" | "NotFoundException") => {
            "The requested AWS resource was not found."
        }
        Some(
            "InvalidParameterException"
            | "InvalidParameterValueException"
            | "ValidationException"
            | "ValidationError"
            | "MalformedQueryException",
        ) => "AWS rejected the request parameters. Check the widget inputs.",
        Some(
            "ServiceUnavailable"
            | "ServiceUnavailableException"
            | "InternalFailure"
            | "InternalServerException"
            | "InternalServerError",
        ) => "The AWS service is temporarily unavailable. Try again later.",
        Some("ConflictException" | "ResourceInUseException") => {
            "AWS could not complete the request in the resource's current state."
        }
        _ => "The AWS request failed. Check the connection and try again.",
    }
    .to_string()
}

/// Dispatch a fetch to the named widget. Never panics; unknown names return an
/// inline error render.
pub async fn fetch(name: &str, ctx: &WidgetCtx) -> Value {
    match name {
        "aws-cli" => aws_cli::fetch(ctx).await,
        "cfn-stacks" => cfn_stacks::fetch(ctx).await,
        "cfn-stack-detail" => cfn_stack_detail::fetch(ctx).await,
        "cloudwatch-logs" => cloudwatch_logs::fetch(ctx).await,
        "logs-insights" => logs_insights::fetch(ctx).await,
        "log-tail" => log_tail::fetch(ctx).await,
        "errors-by-stack" => errors_by_stack::fetch(ctx).await,
        "resource-lookup" => resource_lookup::fetch(ctx).await,
        "pipeline-runs" => pipeline_runs::fetch(ctx).await,
        "codeartifact-packages" => codeartifact_packages::fetch(ctx).await,
        "codeartifact-package-version-history" => {
            codeartifact_packages::fetch_version_history(ctx).await
        }
        "pipeline-execution-detail" => pipeline_execution_detail::fetch(ctx).await,
        "codebuild-log" => codebuild_log::fetch(ctx).await,
        other => json!({
            "render": "raw_json",
            "data": {"error": format!("Unknown widget: {other}")}
        }),
    }
}

pub fn is_known(name: &str) -> bool {
    matches!(
        name,
        "aws-cli"
            | "cfn-stacks"
            | "cfn-stack-detail"
            | "cloudwatch-logs"
            | "logs-insights"
            | "log-tail"
            | "errors-by-stack"
            | "resource-lookup"
            | "pipeline-runs"
            | "codeartifact-packages"
            | "codeartifact-package-version-history"
            | "pipeline-execution-detail"
            | "codebuild-log"
    )
}

/// Check a widget's entry capability before resolving credentials. Individual
/// API calls still preflight inside each widget. Query start/stop is coupled by
/// the policy gate, so a disabled cleanup capability cannot trigger SSO/STS.
pub(crate) fn entry_operations(
    name: &str,
    inputs: &Value,
) -> &'static [(&'static str, &'static str)] {
    let mode = inputs.get("mode").and_then(Value::as_str).unwrap_or("");
    match name {
        "cfn-stacks" => &[("cloudformation", "ListStacks")],
        "cfn-stack-detail" => &[("cloudformation", "DescribeStackResources")],
        "cloudwatch-logs" => match mode {
            "streams" => &[("logs", "DescribeLogStreams")],
            "events" => &[("logs", "GetLogEvents")],
            _ => &[("logs", "DescribeLogGroups")],
        },
        "log-tail" => match mode {
            "list" => &[("lambda", "ListFunctions")],
            "streams" => &[("logs", "DescribeLogStreams")],
            "events" => &[("logs", "GetLogEvents")],
            _ => &[("logs", "FilterLogEvents")],
        },
        "logs-insights" if mode == "groups" => &[("logs", "DescribeLogGroups")],
        "logs-insights" => &[("logs", "StartQuery"), ("logs", "GetQueryResults")],
        "errors-by-stack" => &[
            ("logs", "DescribeLogGroups"),
            ("logs", "StartQuery"),
            ("logs", "GetQueryResults"),
        ],
        "resource-lookup" => &[("resourcegroupstaggingapi", "GetResources")],
        "pipeline-runs" => &[("codepipeline", "ListPipelineExecutions")],
        "pipeline-execution-detail" => &[("codepipeline", "ListActionExecutions")],
        "codeartifact-packages" => &[("codeartifact", "ListPackages")],
        "codeartifact-package-version-history" => &[("codeartifact", "DescribePackageVersion")],
        "codebuild-log" => &[("codebuild", "BatchGetBuilds")],
        _ => &[],
    }
}

/// Source viewer payload. `yaml` is the declared manifest, `py` is the actual
/// Rust implementation (so a user can see exactly what a widget does before
/// trusting it — the same intent as the Python `widget.getSource`).
pub fn get_source(name: &str) -> Value {
    let (yaml, py, doc): (&str, &str, &str) = match name {
        "aws-cli" => (
            AWS_CLI_YAML,
            include_str!("aws_cli.rs"),
            "AWS CLI Table — run a supported resource-read aws command and render its JSON as a table.",
        ),
        "logs-insights" => (
            LOGS_INSIGHTS_YAML,
            include_str!("logs_insights.rs"),
            "Logs Insights Query — run a CloudWatch Logs Insights query and view rows as a table.",
        ),
        "cfn-stacks" => (
            CFN_STACKS_YAML,
            include_str!("cfn_stacks.rs"),
            "CloudFormation Stacks — active stacks with status and resource count.",
        ),
        "cfn-stack-detail" => (
            CFN_STACK_DETAIL_YAML,
            include_str!("cfn_stack_detail.rs"),
            "CFN Stack Detail — DescribeStackResources + DescribeStackEvents.",
        ),
        "cloudwatch-logs" => (
            CLOUDWATCH_LOGS_YAML,
            include_str!("cloudwatch_logs.rs"),
            "CloudWatch Logs — search log groups, browse their streams, view events.",
        ),
        "log-tail" => (
            LOG_TAIL_YAML,
            include_str!("log_tail.rs"),
            "Lambda Logs — browse Lambda functions, streams, and CloudWatch log events.",
        ),
        "errors-by-stack" => (
            ERRORS_BY_STACK_YAML,
            include_str!("errors_by_stack.rs"),
            "Errors by Stack — CloudWatch errors grouped by stack over a selected time window.",
        ),
        "resource-lookup" => (
            RESOURCE_LOOKUP_YAML,
            include_str!("resource_lookup.rs"),
            "Resource Reverse Lookup — find the CloudFormation stack that owns a resource.",
        ),
        "pipeline-runs" => (
            PIPELINE_RUNS_YAML,
            include_str!("pipeline_runs.rs"),
            "Pipeline Runs — recent executions of one CodePipeline.",
        ),
        "codeartifact-packages" => (
            CODEARTIFACT_PACKAGES_YAML,
            include_str!("codeartifact_packages.rs"),
            "CodeArtifact Packages — latest package versions filtered by package prefix.",
        ),
        "codeartifact-package-version-history" => (
            CODEARTIFACT_PACKAGES_YAML,
            include_str!("codeartifact_packages.rs"),
            "CodeArtifact Package Version History — publish dates for one package's recent versions.",
        ),
        "pipeline-execution-detail" => (
            PIPELINE_EXECUTION_DETAIL_YAML,
            include_str!("pipeline_execution_detail.rs"),
            "Pipeline Execution Detail — action results, links, and logs for one CodePipeline run.",
        ),
        "codebuild-log" => (
            CODEBUILD_LOG_YAML,
            include_str!("codebuild_log.rs"),
            "CodeBuild Log — CloudWatch log lines for one CodeBuild build.",
        ),
        other => {
            return json!({"ok": false, "error": format!("Unknown widget: {other}")});
        }
    };
    json!({"ok": true, "name": name, "yaml": yaml, "py": py, "doc": doc})
}

const AWS_CLI_YAML: &str = r#"name: "AWS CLI Table"
version: 1
description: "Run a supported resource-read aws CLI command and render its JSON output as a table."
inputs:
  command: { type: string, required: true }
refresh: 0
permissions: ["exact CLI operation mapping and reviewed arguments required; policy.yaml can only restrict supported operations"]
"#;

const LOGS_INSIGHTS_YAML: &str = r#"name: "Logs Insights Query"
version: 1
description: "Run a CloudWatch Logs Insights query against one log group and view the result rows as a table."
inputs:
  mode: { type: string, default: "query" }
  log_group: { type: string, required: true }
  query: { type: string, required: true }
  range_seconds: { type: number, default: 3600 }
  name_pattern: { type: string, default: "" }
  max_groups: { type: number, default: 500 }
refresh: 0
permissions: [logs:DescribeLogGroups, logs:StartQuery, logs:GetQueryResults, logs:StopQuery]
"#;

const CFN_STACKS_YAML: &str = r#"name: "CloudFormation Stacks"
version: 1
description: "All non-deleted stacks in the active region with their current status and resource count."
inputs:
  name_prefix: { type: string, default: "" }
  status_filter:
    type: list
    default: []
refresh: 60s
permissions: [cloudformation:read]
"#;

const CFN_STACK_DETAIL_YAML: &str = r#"name: "CFN Stack Detail"
version: 1
description: "A CloudFormation stack's resources and recent events."
inputs:
  stack_name: { type: string, required: true }
refresh: 0
permissions: [cloudformation:read]
"#;

const CLOUDWATCH_LOGS_YAML: &str = r#"name: "CloudWatch Logs"
version: 1
description: "Search CloudWatch log groups, browse their streams, and view events."
inputs:
  mode: { type: string, default: "groups" }
  name_pattern: { type: string, default: "" }
  log_group: { type: string }
  log_stream: { type: string }
  max_groups: { type: number, default: 500 }
  max_streams: { type: number, default: 50 }
  limit: { type: number, default: 500 }
refresh: 0
permissions: [logs:DescribeLogGroups, logs:DescribeLogStreams, logs:GetLogEvents]
"#;

const LOG_TAIL_YAML: &str = r#"name: "Lambda Logs"
version: 1
description: "Browse Lambda functions, inspect metadata, choose a CloudWatch log stream, and view events."
inputs:
  mode: { type: string, default: "list" }
  log_group: { type: string }
  log_stream: { type: string }
  max_functions: { type: number, default: 500 }
  max_streams: { type: number, default: 50 }
  limit: { type: number, default: 500 }
  tail_minutes: { type: number, default: 5 }
  filter: { type: string, default: "" }
refresh: 15s
permissions: [lambda:ListFunctions, logs:DescribeLogStreams, logs:GetLogEvents, logs:FilterLogEvents]
"#;

const ERRORS_BY_STACK_YAML: &str = r#"name: "Errors by Stack"
version: 1
description: "CloudWatch errors grouped by stack over a selected time window."
inputs:
  hours: { type: number, default: 24 }
  log_group_pattern: { type: string, default: "/aws/lambda/" }
refresh: 120s
permissions: [logs:DescribeLogGroups, logs:StartQuery, logs:GetQueryResults, logs:StopQuery]
"#;

const RESOURCE_LOOKUP_YAML: &str = r#"name: "Resource Reverse Lookup"
version: 1
inputs:
  query: { type: string }
  max_results: { type: number, default: 25 }
refresh: 0
permissions: [tagging:GetResources, cloudformation:DescribeStackResources]
"#;

const PIPELINE_RUNS_YAML: &str = r#"name: "Pipeline Runs"
version: 1
description: "Recent runs of one CodePipeline, color-coded by status."
inputs:
  pipeline_name: { type: string, required: true }
  max_results: { type: number, default: 10 }
refresh: 30s
permissions: [codepipeline:read]
"#;

const CODEARTIFACT_PACKAGES_YAML: &str = r#"name: "CodeArtifact Packages"
version: 1
description: "Latest package versions in CodeArtifact filtered by package prefix."
inputs:
  domain: { type: string, default: "example-domain" }
  repository: { type: string, default: "example_pypi_repo" }
  package_prefix: { type: string, default: "example" }
  max_packages: { type: number, default: 50 }
refresh: 300s
permissions: [codeartifact:ListPackages, codeartifact:ListPackageVersions, codeartifact:DescribePackageVersion]
"#;

const PIPELINE_EXECUTION_DETAIL_YAML: &str = r#"name: "Pipeline Execution Detail"
version: 1
description: "Action results, links, and logs for one CodePipeline execution."
inputs:
  pipeline_name: { type: string, required: true }
  execution_id: { type: string, required: true }
refresh: 0
permissions: [codepipeline:read]
"#;

const CODEBUILD_LOG_YAML: &str = r#"name: "CodeBuild Log"
version: 1
description: "CloudWatch log lines for one CodeBuild build (a pipeline action)."
inputs:
  build_id: { type: string, required: true }
refresh: 0
permissions: [codebuild:read, logs:read]
"#;

#[cfg(test)]
mod diagnostic_tests {
    use super::err_msg;
    use aws_sdk_cloudwatchlogs::{
        error::SdkError, operation::get_query_results::GetQueryResultsError,
    };
    use aws_smithy_types::error::metadata::ErrorMetadata;

    #[test]
    fn reviewed_service_codes_use_fixed_messages_without_metadata_content() {
        for (code, expected) in [
            (
                "AccessDeniedException",
                "AWS denied this request. Check the selected account's permissions.",
            ),
            (
                "ExpiredTokenException",
                "AWS credentials could not be verified. Reconnect the selected account.",
            ),
            (
                "ThrottlingException",
                "AWS throttled this request. Try again later.",
            ),
            (
                "ResourceNotFoundException",
                "The requested AWS resource was not found.",
            ),
            (
                "MalformedQueryException",
                "AWS rejected the request parameters. Check the widget inputs.",
            ),
            (
                "ServiceUnavailableException",
                "The AWS service is temporarily unavailable. Try again later.",
            ),
            (
                "ConflictException",
                "AWS could not complete the request in the resource's current state.",
            ),
        ] {
            let error = ErrorMetadata::builder()
                .code(code)
                .message("SYNTHETIC_PRIVATE_MESSAGE_MARKER")
                .custom("request_id", "SYNTHETIC_PRIVATE_REQUEST_MARKER")
                .build();
            assert_eq!(err_msg(error), expected);
        }
    }

    #[test]
    fn unknown_codes_and_sdk_source_chains_are_never_echoed() {
        let fallback = "The AWS request failed. Check the connection and try again.";
        for code in [
            "SYNTHETIC_PRIVATE_CODE_MARKER",
            "AccessDeniedException:SYNTHETIC_PRIVATE_CODE_MARKER",
        ] {
            let error = ErrorMetadata::builder()
                .code(code)
                .message("SYNTHETIC_PRIVATE_MESSAGE_MARKER")
                .build();
            assert_eq!(err_msg(error), fallback);
        }
        let error: SdkError<GetQueryResultsError> = SdkError::construction_failure(
            std::io::Error::other("SYNTHETIC_PRIVATE_SOURCE_MARKER"),
        );
        assert_eq!(err_msg(error), fallback);
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::{aws::policy::Policy, test_aws, test_support::TestDir};
    use std::sync::{atomic::Ordering, Arc};

    #[tokio::test]
    async fn query_workflows_require_cleanup_before_any_sdk_work() {
        for policy_text in [
            "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: ['logs:StopQuery']\n",
            "statements:\n  - effect: Allow\n    action: ['sso:GetRoleCredentials', 'logs:DescribeLogGroups', 'logs:StartQuery', 'logs:GetQueryResults']\n",
        ] {
            for widget in ["logs-insights", "errors-by-stack"] {
                let dir = TestDir::new();
                let counters = Arc::new(test_aws::Counters::default());
                let ctx = WidgetCtx {
                    runtime: Runtime::for_test(dir.paths()),
                    sdk: test_aws::sdk_config_with_counters(counters.clone()),
                    account_id: "acct-demo-fixture".into(),
                    region: "us-east-1".into(),
                    widget_name: widget.into(),
                    inputs: json!({"log_group": "/demo/test", "query": "fields @message"}),
                    policy: Policy::parse(policy_text).map_err(|e| e.message),
                    cli: None,
                };
                let denied = fetch(widget, &ctx).await;
                assert_eq!(denied["render"], "permission_denied", "{widget}");
                assert!(denied["reason"].as_str().unwrap().contains("logs:StopQuery"));
                assert_eq!(counters.credentials.load(Ordering::SeqCst), 0);
                assert_eq!(counters.transport.load(Ordering::SeqCst), 0);
            }
        }
    }
}
