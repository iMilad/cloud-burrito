//! AWS CLI Table — run a user-supplied *read-only* `aws` command and render
//! its JSON output as a table.
//!
//! Commands resolve through an exact reviewed operation mapping and a closed
//! argument schema before the narrowing policy and injected process boundary.
//! No shell is used. Credential issuance and query lifecycle operations are
//! unavailable through this widget; the dedicated workflows own those paths.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::aws::guard;
use crate::process::CliRequest;

use super::WidgetCtx;

/// Refuse to parse outputs bigger than this — use --query / --max-items.
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let command = ctx.input_str("command", "");
    if command.trim().is_empty() {
        return json!({
            "ok": false,
            "error": "command is required — e.g. `aws sts get-caller-identity`",
        });
    }
    let parsed = match parse_cli_command(&command) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let action = format!("{}:{}", parsed.service, parsed.operation);
    if let Some(denied) = ctx.preflight_cli(&parsed.service, &parsed.operation) {
        return denied;
    }
    run_cli(ctx, &parsed, &action).await
}

async fn run_cli(ctx: &WidgetCtx, parsed: &ParsedCli, action: &str) -> Value {
    let Some(access) = &ctx.cli else {
        return json!({"ok": false, "error_type": "CliCredentialsUnavailable", "error": "Select a verified account before running a command"});
    };
    if access.cancellation.is_cancelled() {
        return json!({"ok": false, "error_type": "Cancelled", "error": "Command cancelled"});
    }
    if !access
        .credentials
        .expiry()
        .is_some_and(|expiry| crate::runtime::unexpired(expiry, ctx.runtime.clock.now_epoch(), 0.0))
    {
        return json!({"ok": false, "error_type": "CredentialsExpired", "error": "Verified credentials expired; reconnect the account"});
    }
    let output = match ctx
        .runtime
        .process
        .run(CliRequest {
            argv: parsed.argv.clone(),
            region: ctx.region.clone(),
            credentials: access.credentials.clone(),
            cancellation: access.cancellation.clone(),
        })
        .await
    {
        Ok(out) => out,
        Err(error) if error == crate::process::CLEANUP_FAILED => {
            return json!({"ok": false, "error_type": "CliCleanupFailed", "error": error});
        }
        Err(error) => {
            return json!({"ok": false, "error": safe_diagnostic(&error, &access.credentials)})
        }
    };
    if !output.success {
        // Child diagnostics can contain request data, credentials or proxy URLs.
        // Do not echo them into the webview or audit log.
        return json!({
            "ok": false,
            "error": "AWS CLI command failed; check the selected account, permissions and inputs",
        });
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return json!({
            "ok": false,
            "error": "output larger than 2 MB — narrow it with --query or --max-items",
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if contains_credentials(&stdout, &access.credentials) {
        return json!({"ok": false, "error": "CLI output contained authentication material and was not displayed"});
    }
    let trimmed = stdout.trim();
    let mut out = if trimmed.is_empty() {
        json!({"render": "table", "columns": [], "rows": []})
    } else {
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) if value_contains_credentials(&v, &access.credentials) => {
                return json!({"ok": false, "error": "CLI output contained authentication material and was not displayed"});
            }
            Ok(v) => table_model(&v),
            Err(e) => return json!({"ok": false, "error": format!("output was not JSON: {e}")}),
        }
    };
    out["action"] = json!(action);
    out["account_id"] = json!(ctx.account_id);
    out["region"] = json!(ctx.region);
    out
}

fn contains_credentials(text: &str, credentials: &aws_credential_types::Credentials) -> bool {
    [
        credentials.access_key_id(),
        credentials.secret_access_key(),
        credentials.session_token().unwrap_or(""),
    ]
    .iter()
    .any(|value| !value.is_empty() && text.contains(value))
}

fn safe_diagnostic(text: &str, credentials: &aws_credential_types::Credentials) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if contains_credentials(text, credentials) || contains_credentials(&clean, credentials) {
        return "AWS CLI execution failed; authentication details were withheld".into();
    }
    clean.chars().take(400).collect()
}

fn value_contains_credentials(
    value: &Value,
    credentials: &aws_credential_types::Credentials,
) -> bool {
    match value {
        Value::String(text) => contains_credentials(text, credentials),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_credentials(value, credentials)),
        Value::Object(values) => values.iter().any(|(key, value)| {
            contains_credentials(key, credentials) || value_contains_credentials(value, credentials)
        }),
        _ => false,
    }
}

/// A validated, ready-to-spawn CLI command.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedCli {
    /// Exact reviewed CLI service segment.
    pub service: String,
    /// Exact reviewed canonical application operation for gating and auditing.
    pub operation: String,
    /// argv after the `aws` binary, with `--output json` appended.
    pub argv: Vec<String>,
}

const MAX_COMMAND_BYTES: usize = 16 * 1024;
const MAX_COMMAND_TOKENS: usize = 128;

/// Resolve a reviewed operation and rebuild argv from its closed argument
/// schema. AWS CLI abbreviation, structured input and parameter-file loading
/// are never delegated to the executable.
pub fn parse_cli_command(command: &str) -> Result<ParsedCli, String> {
    if command.len() > MAX_COMMAND_BYTES || command.chars().any(char::is_control) {
        return Err("command must be one line and at most 16 KiB".into());
    }
    let tokens =
        shell_words::split(command.trim()).map_err(|e| format!("could not parse command: {e}"))?;
    if tokens.first().map(String::as_str) != Some("aws") {
        return Err("command must start with `aws`".to_string());
    }
    if tokens.len() < 3 {
        return Err("expected `aws <service> <operation> [args...]`".to_string());
    }
    if tokens.len() > MAX_COMMAND_TOKENS {
        return Err("command has too many arguments (maximum 128 tokens)".into());
    }
    for token in &tokens {
        let lower = token.to_ascii_lowercase();
        if ["file://", "fileb://", "http://", "https://"]
            .iter()
            .any(|prefix| lower.contains(prefix))
        {
            return Err("file and URL parameter loading is not supported".into());
        }
    }
    let operation = guard::cli_operation(&tokens[1], &tokens[2])
        .ok_or_else(|| "this exact AWS CLI operation is not supported by this app".to_string())?;
    let schema = argument_schema(operation.service, operation.operation)
        .ok_or_else(|| "this operation has no reviewed CLI argument schema".to_string())?;
    let mut argv = vec![tokens[1].clone(), tokens[2].clone()];
    argv.extend(validate_arguments(&tokens[3..], &schema)?);
    argv.push("--output".to_string());
    argv.push("json".to_string());
    Ok(ParsedCli {
        service: operation.service.to_string(),
        operation: operation.operation.to_string(),
        argv,
    })
}

