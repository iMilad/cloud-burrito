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

use super::{coverage::Coverage, WidgetCtx};

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
        Err(_) => {
            return json!({"ok": false, "error": "Only reviewed AWS read commands and arguments are supported"})
        }
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
        Err(error) if error == "AWS CLI request cancelled" => {
            return json!({"ok":false, "error_type":"Cancelled", "error":"AWS CLI request cancelled"});
        }
        Err(error) => {
            return json!({"ok": false, "error": safe_diagnostic(&error, &access.credentials)})
        }
    };
    if access.cancellation.is_cancelled() {
        return json!({"ok":false,"error_type":"Cancelled","error":"AWS CLI request cancelled"});
    }
    if !output.success {
        // Child diagnostics can contain request data, credentials or proxy URLs.
        // Do not echo them into the webview or audit log.
        return json!({
            "ok": false,
            "error": "AWS CLI command failed; check the selected account, permissions and inputs",
        });
    }
    if output.stderr.len() > 256 * 1024 {
        return json!({"ok":false,"error":"AWS CLI stderr exceeded its 256 KiB limit"});
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return json!({
            "ok": false,
            "error": "output larger than 2 MB — narrow it with --query or --max-items",
        });
    }
    let stdout = match std::str::from_utf8(&output.stdout) {
        Ok(text) => text,
        Err(_) => {
            return json!({"ok":false,"error_type":"CliInvalidOutput","error":"AWS CLI output was not valid UTF-8 JSON"})
        }
    };
    if contains_credentials(stdout, &access.credentials) {
        return json!({"ok": false, "error": "CLI output contained authentication material and was not displayed"});
    }
    let trimmed = stdout.trim();
    let mut out = if trimmed.is_empty() {
        cli_result_model(&json!([]), parsed)
    } else {
        match super::cli_json::decode(trimmed) {
            Ok(v) if value_contains_credentials(&v, &access.credentials) => {
                return json!({"ok": false, "error": "CLI output contained authentication material and was not displayed"});
            }
            Ok(v) => cli_result_model(&v, parsed),
            Err(super::cli_json::DecodeError::Budget) => {
                return json!({"ok":false,"error_type":"CliDecodeBudgetExceeded","error":"CLI JSON exceeds the local structure budget; narrow it with --query or --max-items"})
            }
            Err(super::cli_json::DecodeError::Invalid) => {
                return json!({"ok": false, "error": "AWS CLI output was not valid JSON"})
            }
        }
    };
    out["action"] = json!(action);
    out["account_id"] = json!(ctx.account_id);
    out["region"] = json!(ctx.region);
    if super::cli_json::encoded_size(&out, MAX_RESULT_BYTES).is_none() {
        return result_budget_error();
    }
    out
}

