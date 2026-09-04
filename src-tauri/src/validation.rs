//! Pure IPC shape and resource-budget checks, before storage or AWS work.
//!
//! These are application limits, not a claim that AWS accepts every bounded
//! identifier. SSO snapshots and the exact CLI parser retain semantic checks.

use std::collections::HashSet;
use std::io::{self, Write};

use serde_json::{Map, Value};

type Check = Result<(), &'static str>;
const MAX_JSON_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 50_000;
const MAX_NAME: usize = 2048;
const MAX_QUERY: usize = 10 * 1024;
const MAX_COMMAND: usize = 16 * 1024;
const MAX_POLICY: usize = 64 * 1024;

/// Validate only the supplied value. This function never consults environment,
/// files, credentials, subprocesses, providers or a service transport.
pub(crate) fn validate(command: &str, params: &Value) -> Check {
    json_budget(params)?;
    let p = object(params)?;
    match command {
        "ping" | "settings_get" | "dashboard_get" | "aws_list_profiles" | "aws_auth_status"
        | "policy_get" | "cli_availability" => keys(p, &[]),
        "aws_set_account" => {
            keys(
                p,
                &[
                    "request_id",
                    "profile",
                    "account_id",
                    "region",
                    "sso_session_name",
                ],
            )?;
            request_id(p)?;
            required_text(p, "profile", 256, false)?;
            required_text(p, "account_id", 128, false)?;
            // Selection may use the configured/default region. Pinned
            // snapshots below must instead provide their own complete region.
            optional_text(p, "region", 128, true, false, true)?;
            optional_text(p, "sso_session_name", 256, true, false, true)
        }
        "aws_list_pipelines" => {
            keys(p, &["request_id", "context", "account_override"])?;
            request_id(p)?;
            request_context(p)
        }
        "widget_fetch" => {
            keys(
                p,
                &[
                    "request_id",
                    "widget",
                    "inputs",
                    "context",
                    "account_override",
                ],
            )?;
            request_id(p)?;
            request_context(p)?;
            let name = required_text(p, "widget", 128, false)?;
            let empty = Map::new();
            let inputs = match p.get("inputs") {
                None => &empty,
                Some(value) => object(value)?,
            };
            widget_inputs(name, inputs)
        }
        "widget_get_source" => {
            keys(p, &["widget"])?;
            let name = required_text(p, "widget", 128, false)?;
            if crate::widgets::is_known(name) {
                Ok(())
            } else {
                Err("Unknown widget")
            }
        }
        "settings_set" => {
            settings_shape(params)?;
            // IPC writes replace the complete editable form. Missing values
            // must never silently reset a custom credential configuration.
            if SETTINGS_FIELDS
                .iter()
                .any(|(field, _, _)| !p.contains_key(*field))
            {
                return Err("Submit every settings field; use a blank value to reset a field");
            }
            if settings_field_errors(params).is_empty() {
                Ok(())
            } else {
                Err("Correct the highlighted settings fields")
            }
        }
        "dashboard_set" => {
            keys(p, &["tiles"])?;
            let tiles = array(required(p, "tiles")?, 200)?;
            let mut ids = HashSet::new();
            for tile in tiles {
                let tile = object(tile)?;
                keys(tile, &["id", "widget", "x", "y", "w", "h", "config"])?;
                let id = required_text(tile, "id", 256, false)?;
                if !ids.insert(id) {
                    return Err("Dashboard tile IDs must be unique");
                }
                optional_text(tile, "widget", 128, false, false, false)?;
                integer(tile, "x", 0, 11)?;
                integer(tile, "y", 0, 100_000)?;
                integer(tile, "w", 1, 12)?;
                integer(tile, "h", 1, 1000)?;
                if let Some(config) = tile.get("config") {
                    dashboard_config(config)?;
                }
            }
            Ok(())
        }
        "audit_tail" => {
            keys(p, &["limit"])?;
            integer(p, "limit", 0, 1000)
        }
        "policy_set" => {
            keys(p, &["text"])?;
            text(required(p, "text")?, MAX_POLICY, true, true).map(|_| ())
        }
        _ => Err("Unknown command"),
    }
}