#[derive(Clone, Copy)]
enum ValueType {
    Scalar(usize, usize),
    Opaque(usize),
    Expression(usize, usize),
    Number(u64, u64),
    Enum(&'static [&'static str]),
    AccountId,
    PipelineName,
    LogStreamName,
}

#[derive(Clone, Copy)]
enum Arity {
    Flag,
    One(ValueType),
    List(ValueType, usize),
}

#[derive(Clone, Copy)]
struct Argument {
    name: &'static str,
    arity: Arity,
    required: bool,
}

const fn optional(name: &'static str, arity: Arity) -> Argument {
    Argument {
        name,
        arity,
        required: false,
    }
}

const fn required(name: &'static str, kind: ValueType) -> Argument {
    Argument {
        name,
        arity: Arity::One(kind),
        required: true,
    }
}

#[derive(Clone, Copy)]
enum Pagination {
    None,
    Items,
    Pages(u64),
}

struct ArgumentSchema {
    arguments: Vec<Argument>,
}

const PACKAGE_FORMATS: &[&str] = &[
    "npm", "pypi", "maven", "nuget", "generic", "ruby", "swift", "cargo",
];
const STACK_STATUSES: &[&str] = &[
    "CREATE_IN_PROGRESS",
    "CREATE_FAILED",
    "CREATE_COMPLETE",
    "ROLLBACK_IN_PROGRESS",
    "ROLLBACK_FAILED",
    "ROLLBACK_COMPLETE",
    "DELETE_IN_PROGRESS",
    "DELETE_FAILED",
    "DELETE_COMPLETE",
    "UPDATE_IN_PROGRESS",
    "UPDATE_COMPLETE_CLEANUP_IN_PROGRESS",
    "UPDATE_COMPLETE",
    "UPDATE_FAILED",
    "UPDATE_ROLLBACK_IN_PROGRESS",
    "UPDATE_ROLLBACK_FAILED",
    "UPDATE_ROLLBACK_COMPLETE_CLEANUP_IN_PROGRESS",
    "UPDATE_ROLLBACK_COMPLETE",
    "REVIEW_IN_PROGRESS",
    "IMPORT_IN_PROGRESS",
    "IMPORT_COMPLETE",
    "IMPORT_ROLLBACK_IN_PROGRESS",
    "IMPORT_ROLLBACK_FAILED",
    "IMPORT_ROLLBACK_COMPLETE",
];

/// Reviewed against the official CLI reference for each exact command.
/// Structured filters, cross-account discovery, unmasking and other flags
/// outside these schemas remain unsupported, even with wildcard user policy.
fn argument_schema(service: &str, operation: &str) -> Option<ArgumentSchema> {
    use Arity::{Flag, List, One};
    use ValueType::{
        AccountId, Enum, Expression, LogStreamName, Number, Opaque, PipelineName, Scalar,
    };

    let (mut arguments, pagination) = match (service, operation) {
        ("sts", "GetCallerIdentity") => (vec![], Pagination::None),
        ("cloudformation", "ListStacks") => (
            vec![optional(
                "--stack-status-filter",
                List(Enum(STACK_STATUSES), STACK_STATUSES.len()),
            )],
            Pagination::Items,
        ),
        ("cloudformation", "DescribeStackResources") => (
            vec![
                optional("--stack-name", One(Scalar(1, 2048))),
                optional("--physical-resource-id", One(Scalar(1, 2048))),
                optional("--logical-resource-id", One(Scalar(1, 2048))),
            ],
            Pagination::None,
        ),
        ("cloudformation", "DescribeStackEvents") => (
            vec![required("--stack-name", Scalar(1, 2048))],
            Pagination::Items,
        ),
        ("logs", "DescribeLogGroups") => (
            vec![
                optional("--log-group-name-prefix", One(Scalar(1, 512))),
                optional("--log-group-name-pattern", One(Scalar(1, 512))),
                optional(
                    "--log-group-class",
                    One(Enum(&["STANDARD", "INFREQUENT_ACCESS", "DELIVERY"])),
                ),
            ],
            Pagination::Pages(50),
        ),
        ("logs", "GetQueryResults") => (
            vec![
                required("--query-id", Scalar(1, 256)),
                optional("--next-token", One(Opaque(1024))),
                optional("--max-items", One(Number(0, 10_000))),
            ],
            Pagination::None,
        ),
        ("logs", "FilterLogEvents") => (
            vec![
                optional("--log-group-name", One(Scalar(1, 512))),
                optional("--log-group-identifier", One(Scalar(1, 2048))),
                optional("--log-stream-names", List(LogStreamName, 100)),
                optional("--log-stream-name-prefix", One(LogStreamName)),
                optional("--start-time", One(Number(0, i64::MAX as u64))),
                optional("--end-time", One(Number(0, i64::MAX as u64))),
                optional("--filter-pattern", One(Expression(0, 1024))),
            ],
            Pagination::Pages(10_000),
        ),
        ("logs", "GetLogEvents") => (
            vec![
                optional("--log-group-name", One(Scalar(1, 512))),
                optional("--log-group-identifier", One(Scalar(1, 2048))),
                required("--log-stream-name", LogStreamName),
                optional("--start-time", One(Number(0, i64::MAX as u64))),
                optional("--end-time", One(Number(0, i64::MAX as u64))),
                optional("--next-token", One(Opaque(4096))),
                optional("--limit", One(Number(1, 10_000))),
                optional("--start-from-head", Flag),
                optional("--no-start-from-head", Flag),
            ],
            Pagination::None,
        ),
        ("logs", "DescribeLogStreams") => (
            vec![
                optional("--log-group-name", One(Scalar(1, 512))),
                optional("--log-group-identifier", One(Scalar(1, 2048))),
                optional("--log-stream-name-prefix", One(LogStreamName)),
                optional("--order-by", One(Enum(&["LogStreamName", "LastEventTime"]))),
                optional("--descending", Flag),
                optional("--no-descending", Flag),
            ],
            Pagination::Pages(50),
        ),
        ("codepipeline", "ListPipelines") => (vec![], Pagination::Pages(1000)),
        ("codepipeline", "ListPipelineExecutions" | "ListActionExecutions") => (
            vec![required("--pipeline-name", PipelineName)],
            Pagination::Pages(100),
        ),
        ("codebuild", "BatchGetBuilds") => (
            vec![Argument {
                name: "--ids",
                arity: List(Scalar(1, 2048), 100),
                required: true,
            }],
            Pagination::None,
        ),
        ("codeartifact", "ListPackages") => (
            vec![
                optional("--format", One(Enum(PACKAGE_FORMATS))),
                optional("--namespace", One(Scalar(1, 255))),
                optional("--package-prefix", One(Scalar(1, 255))),
                optional("--publish", One(Enum(&["ALLOW", "BLOCK"]))),
                optional("--upstream", One(Enum(&["ALLOW", "BLOCK"]))),
            ],
            Pagination::Pages(1000),
        ),
        ("codeartifact", "ListPackageVersions") => (
            vec![
                required("--format", Enum(PACKAGE_FORMATS)),
                required("--package", Scalar(1, 255)),
                optional("--namespace", One(Scalar(1, 255))),
                optional(
                    "--status",
                    One(Enum(&[
                        "Published",
                        "Unfinished",
                        "Unlisted",
                        "Archived",
                        "Disposed",
                        "Deleted",
                    ])),
                ),
                optional("--sort-by", One(Enum(&["PUBLISHED_TIME"]))),
                optional(
                    "--origin-type",
                    One(Enum(&["INTERNAL", "EXTERNAL", "UNKNOWN"])),
                ),
            ],
            Pagination::Pages(1000),
        ),
        ("codeartifact", "DescribePackageVersion") => (
            vec![
                required("--format", Enum(PACKAGE_FORMATS)),
                required("--package", Scalar(1, 255)),
                required("--package-version", Scalar(1, 255)),
                optional("--namespace", One(Scalar(1, 255))),
            ],
            Pagination::None,
        ),
        ("lambda", "ListFunctions") => (
            vec![
                optional("--function-version", One(Enum(&["ALL"]))),
                optional("--master-region", One(Scalar(1, 50))),
            ],
            Pagination::Pages(50),
        ),
        ("resourcegroupstaggingapi", "GetResources") => (
            vec![optional(
                "--resource-type-filters",
                List(Scalar(1, 256), 100),
            )],
            Pagination::Pages(100),
        ),
        _ => return None,
    };
    if service == "codeartifact" {
        arguments.extend([
            required("--domain", Scalar(2, 50)),
            required("--repository", Scalar(2, 100)),
            optional("--domain-owner", One(AccountId)),
        ]);
    }
    if !matches!(pagination, Pagination::None) {
        arguments.extend([
            optional("--starting-token", One(Opaque(4096))),
            optional("--max-items", One(Number(1, 1000))),
            optional("--no-paginate", Flag),
        ]);
    }
    if let Pagination::Pages(max) = pagination {
        arguments.push(optional("--page-size", One(Number(1, max))));
    }
    arguments.push(optional("--query", One(Expression(1, 4096))));
    Some(ArgumentSchema { arguments })
}