fn cli_result_model(value: &Value, parsed: &ParsedCli) -> Value {
    let (model, stats) = bounded_table_model(value);
    let clipped_cells = stats.clipped_cells;
    // CLI pagination is delegated to the executable. Its output is not proof
    // of complete AWS coverage, particularly after a JMESPath projection.
    let mut coverage = match model["rows"].as_array() {
        Some(rows) => Coverage::unknown(rows.len()),
        None => Coverage::unmeasured(),
    };
    coverage.unknown_reason("cli_scope", "CLI output reflects the selected command and filters; complete resource coverage is not established.");
    let has = |flag: &str| parsed.argv.iter().any(|argument| argument == flag);
    if has("--query") {
        coverage.unknown_reason(
            "cli_projection",
            "The selected query can omit results and pagination metadata.",
        );
    } else if parsed.service == "logs" && parsed.operation == "GetLogEvents" {
        coverage.unknown_reason("stream_window", "A log-stream token does not establish whether more events exist outside this response.");
    } else if ["NextToken", "nextToken", "PaginationToken"]
        .iter()
        .any(|key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty())
        })
    {
        coverage.has_more(Some(true));
        coverage.limited(
            "continuation",
            "The CLI returned a continuation marker; further results may exist.",
        );
    }
    if has("--no-paginate") {
        coverage.unknown_reason(
            "cli_single_page",
            "Automatic CLI pagination is disabled for this command.",
        );
    }
    for (flag, key) in [
        ("--max-items", "requested_items"),
        ("--limit", "requested_limit"),
        ("--max-results", "requested_results"),
    ] {
        if let Some(limit) = parsed
            .argv
            .windows(2)
            .find(|pair| pair[0] == flag)
            .and_then(|pair| pair[1].parse::<usize>().ok())
        {
            coverage.limit(key, Some(limit));
        }
    }
    if clipped_cells > 0 {
        coverage.count("clipped_cells", clipped_cells);
        coverage.limit("nested_cell_characters", Some(MAX_CELL_CHARS));
        coverage.limited(
            "cell_display_limit",
            "Long cells are shortened for display; open available cell details to inspect the full bounded value.",
        );
    }
    coverage.limit("rows", Some(MAX_ROWS));
    coverage.limit("columns", Some(MAX_COLUMNS));
    coverage.limit("column_name_characters", Some(MAX_COLUMN_CHARS));
    coverage.limit("string_cell_characters", Some(MAX_STRING_CHARS));
    coverage.limit("result_bytes", Some(MAX_RESULT_BYTES));
    coverage.limit("json_nodes", Some(super::cli_json::MAX_NODES));
    coverage.limit("json_depth", Some(super::cli_json::MAX_DEPTH));
    if stats.omitted_rows > 0 {
        coverage.count("omitted_rows", stats.omitted_rows);
        coverage.limited("row_limit","Some CLI output rows were omitted by the local row or display byte budget; narrow the command to inspect them.");
    }
    if stats.omitted_fields > 0 {
        coverage.count("omitted_fields", stats.omitted_fields);
        coverage.limited("column_limit","Some fields exceed the local column count or name limit; narrow the command to inspect them.");
    }
    if stats.full_values_withheld > 0 {
        coverage.count("full_values_withheld", stats.full_values_withheld);
        coverage.limited("full_value_limit","Some full cell values exceed the local detail budget; use --query to return a smaller value.");
    }
    if stats.byte_limited {
        coverage.limited("result_byte_limit","The local CLI display byte budget was reached; omitted output remains available only through a narrower command.");
    }
    coverage.attach(model)
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
    match text {
        "AWS CLI deadline reached" | "AWS CLI stdout exceeded its 2 MiB limit" | "AWS CLI stderr exceeded its 256 KiB limit"
        | "AWS CLI environment has conflicting runtime settings" | "AWS CLI certificate paths must be absolute"
        | "AWS CLI temporary directory is unavailable" | "AWS CLI temporary directory could not be created"
        | "AWS CLI temporary directory could not be allocated" | "AWS CLI process could not be started"
        | "AWS CLI output could not be read" | "AWS CLI exit status could not be read"
        | "AWS CLI output pipes could not be opened"
        | "AWS CLI v2 executable not found; use a native install or a supported absolute-Python wrapper" => text.into(),
        _ => "AWS CLI execution failed; check the local installation and selected context".into(),
    }
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
    StartingToken(&'static str),
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
        // These are botocore paginator input names, not CLI switch names.
        let token_key = match service {
            "cloudformation" => "NextToken",
            "logs" | "codepipeline" | "codeartifact" => "nextToken",
            "lambda" => "Marker",
            "resourcegroupstaggingapi" => "PaginationToken",
            _ => return None,
        };
        arguments.extend([
            optional("--starting-token", One(ValueType::StartingToken(token_key))),
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
        ValueType::StartingToken(key) => valid_starting_token(value, key),
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

/// Botocore merges decoded starting-token fields into request parameters.
/// Accept only the reviewed string/null cursor and its optional local offset;
/// legacy raw tokens, binary metadata and operation parameters are unsupported.
fn valid_starting_token(value: &str, token_key: &str) -> bool {
    if value.is_empty() || value.len() > 4096 {
        return false;
    }
    let Ok(bytes) = aws_smithy_types::base64::decode(value) else {
        return false;
    };
    if aws_smithy_types::base64::encode(&bytes) != value {
        return false;
    }
    let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    fields.get(token_key).is_some_and(|token| {
        token.is_null()
            || token
                .as_str()
                .is_some_and(|s| !s.is_empty() && !s.chars().any(char::is_control))
    }) && fields.iter().all(|(key, value)| {
        key == token_key
            || key == "boto_truncate_amount" && value.as_u64().is_some_and(|n| n <= i64::MAX as u64)
    })
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

/// Preserve the compact nested preview; full bounded values are explicit details.
const MAX_CELL_CHARS: usize = 120;
const MAX_STRING_CHARS: usize = 4096;
const MAX_FULL_CELL_BYTES: usize = 16 * 1024;
const MAX_ROWS: usize = 500;
const MAX_COLUMNS: usize = 32;
const MAX_COLUMN_CHARS: usize = 256;
const MAX_RESULT_BYTES: usize = 512 * 1024;
// Reserve headers, coverage, action and verified-context metadata separately.
const MAX_TABLE_DATA_BYTES: usize = MAX_RESULT_BYTES - 64 * 1024;

#[derive(Default)]
struct ModelStats {
    clipped_cells: usize,
    omitted_rows: usize,
    omitted_fields: usize,
    full_values_withheld: usize,
    byte_limited: bool,
}
struct TableBuilder {
    rows: Vec<Value>,
    details: Vec<Value>,
    bytes: usize,
    stats: ModelStats,
}
impl TableBuilder {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            details: Vec::new(),
            bytes: 0,
            stats: ModelStats::default(),
        }
    }
    fn push(&mut self, row: Value, details: Vec<Value>, row_stats: ModelStats) -> bool {
        let Some(bytes) = super::cli_json::encoded_size(
            &(&row, &details),
            MAX_TABLE_DATA_BYTES.saturating_sub(self.bytes),
        ) else {
            self.stats.byte_limited = true;
            return false;
        };
        self.stats.clipped_cells += row_stats.clipped_cells;
        self.stats.omitted_fields += row_stats.omitted_fields;
        self.stats.full_values_withheld += row_stats.full_values_withheld;
        self.bytes += bytes;
        self.rows.push(row);
        self.details.extend(details);
        true
    }
    fn finish(mut self, columns: Vec<String>, source_rows: usize) -> (Value, ModelStats) {
        self.stats.omitted_rows = source_rows.saturating_sub(self.rows.len());
        let mut model = json!({"render":"table","columns":columns,"rows":self.rows});
        if !self.details.is_empty() {
            model["cell_details"] = Value::Array(self.details);
        }
        (model, self.stats)
    }
}

#[cfg(test)]
fn table_model(value: &Value) -> (Value, usize) {
    let (model, stats) = bounded_table_model(value);
    (model, stats.clipped_cells)
}

fn bounded_table_model(value: &Value) -> (Value, ModelStats) {
    match value {
        Value::Array(items) => array_table(items).unwrap_or_else(|| raw_json(value)),
        Value::Object(map) => {
            let mut arrays = map.values().filter_map(Value::as_array);
            let first = arrays.next();
            if arrays.next().is_none() && map.values().all(|v| v.is_array() || is_scalar(v)) {
                if let Some(items) = first {
                    return array_table(items).unwrap_or_else(|| raw_json(value));
                }
            }
            if !map.is_empty() && map.values().all(is_scalar) {
                let mut table = TableBuilder::new();
                for (key, value) in map.iter().take(MAX_ROWS) {
                    let index = table.rows.len();
                    let mut details = Vec::new();
                    let mut row_stats = ModelStats::default();
                    let row = json!({"key":cell(&Value::String(key.clone()),index,"key",&mut row_stats,&mut details),
                        "value":cell(value,index,"value",&mut row_stats,&mut details)});
                    if !table.push(row, details, row_stats) {
                        break;
                    }
                }
                return table.finish(vec!["key".into(), "value".into()], map.len());
            }
            raw_json(value)
        }
        _ => raw_json(value),
    }
}

fn array_table(items: &[Value]) -> Option<(Value, ModelStats)> {
    if items.iter().all(Value::is_object) {
        let mut table = TableBuilder::new();
        let mut columns = Vec::new();
        for item in items.iter().take(MAX_ROWS) {
            let index = table.rows.len();
            let mut row = serde_json::Map::new();
            let mut details = Vec::new();
            let mut row_stats = ModelStats::default();
            for (key, value) in item.as_object().expect("checked object") {
                if key.chars().count() > MAX_COLUMN_CHARS
                    || (!columns.contains(key) && columns.len() == MAX_COLUMNS)
                {
                    row_stats.omitted_fields += 1;
                    continue;
                }
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
                row.insert(
                    key.clone(),
                    cell(value, index, key, &mut row_stats, &mut details),
                );
            }
            if !table.push(Value::Object(row), details, row_stats) {
                break;
            }
        }
        return Some(table.finish(columns, items.len()));
    }
    if items.iter().all(is_scalar) {
        let mut table = TableBuilder::new();
        for value in items.iter().take(MAX_ROWS) {
            let index = table.rows.len();
            let mut details = Vec::new();
            let mut row_stats = ModelStats::default();
            let row = json!({"value":cell(value,index,"value",&mut row_stats,&mut details)});
            if !table.push(row, details, row_stats) {
                break;
            }
        }
        return Some(table.finish(vec!["value".into()], items.len()));
    }
    None
}

fn is_scalar(value: &Value) -> bool {
    !value.is_array() && !value.is_object()
}
fn shortened(text: &str, limit: usize) -> String {
    text.chars()
        .take(limit.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// Bounded byte prefix prevents serializing a large nested value twice in full.
struct JsonPrefix(Vec<u8>);
impl std::io::Write for JsonPrefix {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = MAX_FULL_CELL_BYTES.saturating_sub(self.0.len());
        if bytes.len() > remaining {
            self.0.extend_from_slice(&bytes[..remaining]);
            return Err(std::io::Error::other("CLI cell preview limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn cell(
    value: &Value,
    row: usize,
    column: &str,
    stats: &mut ModelStats,
    details: &mut Vec<Value>,
) -> Value {
    if let Some(text) = value.as_str() {
        if text.chars().count() > MAX_STRING_CHARS {
            stats.clipped_cells += 1;
            stats.full_values_withheld += 1;
            details.push(json!({"row":row,"column":column,"unavailable":true}));
            return json!(shortened(text, MAX_STRING_CHARS));
        }
        return value.clone();
    }
    if is_scalar(value) {
        return value.clone();
    }
    let mut prefix = JsonPrefix(Vec::new());
    let complete = serde_json::to_writer(&mut prefix, value).is_ok();
    if let Err(error) = std::str::from_utf8(&prefix.0) {
        prefix.0.truncate(error.valid_up_to());
    }
    let text = String::from_utf8(prefix.0).expect("trimmed valid UTF-8 prefix");
    let characters = text.chars().count();
    if !complete || characters > MAX_CELL_CHARS {
        stats.clipped_cells += 1;
        if complete && characters <= MAX_STRING_CHARS {
            details.push(json!({"row":row,"column":column,"value":value}));
        } else {
            stats.full_values_withheld += 1;
            details.push(json!({"row":row,"column":column,"unavailable":true}));
        }
        return json!(shortened(&text, MAX_CELL_CHARS));
    }
    json!(text)
}

fn raw_json(value: &Value) -> (Value, ModelStats) {
    if super::cli_json::encoded_size(value, MAX_TABLE_DATA_BYTES).is_none() {
        return (
            result_budget_error(),
            ModelStats {
                byte_limited: true,
                ..ModelStats::default()
            },
        );
    }
    // A raw response has one complete value; it never duplicates the table data.
    (
        json!({"render":"raw_json","data":value}),
        ModelStats::default(),
    )
}
fn result_budget_error() -> Value {
    json!({"ok":false,"error_type":"CliResultBudgetExceeded","error":"CLI result exceeds the local display budget; narrow the command with --query or --max-items. No truncated JSON was displayed."})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        policy::load(&dir.paths()).unwrap();
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
    async fn cli_continuation_and_clipped_cells_remain_visible_without_failing_the_read() {
        let directory = TestDir::new();
        let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
            stdout: serde_json::to_vec(&json!({"StackSummaries":[{
                "StackName":"synthetic-stack", "Nested":{"description":"x".repeat(300)}
            }], "NextToken":"synthetic-unvisited-continuation"}))
            .unwrap(),
            stderr: Vec::new(),
            success: true,
        }));
        let mut ctx = fetch_context(&directory, process.clone(), allowed_fetch_policy());
        ctx.inputs = json!({"command":"aws cloudformation list-stacks --max-items 1"});
        let response = fetch(&ctx).await;
        assert_eq!(response["rows"][0]["StackName"], "synthetic-stack");
        assert_eq!(response["coverage"]["has_more"], true);
        assert_eq!(response["coverage"]["completeness"], "limited");
        assert_eq!(response["coverage"]["counts"]["clipped_cells"], 1);
        assert_eq!(
            response["coverage"]["limits"]["nested_cell_characters"],
            MAX_CELL_CHARS
        );
        assert_eq!(response["coverage"]["limits"]["requested_items"], 1);
        assert_eq!(response["truncated"], true);
        assert_eq!(crate::request::outcome(&response), "succeeded");
        assert!(!response
            .to_string()
            .contains("synthetic-unvisited-continuation"));
        assert_eq!(process.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn projected_cli_output_does_not_claim_empty_or_token_fields_prove_completeness() {
        for value in [
            json!([]),
            json!({"Items":[], "NextToken":"synthetic-projected-value"}),
        ] {
            let directory = TestDir::new();
            let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
                stdout: serde_json::to_vec(&value).unwrap(),
                stderr: Vec::new(),
                success: true,
            }));
            let mut ctx = fetch_context(&directory, process.clone(), allowed_fetch_policy());
            ctx.inputs = json!({"command":"aws cloudformation list-stacks --query StackSummaries"});
            let response = fetch(&ctx).await;
            assert!(response["rows"].as_array().unwrap().is_empty());
            assert_eq!(response["coverage"]["completeness"], "unknown");
            assert!(response["coverage"]["has_more"].is_null());
            assert_eq!(crate::request::outcome(&response), "succeeded");
            assert_eq!(process.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn single_cli_log_window_and_raw_json_do_not_invent_row_or_page_coverage() {
        let directory = TestDir::new();
        let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
            stdout: serde_json::to_vec(
                &json!({"events":[],"nextForwardToken":"synthetic-boundary"}),
            )
            .unwrap(),
            stderr: Vec::new(),
            success: true,
        }));
        let policy = Policy::parse("statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials, logs:GetLogEvents]\n")
            .map_err(|error| error.message);
        let mut ctx = fetch_context(&directory, process.clone(), policy);
        ctx.inputs = json!({"command":"aws logs get-log-events --log-group-name synthetic-group --log-stream-name synthetic-stream --limit 10"});
        let response = fetch(&ctx).await;
        assert_eq!(response["coverage"]["completeness"], "unknown");
        assert!(response["coverage"]["has_more"].is_null());
        assert_eq!(response["coverage"]["limits"]["requested_limit"], 10);
        assert!(response["coverage"]["counts"].get("pages").is_none());
        assert_eq!(process.calls.load(Ordering::SeqCst), 1);

        let parsed =
            parse_cli_command("aws cloudformation list-stacks --query length(StackSummaries)")
                .unwrap();
        let response = cli_result_model(&json!(7), &parsed);
        assert_eq!(response["render"], "raw_json");
        assert_eq!(response["data"], 7);
        assert!(response["coverage"]["counts"].get("returned").is_none());
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
                Err("AWS CLI deadline reached".into()),
                "AWS CLI deadline reached",
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

    fn starting_token(value: Value) -> String {
        aws_smithy_types::base64::encode(serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn pagination_accepts_service_cursors_and_cli_truncation_offsets() {
        for (command, key) in [
            ("cloudformation list-stacks", "NextToken"),
            (
                "cloudformation describe-stack-events --stack-name demo",
                "NextToken",
            ),
            ("logs describe-log-groups", "nextToken"),
            ("logs filter-log-events --log-group-name demo", "nextToken"),
            (
                "logs describe-log-streams --log-group-name demo",
                "nextToken",
            ),
            ("codepipeline list-pipelines", "nextToken"),
            (
                "codeartifact list-packages --domain demo --repository demo",
                "nextToken",
            ),
            ("lambda list-functions", "Marker"),
            ("resourcegroupstaggingapi get-resources", "PaginationToken"),
        ] {
            for cursor in [Value::Null, json!("synthetic-cursor==")] {
                for offset in [None, Some(0), Some(20)] {
                    let mut token = json!({key:cursor});
                    if let Some(offset) = offset {
                        token["boto_truncate_amount"] = json!(offset);
                    }
                    let token = starting_token(token);
                    let parsed = parse_cli_command(&format!(
                        "aws {command} --starting-token {token} --max-items 25"
                    ))
                    .unwrap();
                    assert!(parsed
                        .argv
                        .windows(2)
                        .any(|pair| pair[0] == "--starting-token" && pair[1] == token));
                }
            }
        }
    }

    #[tokio::test]
    async fn pagination_cannot_inject_request_parameters_even_with_wildcard_policy() {
        for (command, payload) in [
            (
                "logs filter-log-events --log-group-name demo",
                json!({"nextToken":null,"unmask":true}),
            ),
            (
                "logs describe-log-groups",
                json!({"nextToken":null,"includeLinkedAccounts":true}),
            ),
            (
                "logs describe-log-groups",
                json!({"nextToken":null,"accountIdentifiers":["synthetic-account"]}),
            ),
            (
                "cloudformation list-stacks",
                json!({"NextToken":null,"StackStatusFilter":["DELETE_COMPLETE"]}),
            ),
            (
                "codepipeline list-action-executions --pipeline-name demo",
                json!({"nextToken":null,"filter":{"pipelineExecutionId":"synthetic-execution"}}),
            ),
            (
                "logs filter-log-events --log-group-name demo",
                json!({"nextToken":null,"logGroupName":null}),
            ),
            (
                "logs describe-log-groups",
                json!({"nextToken":null,"boto_encoded_keys":[]}),
            ),
        ] {
            let dir = TestDir::new();
            let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
                stdout: b"[]".to_vec(),
                stderr: Vec::new(),
                success: true,
            }));
            let policy = Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n")
                .map_err(|error| error.message);
            let mut ctx = fetch_context(&dir, process.clone(), policy);
            ctx.inputs = json!({"command":format!("aws {command} --starting-token {}", starting_token(payload))});
            let result = fetch(&ctx).await;
            assert_eq!(result["ok"], false, "{command}: {result}");
            assert_eq!(process.calls.load(Ordering::SeqCst), 0);
            assert!(process.requests.lock().is_empty());
        }
    }

    #[test]
    fn pagination_rejects_malformed_tokens_and_wrong_cursor_shapes() {
        let mut tokens = vec!["opaque".into(), "e30".into(), "{}".into(), "[1]".into()];
        for payload in [
            json!([]),
            json!({}),
            json!({"NextToken":"wrong-service"}),
            json!({"nextToken":{}}),
            json!({"nextToken":[]}),
            json!({"nextToken":42}),
            json!({"nextToken":""}),
            json!({"nextToken":"line\nbreak"}),
            json!({"nextToken":null,"boto_truncate_amount":-1}),
            json!({"nextToken":null,"boto_truncate_amount":1.5}),
            json!({"nextToken":null,"boto_truncate_amount":"1"}),
            json!({"nextToken":null,"boto_truncate_amount":null}),
            json!({"nextToken":null,"boto_truncate_amount":u64::MAX}),
            json!({"nextToken":"x".repeat(4096)}),
        ] {
            tokens.push(starting_token(payload));
        }
        tokens.push(aws_smithy_types::base64::encode([0xff]));
        for token in tokens {
            assert!(parse_cli_command(&format!(
                "aws logs describe-log-groups --starting-token '{token}'"
            ))
            .is_err());
        }
    }

    #[tokio::test]
    async fn every_reviewed_schema_reaches_fake_process_with_default_policy() {
        let cfn_resume = format!(
            "aws cloudformation list-stacks --stack-status-filter CREATE_COMPLETE UPDATE_COMPLETE --max-items 20 --starting-token {}",
            starting_token(json!({"NextToken":"synthetic-cursor"}))
        );
        let pipeline_resume = format!(
            "aws codepipeline list-pipeline-executions --pipeline-name demo.pipeline --max-items 20 --starting-token {}",
            starting_token(json!({"nextToken":null,"boto_truncate_amount":20}))
        );
        let cases = [
            ("aws sts get-caller-identity --query Account", "sts", "GetCallerIdentity"),
            (cfn_resume.as_str(), "cloudformation", "ListStacks"),
            ("aws cloudformation describe-stack-resources --stack-name demo-stack --logical-resource-id DemoFunction", "cloudformation", "DescribeStackResources"),
            ("aws cloudformation describe-stack-events --stack-name demo-stack --no-paginate", "cloudformation", "DescribeStackEvents"),
            ("aws logs describe-log-groups --log-group-name-prefix /aws/lambda/ --log-group-class STANDARD --page-size 10 --max-items 20", "logs", "DescribeLogGroups"),
            ("aws logs get-query-results --query-id demo-query --next-token opaque== --max-items 100", "logs", "GetQueryResults"),
            ("aws logs filter-log-events --log-group-name /aws/lambda/demo --log-stream-names demo-a demo-b --start-time 0 --end-time 2000 --filter-pattern ERROR --page-size 50 --max-items 100", "logs", "FilterLogEvents"),
            ("aws logs get-log-events --log-group-name /aws/lambda/demo --log-stream-name demo-stream --limit 100 --start-from-head --next-token forward-fixture", "logs", "GetLogEvents"),
            ("aws logs describe-log-streams --log-group-name /aws/lambda/demo --order-by LastEventTime --descending --page-size 10", "logs", "DescribeLogStreams"),
            ("aws codepipeline list-pipelines --page-size 100 --max-items 20", "codepipeline", "ListPipelines"),
            (pipeline_resume.as_str(), "codepipeline", "ListPipelineExecutions"),
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
        let (t, _) = table_model(&v);
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a", "b", "c"]));
        assert_eq!(t["rows"][0]["a"], json!(1));
        assert_eq!(t["rows"][1]["c"], json!(true));
    }

    #[test]
    fn single_key_wrapper_object_is_unwrapped() {
        let (t, _) = table_model(&json!({"Reservations": [{"a": 1}]}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a"]));
    }

    #[test]
    fn wrapper_with_scalar_siblings_unwraps_the_single_array() {
        // NextToken-style pagination keys must not defeat the table mapping.
        let (t, _) = table_model(&json!({"Reservations": [{"a": 1}], "NextToken": "t"}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["a"]));
    }

    #[test]
    fn array_of_scalars_becomes_single_value_column() {
        let (t, _) = table_model(&json!(["x", "y"]));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["value"]));
        assert_eq!(t["rows"][0]["value"], "x");
    }

    #[test]
    fn flat_object_becomes_key_value_rows() {
        let (t, _) = table_model(&json!({"UserId": "AIDX", "Account": "acct-fixture"}));
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["key", "value"]));
        assert_eq!(t["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn nested_cells_are_stringified_and_truncated() {
        let long = "x".repeat(300);
        let (t, _) = table_model(&json!([{"nested": {"deep": long}}]));
        let cell = t["rows"][0]["nested"].as_str().unwrap();
        assert!(cell.starts_with('{'));
        assert!(cell.chars().count() <= 121, "cell too long: {}", cell.len());
    }

    #[test]
    fn empty_array_is_an_empty_table_and_scalars_fall_back_to_raw_json() {
        let (empty, _) = table_model(&json!([]));
        assert_eq!(empty["render"], "table");
        assert_eq!(empty["rows"], json!([]));
        assert_eq!(table_model(&json!("just a string")).0["render"], "raw_json");
        assert_eq!(table_model(&json!({})).0["render"], "raw_json");
    }
    #[test]
    fn table_budgets_report_omitted_rows_columns_and_whole_result_bytes() {
        let parsed = parse_cli_command("aws cloudformation list-stacks").unwrap();
        let wide = json!((0..600)
            .map(|_| (0..40)
                .map(|i| (format!("field-{i:02}"), json!(i)))
                .collect::<serde_json::Map<_, _>>())
            .collect::<Vec<_>>());
        let result = cli_result_model(&wide, &parsed);
        assert_eq!(result["columns"].as_array().unwrap().len(), 32);
        assert_eq!(result["rows"].as_array().unwrap().len(), 500);
        assert_eq!(result["coverage"]["counts"]["omitted_rows"], 100);
        assert_eq!(result["coverage"]["counts"]["omitted_fields"], 4000);
        assert_eq!(result["coverage"]["completeness"], "limited");
        assert!(super::super::cli_json::encoded_size(&result, MAX_RESULT_BYTES).is_some());

        let large = json!((0..1000)
            .map(|_| json!({"description":"x".repeat(2000)}))
            .collect::<Vec<_>>());
        let result = cli_result_model(&large, &parsed);
        assert!(result["rows"].as_array().unwrap().len() < 500);
        assert!(
            result["coverage"]["counts"]["omitted_rows"]
                .as_u64()
                .unwrap()
                > 500
        );
        assert!(super::super::cli_json::encoded_size(&result, MAX_RESULT_BYTES).is_some());
        assert!(result["coverage"].to_string().contains("result_byte_limit"));
        assert!(result.get("data").is_none());
    }

    #[test]
    fn full_bounded_cell_details_are_separate_from_previews_and_large_values_are_explicit() {
        let parsed = parse_cli_command("aws cloudformation list-stacks").unwrap();
        let source = json!([{"nested":{"description":"x".repeat(300)},"long":"🦀".repeat(5000)}]);
        let result = cli_result_model(&source, &parsed);
        assert!(
            result["rows"][0]["nested"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= MAX_CELL_CHARS
        );
        assert_eq!(
            result["rows"][0]["long"].as_str().unwrap().chars().count(),
            MAX_STRING_CHARS
        );
        let details = result["cell_details"].as_array().unwrap();
        let nested = details
            .iter()
            .find(|detail| detail["column"] == "nested")
            .unwrap();
        assert_eq!(nested["row"], 0);
        assert_eq!(nested["value"], source[0]["nested"]);
        let unavailable = details
            .iter()
            .find(|detail| detail["column"] == "long")
            .unwrap();
        assert_eq!(unavailable["unavailable"], true);
        assert!(unavailable.get("value").is_none());
        assert_eq!(result["coverage"]["counts"]["full_values_withheld"], 1);
    }

    #[tokio::test]
    async fn invalid_utf8_structural_overflow_and_large_raw_values_never_become_successful_json() {
        let dir = TestDir::new();
        let cases = vec![
            (vec![b'"', 0xff, b'"'], "CliInvalidOutput"),
            (
                format!(
                    "[{}]",
                    vec!["0"; super::super::cli_json::MAX_NODES].join(",")
                )
                .into_bytes(),
                "CliDecodeBudgetExceeded",
            ),
            (
                format!("{}0{}", "[".repeat(33), "]".repeat(33)).into_bytes(),
                "CliDecodeBudgetExceeded",
            ),
            (
                serde_json::to_vec(&json!([{"nested":"x".repeat(MAX_RESULT_BYTES)},true])).unwrap(),
                "CliResultBudgetExceeded",
            ),
        ];
        for (stdout, expected) in cases {
            let process = FakeProcessRunner::with_response(Ok(ProcessOutput {
                stdout,
                stderr: Vec::new(),
                success: true,
            }));
            let ctx = fetch_context(&dir, process.clone(), allowed_fetch_policy());
            let result = fetch(&ctx).await;
            assert_eq!(result["ok"], false);
            assert_eq!(result["error_type"], expected);
            assert!(result.get("data").is_none());
            assert!(result.get("rows").is_none());
            assert_eq!(process.calls.load(Ordering::SeqCst), 1);
        }
    }

    struct BudgetProcessRunner {
        calls: AtomicUsize,
        active: Arc<AtomicUsize>,
        peak: AtomicUsize,
        released: AtomicBool,
        release: tokio::sync::Notify,
    }
    struct BudgetActive(Arc<AtomicUsize>);
    impl Drop for BudgetActive {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl ProcessRunner for BudgetProcessRunner {
        fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
            Box::pin(async move {
                let index = self.calls.fetch_add(1, Ordering::SeqCst);
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                let _active = BudgetActive(self.active.clone());
                // Hold admitted fake children until the test has dispatched
                // cancellation, independently of filesystem policy-read timing.
                while !self.released.load(Ordering::SeqCst) {
                    let release = self.release.notified();
                    tokio::pin!(release);
                    release.as_mut().enable();
                    if self.released.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::select! {
                        biased;
                        _ = request.cancellation.cancelled() => return Err("AWS CLI request cancelled".into()),
                        _ = &mut release => {},
                    }
                }
                tokio::select! {
                    _=request.cancellation.cancelled()=>return Err("AWS CLI request cancelled".into()),
                    _=tokio::time::sleep(std::time::Duration::from_millis(1))=>{},
                }
                let stdout = match index % 5 {
                    0 => vec![b'x'; MAX_OUTPUT_BYTES + 1],
                    1 => b"[invalid".to_vec(),
                    2 => vec![b'"', 0xff, b'"'],
                    _ => br#"{"StackSummaries":[{"StackName":"synthetic-stack"}]}"#.to_vec(),
                };
                Ok(ProcessOutput {
                    stdout,
                    stderr: Vec::new(),
                    success: true,
                })
            })
        }
    }

    #[tokio::test]
    async fn fifty_fake_jobs_recover_cli_permits_after_overflow_parse_failure_and_cancellation() {
        let dir = TestDir::new();
        let process = Arc::new(BudgetProcessRunner {
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            peak: AtomicUsize::new(0),
            released: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        let empty = FakeProcessRunner::with_response(Ok(ProcessOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            success: true,
        }));
        let mut template = fetch_context(&dir, empty, allowed_fetch_policy());
        template.runtime.process = process.clone();
        for _ in 0..10 {
            process.released.store(false, Ordering::SeqCst);
            let round_start = process.calls.load(Ordering::SeqCst);
            let contexts: Vec<_> = (0..50)
                .map(|_| {
                    let mut ctx = template.clone();
                    ctx.cli.as_mut().unwrap().cancellation =
                        crate::process::ProcessCancellation::new();
                    ctx
                })
                .collect();
            let mut jobs: Vec<_> = contexts
                .iter()
                .map(|ctx| Box::pin(crate::widgets::fetch("aws-cli", ctx)))
                .collect();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    for job in &mut jobs {
                        assert!(futures::poll!(job.as_mut()).is_pending());
                    }
                    if process.calls.load(Ordering::SeqCst) == round_start + 2 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("two fake children should reach the held runner");
            assert_eq!(process.active.load(Ordering::SeqCst), 2);
            assert_eq!(template.runtime.scheduler.snapshot().cli, 2);
            assert_eq!(template.runtime.scheduler.snapshot().queued, 48);
            let before = process.calls.load(Ordering::SeqCst);
            for index in (10..50).step_by(2) {
                contexts[index].cli.as_ref().unwrap().cancellation.cancel();
            }
            contexts[0].cli.as_ref().unwrap().cancellation.cancel();
            process.released.store(true, Ordering::SeqCst);
            process.release.notify_waiters();
            let results = futures::future::join_all(jobs).await;
            assert!(results.iter().any(|result| result["ok"] == false));
            assert!(results
                .iter()
                .all(
                    |result| super::super::cli_json::encoded_size(result, MAX_RESULT_BYTES)
                        .is_some()
                ));
            assert_eq!(process.calls.load(Ordering::SeqCst) - before, 28);
            assert_eq!(process.active.load(Ordering::SeqCst), 0);
            assert_eq!(template.runtime.scheduler.snapshot().cli, 0);
            assert_eq!(template.runtime.scheduler.snapshot().queued, 0);
        }
        assert_eq!(process.peak.load(Ordering::SeqCst), 2);
    }
}