const SETTINGS_FIELDS: [(&str, usize, &str); 5] = [
    ("aws_config_path", 4096, "Enter a valid AWS config path"),
    ("sso_session_name", 256, "Enter a valid SSO session name"),
    ("default_profile", 256, "Enter a valid default profile"),
    ("default_region", 128, "Choose a supported default region"),
    ("theme", 16, "Choose light or dark theme"),
];

/// On-disk shape validation intentionally does not reject unsupported semantic
/// preference values; loading must preserve them for an explicit correction.
pub(crate) fn settings_shape(value: &Value) -> Check {
    json_budget(value)?;
    let fields = object(value)?;
    keys(
        fields,
        &[
            "aws_config_path",
            "sso_session_name",
            "default_profile",
            "default_region",
            "theme",
        ],
    )?;
    for (name, limit, _) in SETTINGS_FIELDS {
        settings_field_shape(fields, name, limit)?;
    }
    Ok(())
}

pub(crate) fn settings_field_errors(value: &Value) -> Map<String, Value> {
    let mut errors = Map::new();
    let Some(fields) = value.as_object() else {
        return errors;
    };
    for (name, limit, message) in SETTINGS_FIELDS {
        let invalid_shape = settings_field_shape(fields, name, limit).is_err();
        let text = fields
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let invalid_choice = !text.is_empty()
            && match name {
                "default_region" => !crate::settings::allowed_region(text),
                "theme" => !matches!(text, "light" | "dark"),
                _ => false,
            };
        if invalid_shape || invalid_choice {
            errors.insert(name.into(), Value::String(message.into()));
        }
    }
    errors
}

fn settings_field_shape(fields: &Map<String, Value>, name: &str, limit: usize) -> Check {
    let Some(value) = fields.get(name) else {
        return Ok(());
    };
    let text = value.as_str().ok_or("Expected text")?.trim();
    if text.len() > limit || text.chars().any(char::is_control) {
        Err("Setting is too long or contains unsupported controls")
    } else {
        Ok(())
    }
}

fn object(value: &Value) -> Result<&Map<String, Value>, &'static str> {
    value.as_object().ok_or("Expected an object")
}
fn keys(value: &Map<String, Value>, allowed: &[&str]) -> Check {
    if value.keys().any(|key| !allowed.contains(&key.as_str())) {
        Err("Unsupported field")
    } else {
        Ok(())
    }
}
fn required<'a>(value: &'a Map<String, Value>, key: &str) -> Result<&'a Value, &'static str> {
    value.get(key).ok_or("Required field is missing")
}
fn text(value: &Value, max: usize, empty: bool, multiline: bool) -> Result<&str, &'static str> {
    let value = value.as_str().ok_or("Expected text")?;
    if value.len() > max
        || (!empty && value.trim().is_empty())
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err("Text is empty, too long or contains unsupported controls");
    }
    Ok(value)
}
fn required_text<'a>(
    p: &'a Map<String, Value>,
    key: &str,
    max: usize,
    multiline: bool,
) -> Result<&'a str, &'static str> {
    text(required(p, key)?, max, false, multiline)
}
fn optional_text(
    p: &Map<String, Value>,
    key: &str,
    max: usize,
    empty: bool,
    multiline: bool,
    null: bool,
) -> Check {
    match p.get(key) {
        None => Ok(()),
        Some(Value::Null) if null => Ok(()),
        Some(value) => text(value, max, empty, multiline).map(|_| ()),
    }
}
fn integer(p: &Map<String, Value>, key: &str, min: i64, max: i64) -> Check {
    if let Some(value) = p.get(key) {
        let value = value.as_i64().ok_or("Expected an integer")?;
        if !(min..=max).contains(&value) {
            return Err("Integer is outside the application limit");
        }
    }
    Ok(())
}
fn array(value: &Value, max: usize) -> Result<&[Value], &'static str> {
    let values = value.as_array().ok_or("Expected an array")?;
    if values.len() > max {
        return Err("Array exceeds the application limit");
    }
    Ok(values)
}
fn request_id(p: &Map<String, Value>) -> Check {
    match p.get("request_id") {
        None | Some(Value::Null) => Ok(()),
        Some(value) => {
            let id = text(value, 128, false, false)?;
            if id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            {
                Ok(())
            } else {
                Err("Invalid request ID")
            }
        }
    }
}
fn identity(p: &Map<String, Value>) -> Check {
    required_text(p, "profile", 256, false)?;
    required_text(p, "account_id", 128, false)?;
    required_text(p, "region", 128, false)?;
    Ok(())
}
fn request_context(p: &Map<String, Value>) -> Check {
    if p.contains_key("context") && p.contains_key("account_override") {
        return Err("Conflicting context fields");
    }
    if let Some(context) = p.get("context").or_else(|| p.get("account_override")) {
        context_shape(context)?;
    }
    Ok(())
}
fn context_shape(value: &Value) -> Check {
    let context = object(value)?;
    keys(context, &["mode", "profile", "account_id", "region"])?;
    let mode = match context.get("mode") {
        None => "pinned", // Existing account_override snapshot format.
        Some(value) => text(value, 16, true, false)?,
    };
    match mode {
        "pinned" | "" => identity(context),
        "inherit" => {
            for key in ["profile", "account_id", "region"] {
                if let Some(value) = context.get(key) {
                    if !value.is_null() {
                        return Err("Inherited context cannot override identity");
                    }
                }
            }
            Ok(())
        }
        _ => Err("Unknown context mode"),
    }
}

