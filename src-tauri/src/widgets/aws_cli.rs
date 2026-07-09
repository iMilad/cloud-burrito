//! AWS CLI Table — run a user-supplied *read-only* `aws` command and render
//! its JSON output as a table.
//!
//! This widget deliberately steps outside the compiled-in SDK surface: the
//! command is tokenized (never a shell), its `service operation` pair is
//! derived (`ec2 describe-instances` -> `ec2:DescribeInstances`) and gated by
//! the structural read-only guard plus policy.yaml (`policy::gate_cli`), and
//! only then is the `aws` binary spawned with the parsed argv. Identity is
//! always the widget's resolved context: the child gets AWS_PROFILE/AWS_REGION
//! from the app, and `--profile` in the command is rejected.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;

use super::WidgetCtx;

/// Hard kill for hung commands.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
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
    if let Some(mut denied) = ctx.preflight_cli(&parsed.service, &parsed.operation) {
        // Policy denials (not guard/credential ones) get a self-serve hint.
        let is_this_action = denied["action"] == json!(action);
        let reason = denied["reason"].as_str().unwrap_or("").to_string();
        if is_this_action && reason.contains("policy") {
            denied["reason"] = json!(format!(
                "{reason} — add `- {action}` (or a glob like `- {}:Describe*`) to policy.yaml in Settings",
                parsed.service
            ));
        }
        return denied;
    }
    run_cli(ctx, &parsed, &action).await
}

async fn run_cli(ctx: &WidgetCtx, parsed: &ParsedCli, action: &str) -> Value {
    let Some(bin) = aws_binary() else {
        return json!({
            "ok": false,
            "error": "aws CLI not found — install AWS CLI v2 or add it to PATH",
        });
    };
    let mut cmd = Command::new(&bin);
    cmd.args(&parsed.argv)
        .env("AWS_PROFILE", &ctx.profile)
        .env("AWS_REGION", &ctx.region)
        .env("AWS_DEFAULT_REGION", &ctx.region)
        .env("AWS_PAGER", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("could not run {bin}: {e}")}),
    };
    let output = match tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return json!({"ok": false, "error": format!("command failed to run: {e}")}),
        Err(_) => return json!({"ok": false, "error": "command timed out after 30s (killed)"}),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return json!({
            "ok": false,
            "error": format!("aws exited with {}: {}", output.status, tail_chars(stderr.trim(), 400)),
        });
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return json!({
            "ok": false,
            "error": "output larger than 2 MB — narrow it with --query or --max-items",
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    let mut out = if trimmed.is_empty() {
        json!({"render": "table", "columns": [], "rows": []})
    } else {
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => table_model(&v),
            Err(e) => return json!({"ok": false, "error": format!("output was not JSON: {e}")}),
        }
    };
    out["action"] = json!(action);
    out["account_id"] = json!(ctx.account_id);
    out["region"] = json!(ctx.region);
    out
}

/// Last `n` chars of a string (stderr can be pages of stack trace).
fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    format!("…{}", s.chars().skip(count - n).collect::<String>())
}

/// PATH first; GUI-launched macOS apps often miss Homebrew's dirs, so fall
/// back to the usual install locations.
fn aws_binary() -> Option<String> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("aws").is_file() {
                return Some("aws".to_string());
            }
        }
    }
    ["/opt/homebrew/bin/aws", "/usr/local/bin/aws"]
        .iter()
        .find(|c| Path::new(c).is_file())
        .map(|c| c.to_string())
}

/// A validated, ready-to-spawn CLI command.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedCli {
    /// CLI service segment, e.g. "ec2".
    pub service: String,
    /// PascalCase operation for gating/auditing, e.g. "DescribeInstances".
    pub operation: String,
    /// argv after the `aws` binary, with `--output json` appended.
    pub argv: Vec<String>,
}