fn validate_value(name: &str, value: &str, kind: ValueType) -> Result<(), String> {
    let valid = match kind {
        ValueType::Scalar(min, max) => {
            (min..=max).contains(&value.len())
                && !value.trim().is_empty()
                && !value.trim_start().starts_with(['{', '['])
                && !value.contains('=')
        }
        ValueType::Opaque(max) => !value.trim().is_empty() && value.len() <= max,
        ValueType::Expression(min, max) => (min..=max).contains(&value.len()),
        ValueType::Number(min, max) => {
            !value.is_empty()
                && value.bytes().all(|c| c.is_ascii_digit())
                && value.parse::<u64>().is_ok_and(|n| (min..=max).contains(&n))
        }
        ValueType::Enum(values) => values.contains(&value),
        ValueType::AccountId => value.len() == 12 && value.bytes().all(|c| c.is_ascii_digit()),
        ValueType::PipelineName => {
            (1..=100).contains(&value.len())
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".@_-".contains(&c))
        }
        ValueType::LogStreamName => (1..=512).contains(&value.len()) && !value.contains([':', '*']),
    };
    if !valid || value.starts_with('-') {
        return Err(format!("invalid or unsupported value for {name}"));
    }
    Ok(())
}

fn validate_list_value(name: &str, value: &str, kind: ValueType) -> Result<(), String> {
    // The CLI may unpack JSON list syntax. Accept separate reviewed values only,
    // so one token cannot bypass this parser's list element validation or cap.
    if value.trim_start().starts_with(['[', '{']) {
        return Err(format!("structured list input is not supported for {name}"));
    }
    validate_value(name, value, kind)
}