fn widget_inputs(name: &str, p: &Map<String, Value>) -> Check {
    match name {
        "aws-cli" => {
            keys(p, &["command"])?;
            required_text(p, "command", MAX_COMMAND, false)?;
        }
        "cfn-stacks" => {
            keys(p, &["name_prefix", "status_filter"])?;
            optional_text(p, "name_prefix", MAX_NAME, true, false, false)?;
            if let Some(values) = p.get("status_filter") {
                for status in array(values, 64)? {
                    text(status, 128, false, false)?;
                }
            }
        }
        "cfn-stack-detail" => {
            keys(p, &["stack_name"])?;
            required_text(p, "stack_name", MAX_NAME, false)?;
        }
        "pipeline-runs" => {
            keys(p, &["pipeline_name", "max_results"])?;
            required_text(p, "pipeline_name", MAX_NAME, false)?;
            integer(p, "max_results", 1, 100)?;
        }
        "pipeline-execution-detail" => {
            keys(p, &["pipeline_name", "execution_id"])?;
            required_text(p, "pipeline_name", MAX_NAME, false)?;
            required_text(p, "execution_id", 256, false)?;
        }
        "codebuild-log" => {
            keys(p, &["build_id"])?;
            required_text(p, "build_id", MAX_NAME, false)?;
        }
        "resource-lookup" => {
            keys(p, &["query", "max_results"])?;
            required_text(p, "query", MAX_NAME, false)?;
            integer(p, "max_results", 1, 1000)?;
        }
        "errors-by-stack" => {
            keys(p, &["hours", "log_group_pattern"])?;
            integer(p, "hours", 1, 168)?;
            optional_text(p, "log_group_pattern", MAX_NAME, true, false, false)?;
        }
        "log-tail" | "cloudwatch-logs" | "logs-insights" => log_inputs(name, p)?,
        "codeartifact-packages" => {
            keys(
                p,
                &[
                    "domain",
                    "repository",
                    "package_prefix",
                    "domain_owner",
                    "max_packages",
                ],
            )?;
            for key in ["domain", "repository", "package_prefix"] {
                optional_text(p, key, MAX_NAME, false, false, false)?;
            }
            optional_text(p, "domain_owner", 128, true, false, false)?;
            integer(p, "max_packages", 1, 1000)?;
        }
        "codeartifact-package-version-history" => {
            keys(
                p,
                &[
                    "domain",
                    "repository",
                    "package",
                    "domain_owner",
                    "versions",
                ],
            )?;
            for key in ["domain", "repository", "package"] {
                required_text(p, key, MAX_NAME, false)?;
            }
            optional_text(p, "domain_owner", 128, true, false, false)?;
            let versions = array(required(p, "versions")?, 10)?;
            if versions.is_empty() {
                return Err("At least one package version is required");
            }
            for version in versions {
                if version.is_string() {
                    text(version, MAX_NAME, false, false)?;
                } else {
                    let version = object(version)?;
                    keys(version, &["version", "published"])?;
                    required_text(version, "version", MAX_NAME, false)?;
                    optional_text(version, "published", 128, true, false, false)?;
                }
            }
        }
        _ => return Err("Unknown widget"),
    }
    Ok(())
}