/// Tokenize and validate a user command. Shape must be
/// `aws <service> <operation> [args...]`; identity/output overrides are
/// rejected; quoting follows shell rules but no shell ever runs.
pub fn parse_cli_command(command: &str) -> Result<ParsedCli, String> {
    let tokens = shell_words::split(command.trim())
        .map_err(|e| format!("could not parse command: {e}"))?;
    if tokens.first().map(String::as_str) != Some("aws") {
        return Err("command must start with `aws`".to_string());
    }
    if tokens.len() < 3 {
        return Err("expected `aws <service> <operation> [args...]`".to_string());
    }
    let service = tokens[1].clone();
    let op_kebab = tokens[2].clone();
    if !is_kebab_token(&service) {
        return Err(format!(
            "`{service}` is not a service name — global options go after the operation"
        ));
    }
    if !is_kebab_token(&op_kebab) {
        return Err(format!(
            "`{op_kebab}` is not an operation name — global options go after the operation"
        ));
    }
    for t in &tokens[3..] {
        if t == "--profile" || t.starts_with("--profile=") {
            return Err(
                "--profile is not allowed — the widget always runs under the tile's account context"
                    .to_string(),
            );
        }
        if t == "--output" || t.starts_with("--output=") {
            return Err("--output is not allowed — output is always json".to_string());
        }
    }
    let operation = kebab_to_pascal(&op_kebab);
    let mut argv: Vec<String> = tokens[1..].to_vec();
    argv.push("--output".to_string());
    argv.push("json".to_string());
    Ok(ParsedCli { service, operation, argv })
}

/// Lowercase kebab-case, as CLI service/operation names are: `s3api`,
/// `list-objects-v2`. Anything starting with `-` is an option, not a name.
fn is_kebab_token(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `describe-instances` -> `DescribeInstances` (the SDK/IAM operation name).
fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
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
            let array_keys: Vec<&String> =
                map.iter().filter(|(_, v)| v.is_array()).map(|(k, _)| k).collect();
            if array_keys.len() == 1 && map.values().all(|v| v.is_array() || is_scalar(v)) {
                let items = map[array_keys[0]].as_array().expect("filtered on is_array");
                return array_table(items).unwrap_or_else(|| raw_json(value));
            }
            // Flat object of scalars (sts get-caller-identity) -> key/value.
            if !map.is_empty() && map.values().all(is_scalar) {
                let rows: Vec<Value> =
                    map.iter().map(|(k, v)| json!({"key": k, "value": cell(v)})).collect();
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
    use serde_json::json;

    #[test]
    fn parses_basic_read_only_command() {
        let p = parse_cli_command("aws ec2 describe-instances").unwrap();
        assert_eq!(p.service, "ec2");
        assert_eq!(p.operation, "DescribeInstances");
        assert_eq!(p.argv, vec!["ec2", "describe-instances", "--output", "json"]);
    }

    #[test]
    fn preserves_quoted_arguments() {
        let p = parse_cli_command(
            "aws logs filter-log-events --log-group-name \"/aws/lambda/my fn\" --filter-pattern 'ERROR'",
        )
        .unwrap();
        assert!(p.argv.contains(&"/aws/lambda/my fn".to_string()));
        assert!(p.argv.contains(&"ERROR".to_string()));
        // region flags after the operation are the user's business
        let q = parse_cli_command("aws ec2 describe-instances --region eu-west-1").unwrap();
        assert!(q.argv.contains(&"--region".to_string()));
    }

    #[test]
    fn multi_dash_operations_map_to_pascal_case() {
        assert_eq!(
            parse_cli_command("aws codebuild batch-get-builds --ids x").unwrap().operation,
            "BatchGetBuilds"
        );
        assert_eq!(
            parse_cli_command("aws s3api list-objects-v2 --bucket b").unwrap().operation,
            "ListObjectsV2"
        );
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
        assert!(parse_cli_command("aws ec2 describe-instances --profile prod").is_err());
        assert!(parse_cli_command("aws ec2 describe-instances --profile=prod").is_err());
        assert!(parse_cli_command("aws ec2 describe-instances --output table").is_err());
        assert!(parse_cli_command("aws ec2 describe-instances --output=table").is_err());
    }

    #[test]
    fn rejects_global_options_before_the_operation() {
        assert!(parse_cli_command("aws --region eu-west-1 ec2 describe-instances").is_err());
        assert!(parse_cli_command("aws ec2 --region eu-west-1 describe-instances").is_err());
    }

    #[test]
    fn rejects_malformed_tokens_and_quotes() {
        assert!(parse_cli_command("aws EC2 describe-instances").is_err()); // uppercase service
        assert!(parse_cli_command("aws ec2 Describe-Instances").is_err()); // non-kebab operation
        assert!(parse_cli_command("aws ec2 'describe-instances").is_err()); // unbalanced quote
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
