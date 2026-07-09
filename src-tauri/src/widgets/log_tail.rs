//! Lambda Logs — browse Lambda functions and their CloudWatch log streams.
//!
//! The legacy `log_group` tail mode is kept so saved dashboards and generated
//! "tail this lambda" inputs still work.

use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::types::OrderBy;
use serde_json::{json, Value};

use super::{err_msg, WidgetCtx};

const DEFAULT_TAIL_EVENTS: i32 = 200;
const DEFAULT_STREAM_EVENTS: i32 = 500;
const MAX_FUNCTIONS: usize = 1_000;
const MAX_STREAMS: i32 = 100;
const MAX_STREAM_EVENTS: i32 = 10_000;

fn level_for(message: &str) -> &'static str {
    let upper = message.to_uppercase();
    if upper.contains("ERROR") {
        "error"
    } else if upper.contains("WARN") {
        "warn"
    } else {
        "info"
    }
}

fn lambda_log_group(function_name: &str) -> String {
    format!("/aws/lambda/{function_name}")
}

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    match ctx.input_str("mode", "tail").as_str() {
        "list" => fetch_lambdas(ctx).await,
        "streams" => fetch_streams(ctx).await,
        "events" => fetch_stream_events(ctx).await,
        _ => fetch_tail(ctx).await,
    }
}

async fn fetch_lambdas(ctx: &WidgetCtx) -> Value {
    let max_functions = ctx
        .input_i64("max_functions", 500)
        .clamp(1, MAX_FUNCTIONS as i64) as usize;

    if let Some(denied) = ctx.preflight("lambda", "ListFunctions") {
        return denied;
    }
    let client = aws_sdk_lambda::Client::new(&ctx.sdk);

    let mut functions: Vec<Value> = Vec::new();
    let mut marker: Option<String> = None;
    loop {
        let remaining = max_functions.saturating_sub(functions.len());
        if remaining == 0 {
            break;
        }
        let page_size = remaining.min(50) as i32;
        let mut req = client.list_functions().max_items(page_size);
        if let Some(m) = &marker {
            req = req.marker(m);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return json!({"ok": false, "error": err_msg(e)}),
        };
        for f in resp.functions() {
            let name = f.function_name().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let runtime = f.runtime().map(|r| r.as_str()).unwrap_or("").to_string();
            let state = f.state().map(|s| s.as_str()).unwrap_or("").to_string();
            let architectures: Vec<String> = f
                .architectures()
                .iter()
                .map(|a| a.as_str().to_string())
                .collect();
            // Functions can log to a custom group via LoggingConfig (e.g. CDK's
            // logGroup prop); only fall back to the /aws/lambda/<name> convention
            // when no custom group is configured.
            let log_group = f
                .logging_config()
                .and_then(|lc| lc.log_group())
                .filter(|g| !g.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| lambda_log_group(&name));
            functions.push(json!({
                "name": name,
                "arn": f.function_arn().unwrap_or(""),
                "last_modified": f.last_modified().unwrap_or(""),
                "log_group": log_group,
                "runtime": runtime,
                "handler": f.handler().unwrap_or(""),
                "description": f.description().unwrap_or(""),
                "state": state,
                "memory_mb": f.memory_size().unwrap_or(0),
                "timeout_seconds": f.timeout().unwrap_or(0),
                "code_size": f.code_size(),
                "architectures": architectures,
            }));
            if functions.len() >= max_functions {
                break;
            }
        }
        marker = resp
            .next_marker()
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        if marker.is_none() {
            break;
        }
    }

    functions.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_ascii_lowercase())
    });
    let capped = functions.len() >= max_functions;
    json!({
        "ok": true,
        "account_id": ctx.account_id,
        "region": ctx.region,
        "functions": functions,
        "capped": capped,
    })
}