fn validate_arguments(tokens: &[String], schema: &ArgumentSchema) -> Result<Vec<String>, String> {
    let mut seen: HashMap<&str, Vec<String>> = HashMap::new();
    let mut argv = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (name, inline) = tokens[index]
            .split_once('=')
            .map_or((tokens[index].as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        let argument = schema
            .arguments
            .iter()
            .find(|argument| argument.name == name)
            .ok_or_else(|| format!("argument {name} is not supported for this operation"))?;
        if seen.contains_key(name) {
            return Err(format!("repeated argument {name} is not allowed"));
        }
        index += 1;
        let mut values = Vec::new();
        match argument.arity {
            Arity::Flag => {
                if inline.is_some() {
                    return Err(format!("{name} is a boolean switch and takes no value"));
                }
            }
            Arity::One(kind) => {
                let value = if let Some(value) = inline {
                    value
                } else {
                    let value = tokens
                        .get(index)
                        .ok_or_else(|| format!("missing value for {name}"))?;
                    index += 1;
                    value
                };
                validate_value(name, value, kind)?;
                values.push(value.to_string());
            }
            Arity::List(kind, max) => {
                if let Some(value) = inline {
                    validate_list_value(name, value, kind)?;
                    values.push(value.to_string());
                }
                while index < tokens.len() && !tokens[index].starts_with('-') {
                    validate_list_value(name, &tokens[index], kind)?;
                    values.push(tokens[index].clone());
                    index += 1;
                }
                if values.is_empty() || values.len() > max {
                    return Err(format!("{name} requires between 1 and {max} values"));
                }
            }
        }
        argv.push(name.to_string());
        argv.extend(values.iter().cloned());
        seen.insert(argument.name, values);
    }
    for argument in &schema.arguments {
        if argument.required && !seen.contains_key(argument.name) {
            return Err(format!("missing required argument {}", argument.name));
        }
    }
    validate_relationships(schema, &seen)?;
    Ok(argv)
}

fn validate_relationships(
    schema: &ArgumentSchema,
    seen: &HashMap<&str, Vec<String>>,
) -> Result<(), String> {
    let has = |name: &str| seen.contains_key(name);
    let value = |name: &str| {
        seen.get(name)
            .and_then(|values| values.first())
            .map(String::as_str)
    };
    for (left, right) in [
        ("--stack-name", "--physical-resource-id"),
        ("--log-group-name", "--log-group-identifier"),
        ("--log-group-name-prefix", "--log-group-name-pattern"),
        ("--log-stream-names", "--log-stream-name-prefix"),
        ("--descending", "--no-descending"),
        ("--start-from-head", "--no-start-from-head"),
    ] {
        if has(left) && has(right) {
            return Err(format!("{left} and {right} cannot be combined"));
        }
    }
    for (left, right) in [
        ("--stack-name", "--physical-resource-id"),
        ("--log-group-name", "--log-group-identifier"),
    ] {
        if schema
            .arguments
            .iter()
            .any(|argument| argument.name == right)
            && !has(left)
            && !has(right)
        {
            return Err(format!("exactly one of {left} or {right} is required"));
        }
    }
    if has("--no-paginate")
        && ["--starting-token", "--max-items", "--page-size"]
            .iter()
            .any(|flag| has(flag))
    {
        return Err("--no-paginate cannot be combined with pagination arguments".into());
    }
    if let (Some(start), Some(end)) = (value("--start-time"), value("--end-time")) {
        if start.parse::<u64>().unwrap_or(0) > end.parse::<u64>().unwrap_or(0) {
            return Err("--start-time must not exceed --end-time".into());
        }
    }
    if has("--log-stream-name-prefix") && value("--order-by") == Some("LastEventTime") {
        return Err(
            "--log-stream-name-prefix cannot be combined with LastEventTime ordering".into(),
        );
    }
    if has("--master-region") && value("--function-version") != Some("ALL") {
        return Err("--master-region requires --function-version ALL".into());
    }
    if has("--package-version") {
        match value("--format") {
            Some("maven" | "swift" | "generic") if !has("--namespace") => {
                return Err("this package format requires --namespace".into());
            }
            Some("pypi" | "nuget" | "ruby" | "cargo") if has("--namespace") => {
                return Err("this package format does not support --namespace".into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Longest cell text before nested JSON gets truncated.
const MAX_CELL_CHARS: usize = 120;

/// Map arbitrary CLI JSON output onto the closed `table` render shape, falling
/// back to `raw_json` when nothing tabular is recognizable.
pub fn table_model(value: &Value) -> Value {
    match value {
        Value::Array(items) => array_table(items).unwrap_or_else(|| raw_json(value)),
        Value::Object(map) => {
            // The CLI's usual top level: one array of results plus scalar
            // siblings (NextToken and friends). Unwrap to the array.
            let array_keys: Vec<&String> = map
                .iter()
                .filter(|(_, v)| v.is_array())
                .map(|(k, _)| k)
                .collect();
            if array_keys.len() == 1 && map.values().all(|v| v.is_array() || is_scalar(v)) {
                let items = map[array_keys[0]].as_array().expect("filtered on is_array");
                return array_table(items).unwrap_or_else(|| raw_json(value));
            }
            // Flat object of scalars (sts get-caller-identity) -> key/value.
            if !map.is_empty() && map.values().all(is_scalar) {
                let rows: Vec<Value> = map
                    .iter()
                    .map(|(k, v)| json!({"key": k, "value": cell(v)}))
                    .collect();
                return json!({"render": "table", "columns": ["key", "value"], "rows": rows});
            }
            raw_json(value)
        }
        _ => raw_json(value),
    }
}

/// Array of objects -> union columns; array of scalars -> one `value` column;
/// empty -> empty table; mixed shapes -> None (caller falls back to raw JSON).
fn array_table(items: &[Value]) -> Option<Value> {
    if items.is_empty() {
        return Some(json!({"render": "table", "columns": [], "rows": []}));
    }
    if items.iter().all(|v| v.is_object()) {
        let mut columns: Vec<String> = Vec::new();
        let mut rows: Vec<Value> = Vec::new();
        for item in items {
            let map = item.as_object().expect("checked is_object");
            let mut row = serde_json::Map::new();
            for (k, v) in map {
                if !columns.contains(k) {
                    columns.push(k.clone());
                }
                row.insert(k.clone(), cell(v));
            }
            rows.push(Value::Object(row));
        }
        return Some(json!({"render": "table", "columns": columns, "rows": rows}));
    }
    if items.iter().all(is_scalar) {
        let rows: Vec<Value> = items.iter().map(|v| json!({"value": cell(v)})).collect();
        return Some(json!({"render": "table", "columns": ["value"], "rows": rows}));
    }
    None
}

fn is_scalar(v: &Value) -> bool {
    !v.is_array() && !v.is_object()
}

/// Scalars pass through; nested structures become truncated JSON text.
fn cell(v: &Value) -> Value {
    if is_scalar(v) {
        return v.clone();
    }
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.chars().count() > MAX_CELL_CHARS {
        let truncated: String = s.chars().take(MAX_CELL_CHARS).collect();
        Value::String(format!("{truncated}…"))
    } else {
        Value::String(s)
    }
}

fn raw_json(v: &Value) -> Value {
    json!({"render": "raw_json", "data": v})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use futures::future::BoxFuture;
    use parking_lot::Mutex;
    use serde_json::json;

    use crate::aws::policy::{self, Policy};
    use crate::process::{ProcessOutput, ProcessRunner};
    use crate::runtime::{test_sdk_config, Runtime};
    use crate::test_support::TestDir;

    #[derive(Default)]
    struct FakeProcessRunner {
        responses: Mutex<VecDeque<Result<ProcessOutput, String>>>,
        requests: Mutex<Vec<CliRequest>>,
        calls: AtomicUsize,
    }

    impl FakeProcessRunner {
        fn with_response(response: Result<ProcessOutput, String>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(VecDeque::from([response])),
                ..Self::default()
            })
        }
    }

    impl ProcessRunner for FakeProcessRunner {
        fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.requests.lock().push(request);
                self.responses
                    .lock()
                    .pop_front()
                    .expect("unexpected CLI process request")
            })
        }
    }

    fn fetch_context(
        dir: &TestDir,
        process: Arc<FakeProcessRunner>,
        policy: Result<Policy, String>,
    ) -> WidgetCtx {
        let mut runtime = Runtime::for_test(dir.paths());
        runtime.process = process;
        WidgetCtx {
            sdk: test_sdk_config(),
            runtime,
            account_id: "acct-fixture".into(),
            region: "eu-west-1".into(),
            widget_name: "aws-cli".into(),
            inputs: json!({"command": "aws cloudformation list-stacks"}),
            policy,
            cli: Some(crate::widgets::CliAccess {
                credentials: aws_credential_types::Credentials::new(
                    "CB_SYNTHETIC_KEY",
                    "CB_SYNTHETIC_SECRET",
                    Some("CB_SYNTHETIC_TOKEN".into()),
                    Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000)),
                    "widget-test-only",
                ),
                cancellation: crate::process::ProcessCancellation::new(),
            }),
        }
    }

    fn allowed_fetch_policy() -> Result<Policy, String> {
        Policy::parse(
            "statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials, cloudformation:ListStacks]\n",
        )
        .map_err(|error| error.message)
    }

    #[tokio::test]
    async fn wildcard_policy_cannot_spawn_unreviewed_batch_delete() {
        let dir = TestDir::new();
        let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
            stdout: b"{}".to_vec(),
            stderr: Vec::new(),
            success: true,
        }));
        let policy = Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n")
            .map_err(|error| error.message);
        let mut ctx = fetch_context(&dir, process.clone(), policy);
        ctx.inputs = json!({
            "command": "aws ecr batch-delete-image --repository-name demo-repository --image-ids imageTag=demo",
        });

        let result = fetch(&ctx).await;

        assert_eq!(process.calls.load(Ordering::SeqCst), 0);
        assert!(process.requests.lock().is_empty());
        assert!(result["ok"] == false || result["render"] == "permission_denied");
    }

    #[tokio::test]
    async fn allowed_fetch_uses_injected_process_and_renders_its_output() {
        let dir = TestDir::new();
        let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
            stdout: br#"{"StackSummaries":[{"StackName":"demo-stack"}]}"#.to_vec(),
            stderr: Vec::new(),
            success: true,
        }));
        let ctx = fetch_context(&dir, process.clone(), allowed_fetch_policy());

        let result = fetch(&ctx).await;

        assert_eq!(result["render"], "table");
        assert_eq!(result["rows"], json!([{"StackName": "demo-stack"}]));
        assert_eq!(result["action"], "cloudformation:ListStacks");
        assert_eq!(result["account_id"], "acct-fixture");
        assert_eq!(result["region"], "eu-west-1");
        assert_eq!(process.calls.load(Ordering::SeqCst), 1);
        let requests = process.requests.lock();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].argv,
            vec!["cloudformation", "list-stacks", "--output", "json"]
        );
        assert_eq!(requests[0].region, "eu-west-1");
        assert_eq!(requests[0].credentials.access_key_id(), "CB_SYNTHETIC_KEY");
    }

    #[tokio::test]
    async fn denied_fetch_never_runs_injected_process() {
        let explicit_deny = Policy::parse(
            "statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials, cloudformation:ListStacks]\n  - effect: Deny\n    action: [cloudformation:ListStacks]\n",
        )
        .map_err(|error| error.message);
        let no_credential_permission = Policy::parse(
            "statements:\n  - effect: Allow\n    action: [cloudformation:ListStacks]\n",
        )
        .map_err(|error| error.message);
        let cases = [
            (
                Policy::parse(
                    "statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials]\n",
                )
                .map_err(|error| error.message),
                "cloudformation:ListStacks",
            ),
            (explicit_deny, "cloudformation:ListStacks"),
            (no_credential_permission, "sso:GetRoleCredentials"),
            (
                Err("invalid fixture policy".into()),
                "sso:GetRoleCredentials",
            ),
        ];
        for (policy, expected_action) in cases {
            let dir = TestDir::new();
            let process = Arc::new(FakeProcessRunner::default());
            let ctx = fetch_context(&dir, process.clone(), policy);

            let result = fetch(&ctx).await;

            assert_eq!(result["render"], "permission_denied");
            assert_eq!(result["action"], expected_action);
            assert!(!result.to_string().contains("add `-"));
            assert_eq!(process.calls.load(Ordering::SeqCst), 0);
            assert!(process.requests.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn injected_process_failures_preserve_cli_error_rendering() {
        for (response, expected_error) in [
            (
                Err("command timed out after 30s (killed)".into()),
                "command timed out after 30s (killed)",
            ),
            (
                Ok(ProcessOutput {
                    stdout: Vec::new(),
                    stderr: b" fixture failure \n".to_vec(),
                    success: false,
                }),
                "AWS CLI command failed; check the selected account, permissions and inputs",
            ),
        ] {
            let dir = TestDir::new();
            let process = FakeProcessRunner::with_response(response);
            let ctx = fetch_context(&dir, process.clone(), allowed_fetch_policy());

            let result = fetch(&ctx).await;

            assert_eq!(result, json!({"ok": false, "error": expected_error}));
            assert_eq!(process.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn missing_expired_or_cancelled_cli_authority_never_reaches_process() {
        for case in ["missing", "expired", "cancelled"] {
            let dir = TestDir::new();
            let process = Arc::new(FakeProcessRunner::default());
            let mut ctx = fetch_context(&dir, process.clone(), allowed_fetch_policy());
            match case {
                "missing" => ctx.cli = None,
                "expired" => {
                    ctx.cli.as_mut().unwrap().credentials = aws_credential_types::Credentials::new(
                        "CB_SYNTHETIC_KEY",
                        "CB_SYNTHETIC_SECRET",
                        Some("CB_SYNTHETIC_TOKEN".into()),
                        Some(std::time::UNIX_EPOCH),
                        "expired-widget-fixture",
                    );
                }
                _ => ctx.cli.as_ref().unwrap().cancellation.cancel(),
            }
            assert_eq!(fetch(&ctx).await["ok"], false);
            assert_eq!(process.calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn cli_diagnostics_and_results_do_not_expose_captured_credentials() {
        let marker = "CB_SYNTHETIC_TOKEN";
        for response in [
            Err(format!("runner failure containing {marker}")),
            Err("runner failure containing CB_SYNTHETIC_\nTOKEN".into()),
            Ok(ProcessOutput {
                stdout: Vec::new(),
                stderr: format!("private diagnostic {marker}").into_bytes(),
                success: false,
            }),
            Ok(ProcessOutput {
                stdout: serde_json::to_vec(&json!({"unexpected": marker})).unwrap(),
                stderr: Vec::new(),
                success: true,
            }),
            Ok(ProcessOutput {
                stdout: br#"{"unexpected":"\u0043B_SYNTHETIC_TOKEN"}"#.to_vec(),
                stderr: Vec::new(),
                success: true,
            }),
            Ok(ProcessOutput {
                stdout: br#"{"\u0043B_SYNTHETIC_TOKEN":"unexpected key"}"#.to_vec(),
                stderr: Vec::new(),
                success: true,
            }),
        ] {
            let dir = TestDir::new();
            let process = FakeProcessRunner::with_response(response);
            let ctx = fetch_context(&dir, process.clone(), allowed_fetch_policy());
            let result = fetch(&ctx).await;
            assert_eq!(result["ok"], false);
            assert!(!result.to_string().contains(marker));
            let audit = std::fs::read_to_string(ctx.runtime.paths.data_file("audit.log")).unwrap();
            assert!(!audit.contains(marker));
        }
    }

    #[test]
    fn parses_basic_read_only_command() {
        let p = parse_cli_command("aws cloudformation list-stacks").unwrap();
        assert_eq!(p.service, "cloudformation");
        assert_eq!(p.operation, "ListStacks");
        assert_eq!(
            p.argv,
            vec!["cloudformation", "list-stacks", "--output", "json"]
        );
    }

    #[tokio::test]
    async fn every_reviewed_schema_reaches_fake_process_with_default_policy() {
        let cases = [
            ("aws sts get-caller-identity --query Account", "sts", "GetCallerIdentity"),
            ("aws cloudformation list-stacks --stack-status-filter CREATE_COMPLETE UPDATE_COMPLETE --max-items 20 --starting-token opaque==", "cloudformation", "ListStacks"),
            ("aws cloudformation describe-stack-resources --stack-name demo-stack --logical-resource-id DemoFunction", "cloudformation", "DescribeStackResources"),
            ("aws cloudformation describe-stack-events --stack-name demo-stack --no-paginate", "cloudformation", "DescribeStackEvents"),
            ("aws logs describe-log-groups --log-group-name-prefix /aws/lambda/ --log-group-class STANDARD --page-size 10 --max-items 20", "logs", "DescribeLogGroups"),
            ("aws logs get-query-results --query-id demo-query --next-token opaque== --max-items 100", "logs", "GetQueryResults"),
            ("aws logs filter-log-events --log-group-name /aws/lambda/demo --log-stream-names demo-a demo-b --start-time 0 --end-time 2000 --filter-pattern ERROR --page-size 50 --max-items 100", "logs", "FilterLogEvents"),
            ("aws logs get-log-events --log-group-name /aws/lambda/demo --log-stream-name demo-stream --limit 100 --start-from-head --next-token forward-fixture", "logs", "GetLogEvents"),
            ("aws logs describe-log-streams --log-group-name /aws/lambda/demo --order-by LastEventTime --descending --page-size 10", "logs", "DescribeLogStreams"),
            ("aws codepipeline list-pipelines --page-size 100 --max-items 20", "codepipeline", "ListPipelines"),
            ("aws codepipeline list-pipeline-executions --pipeline-name demo.pipeline --starting-token opaque --max-items 20", "codepipeline", "ListPipelineExecutions"),
            ("aws codepipeline list-action-executions --pipeline-name demo --no-paginate", "codepipeline", "ListActionExecutions"),
            ("aws codebuild batch-get-builds --ids demo:fixture-a demo:fixture-b", "codebuild", "BatchGetBuilds"),
            ("aws codeartifact list-packages --domain demo-domain --repository demo-repository --format pypi --package-prefix demo --publish ALLOW --upstream BLOCK --page-size 10", "codeartifact", "ListPackages"),
            ("aws codeartifact list-package-versions --domain demo-domain --repository demo-repository --format pypi --package demo-package --status Published --sort-by PUBLISHED_TIME --origin-type INTERNAL --max-items 10", "codeartifact", "ListPackageVersions"),
            ("aws codeartifact describe-package-version --domain demo-domain --repository demo-repository --format maven --package demo --package-version 1.0 --namespace org.example", "codeartifact", "DescribePackageVersion"),
            ("aws lambda list-functions --function-version ALL --master-region us-east-1 --page-size 50", "lambda", "ListFunctions"),
            ("aws resourcegroupstaggingapi get-resources --resource-type-filters ec2:instance s3 --page-size 10 --max-items 20", "resourcegroupstaggingapi", "GetResources"),
        ];
        assert_eq!(
            cases.len(),
            guard::APP_OPS
                .iter()
                .filter(|spec| spec.cli_command.is_some())
                .count()
        );
        for (command, service, operation) in cases {
            let dir = TestDir::new();
            let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
                stdout: br#"[{"fixture":"ok"}]"#.to_vec(),
                stderr: Vec::new(),
                success: true,
            }));
            let policy = Policy::parse(&policy::default_yaml()).map_err(|error| error.message);
            let mut ctx = fetch_context(&dir, process.clone(), policy);
            ctx.inputs = json!({"command": command});

            let parsed = parse_cli_command(command).unwrap();
            assert_eq!((&*parsed.service, &*parsed.operation), (service, operation));
            let result = fetch(&ctx).await;

            assert_eq!(result["render"], "table", "{command}: {result}");
            assert_eq!(result["action"], format!("{service}:{operation}"));
            assert_eq!(process.calls.load(Ordering::SeqCst), 1, "{command}");
            assert_eq!(process.requests.lock()[0].argv, parsed.argv);
        }
    }

    #[tokio::test]
    async fn wildcard_policy_cannot_bypass_cli_operation_and_argument_schemas() {
        let commands = [
            "aws ec2 describe-instances",
            "aws s3api list-objects-v2 --bucket demo-bucket",
            "aws cloudformation list-stack",
            "aws sts assume-role --role-arn demo --role-session-name demo",
            "aws sts get-session-token",
            "aws sts get-federation-token --name demo",
            "aws sso get-role-credentials --role-name demo --account-id acct-demo-fixture --access-token fixture",
            "aws logs start-query --log-group-name demo --query-string fields",
            "aws logs stop-query --query-id demo",
            "aws cloudformation list-stacks --profile demo",
            "aws cloudformation list-stacks --profile=demo",
            "aws cloudformation list-stacks --region eu-west-1",
            "aws cloudformation list-stacks --region=eu-west-1",
            "aws cloudformation list-stacks --output table",
            "aws cloudformation list-stacks --endpoint-url https://example.invalid",
            "aws cloudformation list-stacks --endpoint-url=demo",
            "aws cloudformation list-stacks --no-verify-ssl",
            "aws cloudformation list-stacks --ca-bundle demo",
            "aws cloudformation list-stacks --no-sign-request",
            "aws cloudformation list-stacks --debug",
            "aws cloudformation list-stacks --no-cli-pager",
            "aws cloudformation list-stacks --cli-auto-prompt",
            "aws cloudformation list-stacks --no-cli-auto-prompt",
            "aws cloudformation list-stacks --cli-binary-format raw-in-base64-out",
            "aws cloudformation list-stacks --cli-input-json '{}'",
            "aws cloudformation list-stacks --cli-input-yaml '{}'",
            "aws cloudformation list-stacks --generate-cli-skeleton",
            "aws cloudformation list-stacks --max-items 1 --max-items=2",
            "aws cloudformation list-stacks --query Account --query=Arn",
            "aws cloudformation list-stacks --max-item 2",
            "aws cloudformation list-stacks --que Account",
            "aws cloudformation list-stacks --unknown demo",
            "aws cloudformation list-stacks -- --profile demo",
            "aws cloudformation list-stacks --query file://fixture",
            "aws cloudformation list-stacks --query=FILEB://fixture",
            "aws cloudformation list-stacks --query 'outer={inner=file://fixture}'",
            "aws cloudformation list-stacks --query 'outer={inner@=fileb://fixture}'",
            "aws cloudformation list-stacks --query http://example.invalid",
            "aws cloudformation list-stacks --starting-token https://example.invalid",
            "aws codepipeline list-action-executions --pipeline-name demo --filter pipelineExecutionId=fixture",
            "aws resourcegroupstaggingapi get-resources --tag-filters Key=demo,Values=fixture",
            "aws codebuild batch-get-builds --ids '[\"demo-a\",\"demo-b\"]'",
            "aws logs filter-log-events --log-group-name demo --log-stream-names '[\"demo-a\",\"demo-b\"]'",
            "aws logs filter-log-events --log-group-name demo --unmask",
        ];
        for command in commands {
            let dir = TestDir::new();
            // A permissive fake response makes any unexpected spawn visible in
            // the explicit call-count assertion instead of masking the reason.
            let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
                stdout: b"[]".to_vec(),
                stderr: Vec::new(),
                success: true,
            }));
            let policy = Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n")
                .map_err(|error| error.message);
            let mut ctx = fetch_context(&dir, process.clone(), policy);
            ctx.inputs = json!({"command": command});

            let result = fetch(&ctx).await;

            assert_eq!(
                process.calls.load(Ordering::SeqCst),
                0,
                "{command}: {result}"
            );
            assert!(process.requests.lock().is_empty());
            assert!(
                result["ok"] == false || result["render"] == "permission_denied",
                "{command}: {result}"
            );
            assert!(!result.to_string().contains("add `-"));
        }
    }

    #[test]
    fn preserves_quoted_arguments() {
        let p = parse_cli_command(
            "aws logs filter-log-events --log-group-name /aws/lambda/demo --filter-pattern '{ $.level = \"ERROR\" }' --query 'events[].{message:message}'",
        )
        .unwrap();
        assert!(p.argv.contains(&"{ $.level = \"ERROR\" }".to_string()));
        assert!(p.argv.contains(&"events[].{message:message}".to_string()));
        let p = parse_cli_command(
            "aws logs get-log-events --log-group-name=demo --log-stream-name='[worker]' --limit=10",
        )
        .unwrap();
        assert!(p.argv.contains(&"[worker]".to_string()));
        assert!(p.argv.windows(2).any(|pair| pair == ["--limit", "10"]));
        let p = parse_cli_command("aws logs filter-log-events --log-group-name demo --log-stream-names env=demo worker=two").unwrap();
        assert!(p.argv.contains(&"env=demo".to_string()));
    }

    #[test]
    fn multi_dash_operations_require_an_exact_reviewed_mapping() {
        assert_eq!(
            parse_cli_command("aws codebuild batch-get-builds --ids x")
                .unwrap()
                .operation,
            "BatchGetBuilds"
        );
        assert!(parse_cli_command("aws s3api list-objects-v2 --bucket b").is_err());
    }

    #[test]
    fn rejects_commands_that_do_not_start_with_aws() {
        assert!(parse_cli_command("kubectl get pods").is_err());
        assert!(parse_cli_command("").is_err());
        assert!(parse_cli_command("aws").is_err());
        assert!(parse_cli_command("aws ec2").is_err());
    }

    #[test]
    fn rejects_identity_and_output_overrides() {
        assert!(parse_cli_command("aws cloudformation list-stacks --profile demo").is_err());
        assert!(parse_cli_command("aws cloudformation list-stacks --profile=demo").is_err());
        assert!(parse_cli_command("aws cloudformation list-stacks --output table").is_err());
        assert!(parse_cli_command("aws cloudformation list-stacks --output=table").is_err());
    }

    #[test]
    fn rejects_global_options_before_the_operation() {
        assert!(parse_cli_command("aws --region eu-west-1 cloudformation list-stacks").is_err());
        assert!(parse_cli_command("aws cloudformation --region eu-west-1 list-stacks").is_err());
    }

    #[test]
    fn rejects_malformed_tokens_and_quotes() {
        assert!(parse_cli_command("aws CloudFormation list-stacks").is_err());
        assert!(parse_cli_command("aws cloudformation List-Stacks").is_err());
        assert!(parse_cli_command("aws cloudformation 'list-stacks").is_err());
        assert!(parse_cli_command("aws cloudformation list-stacks\n--max-items 1").is_err());
    }

    #[test]
    fn rejects_invalid_required_values_relationships_and_pagination() {
        for command in [
            "aws codebuild batch-get-builds",
            "aws codebuild batch-get-builds --ids",
            "aws codeartifact list-packages --repository demo",
            "aws codeartifact list-packages --domain demo --repository demo --domain-owner 123",
            "aws codeartifact list-package-versions --domain demo --repository demo --format unknown --package demo",
            "aws codepipeline list-pipeline-executions --pipeline-name 'bad name'",
            "aws cloudformation list-stacks --stack-status-filter UNKNOWN",
            "aws cloudformation list-stacks --max-items 0",
            "aws cloudformation list-stacks --max-items 1001",
            "aws cloudformation list-stacks --max-items 18446744073709551616",
            "aws cloudformation list-stacks --page-size 10",
            "aws cloudformation list-stacks --no-paginate --max-items 1",
            "aws cloudformation describe-stack-resources",
            "aws cloudformation describe-stack-resources --stack-name demo --physical-resource-id demo",
            "aws logs describe-log-groups --log-group-name-prefix demo --log-group-name-pattern demo",
            "aws logs describe-log-groups --page-size 51",
            "aws logs get-query-results --query-id demo --max-items 10001",
            "aws logs get-query-results --query-id demo --starting-token opaque",
            "aws logs get-query-results --query-id demo --page-size 1",
            "aws logs get-log-events --log-group-name demo --log-stream-name demo --max-items 1",
            "aws logs get-log-events --log-group-name demo --log-stream-name demo --limit 0",
            "aws logs get-log-events --log-group-name demo --log-stream-name 'bad:*'",
            "aws logs get-log-events --log-group-name demo --log-stream-name demo --start-from-head=true",
            "aws logs get-log-events --log-group-name demo --log-stream-name demo --start-from-head --no-start-from-head",
            "aws logs filter-log-events --start-time 0",
            "aws logs filter-log-events --log-group-name demo --log-group-identifier demo",
            "aws logs filter-log-events --log-group-name demo --log-stream-names demo --log-stream-name-prefix demo",
            "aws logs filter-log-events --log-group-name demo --start-time -1",
            "aws logs filter-log-events --log-group-name demo --start-time 2 --end-time 1",
            "aws logs describe-log-streams --log-group-name demo --descending --no-descending",
            "aws logs describe-log-streams --log-group-name demo --order-by LastEventTime --log-stream-name-prefix demo",
            "aws lambda list-functions --master-region us-east-1",
            "aws codeartifact describe-package-version --domain demo --repository demo --format maven --package demo --package-version 1",
            "aws codeartifact describe-package-version --domain demo --repository demo --format pypi --package demo --package-version 1 --namespace demo",
        ] {
            assert!(parse_cli_command(command).is_err(), "accepted: {command}");
        }
        // GetQueryResults has a service --max-items field; it is not a CLI paginator.
        assert!(
            parse_cli_command("aws logs get-query-results --query-id demo --max-items 0").is_ok()
        );
        let oversized_query = format!("aws sts get-caller-identity --query '{}'", "x".repeat(4097));
        assert!(parse_cli_command(&oversized_query).is_err());
        let oversized_list = format!(
            "aws codebuild batch-get-builds --ids {}",
            vec!["demo"; 101].join(" ")
        );
        assert!(parse_cli_command(&oversized_list).is_err());
        let oversized_command = format!(
            "aws sts get-caller-identity --query '{}'",
            "x".repeat(MAX_COMMAND_BYTES)
        );
        assert!(parse_cli_command(&oversized_command).is_err());
    }

    #[test]
    fn array_of_objects_becomes_table_with_union_columns() {
        let v = json!([{"a": 1, "b": "x"}, {"b": "y", "c": true}]);
        let t = table_model(&v);
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a", "b", "c"]));
        assert_eq!(t["rows"][0]["a"], json!(1));
        assert_eq!(t["rows"][1]["c"], json!(true));
    }

    #[test]
    fn single_key_wrapper_object_is_unwrapped() {
        let t = table_model(&json!({"Reservations": [{"a": 1}]}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a"]));
    }

    #[test]
    fn wrapper_with_scalar_siblings_unwraps_the_single_array() {
        // NextToken-style pagination keys must not defeat the table mapping.
        let t = table_model(&json!({"Reservations": [{"a": 1}], "NextToken": "t"}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a"]));
    }

    #[test]
    fn array_of_scalars_becomes_single_value_column() {
        let t = table_model(&json!(["x", "y"]));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["value"]));
        assert_eq!(t["rows"][0]["value"], "x");
    }

    #[test]
    fn flat_object_becomes_key_value_rows() {
        let t = table_model(&json!({"UserId": "AIDX", "Account": "acct-fixture"}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["key", "value"]));
        assert_eq!(t["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn nested_cells_are_stringified_and_truncated() {
        let long = "x".repeat(300);
        let t = table_model(&json!([{"nested": {"deep": long}}]));
        let cell = t["rows"][0]["nested"].as_str().unwrap();
        assert!(cell.starts_with('{'));
        assert!(cell.chars().count() <= 121, "cell too long: {}", cell.len());
    }

    #[test]
    fn empty_array_is_an_empty_table_and_scalars_fall_back_to_raw_json() {
        let empty = table_model(&json!([]));
        assert_eq!(empty["render"], "table");
        assert_eq!(empty["rows"], json!([]));
        assert_eq!(table_model(&json!("just a string"))["render"], "raw_json");
        assert_eq!(table_model(&json!({}))["render"], "raw_json");
    }
}