fn log_inputs(name: &str, p: &Map<String, Value>) -> Check {
    let default = match name {
        "cloudwatch-logs" => "groups",
        "logs-insights" => "query",
        _ => "tail",
    };
    let mode = match p.get("mode") {
        None => default,
        Some(value) => text(value, 16, false, false)?,
    };
    match mode {
        "list" if name == "log-tail" => {
            keys(p, &["mode", "max_functions"])?;
            integer(p, "max_functions", 1, 1000)
        }
        "groups" if name != "log-tail" => {
            keys(p, &["mode", "max_groups", "name_pattern"])?;
            integer(p, "max_groups", 1, 1000)?;
            optional_text(p, "name_pattern", MAX_NAME, true, false, false)
        }
        "streams" if name != "logs-insights" => {
            keys(p, &["mode", "log_group", "max_streams"])?;
            required_text(p, "log_group", MAX_NAME, false)?;
            integer(p, "max_streams", 1, 100)
        }
        "events" if name != "logs-insights" => {
            keys(p, &["mode", "log_group", "log_stream", "limit"])?;
            required_text(p, "log_group", MAX_NAME, false)?;
            required_text(p, "log_stream", MAX_NAME, false)?;
            integer(p, "limit", 1, 10_000)
        }
        "tail" if name == "log-tail" => {
            keys(p, &["mode", "log_group", "tail_minutes", "filter"])?;
            required_text(p, "log_group", MAX_NAME, false)?;
            integer(p, "tail_minutes", 0, 10_080)?;
            optional_text(p, "filter", MAX_QUERY, true, true, false)
        }
        "query" if name == "logs-insights" => {
            keys(p, &["mode", "log_group", "query", "range_seconds"])?;
            required_text(p, "log_group", MAX_NAME, false)?;
            required_text(p, "query", MAX_QUERY, true)?;
            integer(p, "range_seconds", 60, 604_800)
        }
        _ => Err("Unknown widget mode"),
    }
}