pub(super) async fn fetch_streams(ctx: &WidgetCtx) -> Value {
    let log_group = ctx.input_str("log_group", "");
    if log_group.is_empty() {
        return json!({"ok": false, "error": "log_group input is required"});
    }
    let max_streams = ctx
        .input_i64("max_streams", 50)
        .clamp(1, MAX_STREAMS as i64) as i32;

    if let Some(denied) = ctx.preflight("logs", "DescribeLogStreams") {
        return denied;
    }
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);
    let resp = match client
        .describe_log_streams()
        .log_group_name(&log_group)
        .order_by(OrderBy::LastEventTime)
        .descending(true)
        .limit(max_streams)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let mut error = err_msg(e);
            if error.contains("ResourceNotFoundException") {
                error = format!(
                    "{error} (log group {log_group} was not found — if this function \
                     has never been invoked, its log group does not exist yet)"
                );
            }
            return json!({
                "ok": false,
                "log_group": log_group,
                "streams": [],
                "error": error,
            });
        }
    };

    let streams: Vec<Value> = resp
        .log_streams()
        .iter()
        .filter_map(|s| {
            let name = s.log_stream_name()?;
            Some(json!({
                "name": name,
                "last_event_timestamp": s
                    .last_event_timestamp()
                    .or_else(|| s.creation_time())
                    .unwrap_or(0),
                "creation_time": s.creation_time().unwrap_or(0),
            }))
        })
        .collect();

    json!({"ok": true, "log_group": log_group, "streams": streams})
}

pub(super) async fn fetch_stream_events(ctx: &WidgetCtx) -> Value {
    let log_group = ctx.input_str("log_group", "");
    let log_stream = ctx.input_str("log_stream", "");
    if log_group.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "log_group input is required"}});
    }
    if log_stream.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "log_stream input is required"}});
    }
    let limit = ctx
        .input_i64("limit", DEFAULT_STREAM_EVENTS as i64)
        .clamp(1, MAX_STREAM_EVENTS as i64) as i32;

    if let Some(denied) = ctx.preflight("logs", "GetLogEvents") {
        return denied;
    }
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);
    let resp = match client
        .get_log_events()
        .log_group_name(&log_group)
        .log_stream_name(&log_stream)
        .start_from_head(false)
        .limit(limit)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "render": "log_stream",
                "log_group": log_group,
                "log_stream": log_stream,
                "events": [],
                "error": err_msg(e),
            });
        }
    };

    let events: Vec<Value> = resp
        .events()
        .iter()
        .map(|ev| {
            let ts = ev.timestamp().unwrap_or(0);
            let msg = ev.message().unwrap_or("").to_string();
            let level = level_for(&msg);
            json!({"ts": ts, "msg": msg, "level": level})
        })
        .collect();

    json!({
        "render": "log_stream",
        "log_group": log_group,
        "log_stream": log_stream,
        "events": events,
    })
}

async fn fetch_tail(ctx: &WidgetCtx) -> Value {
    let log_group = ctx.input_str("log_group", "");
    if log_group.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "log_group input is required"}});
    }
    let tail_minutes = ctx.input_i64("tail_minutes", 5).max(0);
    let filter = ctx.input_str("filter", "");

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let start_ms = now_ms - tail_minutes * 60 * 1000;

    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);
    if let Some(denied) = ctx.preflight("logs", "FilterLogEvents") {
        return denied;
    }
    let mut req = client
        .filter_log_events()
        .log_group_name(&log_group)
        .start_time(start_ms)
        .limit(DEFAULT_TAIL_EVENTS);
    if !filter.is_empty() {
        req = req.filter_pattern(&filter);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "render": "log_stream",
                "log_group": log_group,
                "events": [],
                "error": err_msg(e),
            });
        }
    };

    let mut events: Vec<(i64, String)> = resp
        .events()
        .iter()
        .map(|ev| {
            (
                ev.timestamp().unwrap_or(0),
                ev.message().unwrap_or("").to_string(),
            )
        })
        .collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.0));
    events.truncate(DEFAULT_TAIL_EVENTS as usize);

    let out: Vec<Value> = events
        .into_iter()
        .map(|(ts, msg)| {
            let level = level_for(&msg);
            json!({"ts": ts, "msg": msg, "level": level})
        })
        .collect();

    json!({"render": "log_stream", "log_group": log_group, "events": out})
}