fn dashboard_config(value: &Value) -> Check {
    let config = object(value)?;
    keys(
        config,
        &["context", "account_override", "header_color", "inputs"],
    )?;
    request_context(config)?;
    if let Some(color) = config.get("header_color").filter(|v| !v.is_null()) {
        let color = text(color, 16, false, false)?;
        if !["blue", "green", "amber", "pink", "purple", "red"].contains(&color) {
            return Err("Unknown header color");
        }
    }
    if let Some(inputs) = config.get("inputs") {
        let inputs = object(inputs)?;
        // Inputs remain frontend-owned draft state; a saved draft need not yet
        // satisfy a widget's required execution fields. Known pins are bounded
        // snapshots, while all other nested values obey the global JSON budget.
        for (key, field) in [
            ("pinned_pipelines", "pipeline_name"),
            ("pinned_cli_commands", "command"),
        ] {
            if let Some(pins) = inputs.get(key) {
                for pin in array(pins, 50)? {
                    let pin = object(pin)?;
                    keys(pin, &["id", field, "profile", "account_id", "region"])?;
                    identity(pin)?;
                    // Frontend pin IDs combine the command with its identity.
                    optional_text(pin, "id", MAX_COMMAND + 1024, true, false, false)?;
                    required_text(
                        pin,
                        field,
                        if field == "command" {
                            MAX_COMMAND
                        } else {
                            MAX_NAME
                        },
                        false,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn json_budget(value: &Value) -> Check {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Check {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return Err("JSON nesting or item limit exceeded");
        }
        match value {
            Value::Array(values) => {
                for child in values {
                    visit(child, depth + 1, nodes)?;
                }
            }
            Value::Object(values) => {
                for (key, child) in values {
                    if key.len() > 128 || key.chars().any(char::is_control) {
                        return Err("Invalid object key");
                    }
                    visit(child, depth + 1, nodes)?;
                }
            }
            Value::String(value) if value.len() > MAX_POLICY => {
                return Err("JSON string limit exceeded")
            }
            _ => {}
        }
        Ok(())
    }
    struct Budget(usize);
    impl Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(io::Error::other("JSON size limit exceeded"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    visit(value, 0, &mut 0)?;
    serde_json::to_writer(Budget(MAX_JSON_BYTES), value).map_err(|_| "JSON size limit exceeded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn widget(name: &str, inputs: Value) -> Value {
        json!({"request_id":"fixture-1", "widget": name, "inputs": inputs,
            "context":{"mode":"inherit", "profile":null, "account_id":null, "region":null}})
    }

    #[test]
    fn all_dispatch_entries_have_a_valid_explicit_shape() {
        let cases = [
            ("aws-cli", json!({"command":"aws sts get-caller-identity"})),
            ("cfn-stacks", json!({})),
            ("cfn-stack-detail", json!({"stack_name":"stack-fixture"})),
            ("cloudwatch-logs", json!({"mode":"groups","max_groups":500})),
            (
                "logs-insights",
                json!({"log_group":"/fixture/group","query":"fields @message\n| limit 1","range_seconds":604800}),
            ),
            ("log-tail", json!({"mode":"list","max_functions":1000})),
            ("errors-by-stack", json!({"hours":168})),
            (
                "resource-lookup",
                json!({"query":"resource-fixture","max_results":1000}),
            ),
            (
                "pipeline-runs",
                json!({"pipeline_name":"pipeline-fixture","max_results":100}),
            ),
            ("codeartifact-packages", json!({})),
            (
                "codeartifact-package-version-history",
                json!({"domain":"domain-fixture","repository":"repo-fixture", "package":"package-fixture", "versions":["1.0.0",{"version":"1.0.1","published":"2026-01-01T00:00:00Z"}]}),
            ),
            (
                "pipeline-execution-detail",
                json!({"pipeline_name":"pipeline-fixture","execution_id":"execution-fixture"}),
            ),
            ("codebuild-log", json!({"build_id":"build-fixture"})),
        ];
        for (name, inputs) in cases {
            assert!(crate::widgets::is_known(name));
            assert_eq!(
                validate("widget_fetch", &widget(name, inputs)),
                Ok(()),
                "{name}"
            );
        }
    }

    #[test]
    fn exact_modes_accept_only_their_own_fields_and_required_values() {
        let valid = [
            (
                "cloudwatch-logs",
                json!({"mode":"streams","log_group":"/fixture/group","max_streams":100}),
            ),
            (
                "cloudwatch-logs",
                json!({"mode":"events","log_group":"/fixture/group","log_stream":"stream-fixture","limit":10000}),
            ),
            (
                "logs-insights",
                json!({"mode":"groups","name_pattern":"fixture","max_groups":1000}),
            ),
            (
                "log-tail",
                json!({"log_group":"/fixture/group","tail_minutes":10080,"filter":"ERROR"}),
            ),
            (
                "log-tail",
                json!({"mode":"events","log_group":"/fixture/group","log_stream":"stream-fixture"}),
            ),
        ];
        for (name, inputs) in valid {
            assert!(validate("widget_fetch", &widget(name, inputs)).is_ok());
        }
        for (name, inputs) in [
            ("log-tail", json!({"mode":"groups"})),
            (
                "logs-insights",
                json!({"mode":"events","log_group":"/fixture/group","log_stream":"stream-fixture"}),
            ),
            ("cloudwatch-logs", json!({"mode":"unknown"})),
            (
                "cloudwatch-logs",
                json!({"mode":"streams","max_streams":50}),
            ),
            (
                "log-tail",
                json!({"mode":"events","log_group":"/fixture/group"}),
            ),
            (
                "log-tail",
                json!({"mode":"list","log_group":"/fixture/group"}),
            ),
            (
                "logs-insights",
                json!({"mode":"query","log_group":"/fixture/group","query":"   "}),
            ),
        ] {
            assert!(validate("widget_fetch", &widget(name, inputs)).is_err());
        }
    }

    #[test]
    fn numeric_values_are_integers_in_range_not_coerced_or_clamped() {
        for bad in [
            json!("24"),
            json!(1.5),
            json!(-1),
            json!(0),
            json!(169),
            json!(u64::MAX),
            Value::Null,
        ] {
            assert!(validate(
                "widget_fetch",
                &widget("errors-by-stack", json!({"hours":bad}))
            )
            .is_err());
        }
        for (name, inputs) in [
            (
                "log-tail",
                json!({"log_group":"/fixture/group","tail_minutes":10081}),
            ),
            (
                "pipeline-runs",
                json!({"pipeline_name":"fixture","max_results":101}),
            ),
            (
                "resource-lookup",
                json!({"query":"fixture","max_results":1001}),
            ),
            ("codeartifact-packages", json!({"max_packages":0})),
            (
                "logs-insights",
                json!({"log_group":"/fixture/group","query":"fields @message","range_seconds":59}),
            ),
            (
                "logs-insights",
                json!({"log_group":"/fixture/group","query":"fields @message","range_seconds":604801}),
            ),
        ] {
            assert!(validate("widget_fetch", &widget(name, inputs)).is_err());
        }
    }

    #[test]
    fn context_snapshots_are_complete_unambiguous_and_preserve_frontend_nulls() {
        let identity =
            json!({"profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1"});
        assert!(validate("aws_set_account", &json!({"profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1","sso_session_name":null})).is_ok());
        assert!(validate(
            "aws_set_account",
            &json!({"profile":"demo-fixture","account_id":"acct-a-fixture"})
        )
        .is_ok());
        for region in [Value::Null, json!("")] {
            assert!(validate(
                "aws_set_account",
                &json!({"profile":"demo-fixture","account_id":"acct-a-fixture","region":region})
            )
            .is_ok());
        }
        assert!(validate(
            "aws_set_account",
            &json!({"profile":"demo-fixture","account_id":"acct-a-fixture","region":42})
        )
        .is_err());
        assert!(validate("aws_list_pipelines", &json!({"account_override":identity})).is_ok());
        assert!(validate("aws_list_pipelines", &json!({"context":{"mode":"","profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1"}})).is_ok());
        assert!(validate("aws_list_pipelines", &json!({"context":{"mode":"pinned","profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1"}})).is_ok());
        for params in [
            json!({"context":{"mode":"pinned","account_id":"acct-a-fixture","region":"eu-west-1"}}),
            json!({"context":{"mode":"inherit","profile":"ambient-fixture"}}),
            json!({"context":{"mode":"inherit"},"account_override":identity}),
            json!({"context":{"mode":"pinned","profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1","endpoint":"https://fixture.invalid"}}),
            json!({"profile":"ambient-fixture"}),
        ] {
            assert!(validate("aws_list_pipelines", &params).is_err());
        }
    }

    #[test]
    fn envelope_unknown_fields_and_malformed_inputs_fail_before_semantic_work() {
        for params in [
            json!([]),
            json!({"widget":"unknown-fixture","inputs":{}}),
            json!({"widget":"cfn-stacks","inputs":null}),
            json!({"widget":"cfn-stacks","inputs":[]}),
            json!({"widget":"cfn-stacks","inputs":{},"credentials":{"key":"synthetic-not-a-key"}}),
            json!({"widget":"cfn-stacks","inputs":{"profile":"ambient-fixture"}}),
            json!({"widget":"cfn-stacks","request_id":"bad request fixture"}),
            json!({"widget":"cfn-stacks","request_id":42}),
        ] {
            assert!(validate("widget_fetch", &params).is_err());
        }
        assert!(validate("unknown-command", &json!({})).is_err());
        // A bounded CLI string passes this structural layer; the existing
        // exact operation/argument parser must still deny unsupported actions.
        assert!(validate(
            "widget_fetch",
            &widget("aws-cli", json!({"command":"aws fixture unsupported"}))
        )
        .is_ok());
    }

    #[test]
    fn version_seeds_have_a_bounded_closed_shape() {
        let mut inputs = json!({"domain":"domain-fixture","repository":"repo-fixture","package":"package-fixture","versions":[]});
        for versions in [
            json!([]),
            json!([{}]),
            json!([42]),
            json!([{"version":"1.0.0","published":42}]),
            json!([{"version":"1.0.0","unknown":"fixture"}]),
            json!(vec!["1.0.0"; 11]),
        ] {
            inputs["versions"] = versions;
            assert!(validate(
                "widget_fetch",
                &widget("codeartifact-package-version-history", inputs.clone())
            )
            .is_err());
        }
    }

    #[test]
    fn dashboard_drafts_remain_opaque_but_contexts_pins_and_geometry_are_checked() {
        let tile = json!({"id":"tile-fixture","widget":"codeartifact-packages","x":0,"y":0,"w":4,"h":3,
            "config":{"header_color":null,"inputs":{"unfinished_frontend_draft":{"selection":null},"domain":""}}});
        assert!(validate("dashboard_set", &json!({"tiles":[tile.clone()]})).is_ok());
        assert!(validate(
            "dashboard_set",
            &json!({"tiles":[tile.clone(),tile.clone()]})
        )
        .is_err());
        for (key, bad) in [
            ("w", json!(0)),
            ("x", json!(12)),
            ("y", json!(-1)),
            ("h", json!(2.5)),
            ("widget", json!([])),
            ("id", json!("")),
        ] {
            let mut changed = tile.clone();
            changed[key] = bad;
            assert!(validate("dashboard_set", &json!({"tiles":[changed]})).is_err());
        }
        let config = json!({"inputs":{"pinned_cli_commands":[{"id":"pin-fixture","command":"aws sts get-caller-identity", "profile":"demo-fixture","account_id":"acct-a-fixture","region":"eu-west-1"}]}});
        assert!(dashboard_config(&config).is_ok());
        let mut bad = config.clone();
        bad["inputs"]["pinned_cli_commands"][0]["region"] = Value::Null;
        assert!(dashboard_config(&bad).is_err());
        assert!(dashboard_config(&json!({"header_color":"unreviewed-fixture"})).is_err());
        assert!(validate("dashboard_set", &json!({"tiles":"not-an-array"})).is_err());
        let too_many: Vec<_> = (0..201)
            .map(|i| json!({"id":format!("tile-fixture-{i}")}))
            .collect();
        assert!(validate("dashboard_set", &json!({"tiles":too_many})).is_err());
    }

    #[test]
    fn settings_policy_and_audit_limits_preserve_valid_ui_values() {
        assert!(validate("settings_set", &json!({"aws_config_path":"~/.aws/config","default_profile":"", "sso_session_name":"", "default_region":"eu-west-1", "theme":"dark"})).is_ok());
        assert!(validate(
            "settings_set",
            &json!({"aws_config_path":"C:\\fixture\\config", "default_profile":"", "sso_session_name":"", "default_region":"", "theme":""})
        )
        .is_ok());
        assert!(validate("policy_set", &json!({"text":"Version: 1\nStatement: []\n"})).is_ok());
        assert!(validate("audit_tail", &json!({"limit":300})).is_ok());
        for (command, params) in [
            ("settings_set", json!({"default_region":42})),
            (
                "settings_set",
                json!({"aws_config_path":"fixture\u{0}config"}),
            ),
            (
                "settings_set",
                json!({"credentials":"synthetic-not-a-secret"}),
            ),
            ("audit_tail", json!({"limit":1001})),
            ("audit_tail", json!({"limit":"200"})),
            ("policy_set", json!({"text":"x".repeat(MAX_POLICY+1)})),
            ("policy_set", json!({"text":null})),
        ] {
            assert!(validate(command, &params).is_err());
        }
        for command in [
            "ping",
            "settings_get",
            "dashboard_get",
            "aws_list_profiles",
            "aws_auth_status",
            "cli_availability",
            "policy_get",
        ] {
            assert!(validate(command, &json!({})).is_ok());
            assert!(validate(command, &json!({"unexpected":true})).is_err());
        }
    }

    #[test]
    fn settings_ipc_requires_full_form_while_legacy_storage_shape_remains_optional() {
        let complete = Value::Object(crate::settings::defaults());
        assert!(validate("settings_set", &complete).is_ok());
        for (field, _, _) in SETTINGS_FIELDS {
            let mut partial = complete.clone();
            partial.as_object_mut().unwrap().remove(field);
            assert_eq!(
                validate("settings_set", &partial),
                Err("Submit every settings field; use a blank value to reset a field")
            );
            assert!(settings_shape(&partial).is_ok());
        }
    }

    #[test]
    fn generic_budget_rejects_deep_wide_and_serialized_oversized_values() {
        let mut deep = Value::Null;
        for _ in 0..=MAX_DEPTH {
            deep = json!([deep]);
        }
        assert!(json_budget(&deep).is_err());
        assert!(json_budget(&json!(vec![Value::Null; MAX_NODES])).is_err());
        assert!(json_budget(&json!(vec!["x".repeat(MAX_POLICY); 17])).is_err());
        // Control characters expand during JSON serialization; counting only
        // decoded bytes would incorrectly admit this otherwise bounded input.
        assert!(json_budget(&json!(vec!["\u{1}".repeat(32_000); 6])).is_err());
        assert!(json_budget(&json!({"fixture": [null, true, 42, "bounded"]})).is_ok());
    }
}
