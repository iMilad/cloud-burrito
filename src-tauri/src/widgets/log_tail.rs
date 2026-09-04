//! Lambda Logs — browse Lambda functions and their CloudWatch log streams.
//!
//! The legacy `log_group` tail mode is kept so saved dashboards and generated
//! "tail this lambda" inputs still work.

use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::types::OrderBy;
use serde_json::{json, Value};

use super::handoff::{self, Source};
use super::{budget::PageBudget, coverage::Coverage, err_msg, WidgetCtx};

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
    let mut pages = 0;
    let mut failure = None;
    let mut locally_omitted = false;
    let mut budget = PageBudget::default();
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
        let resp = match ctx.send("lambda", "ListFunctions", req.send()).await {
            Ok(r) => r,
            Err(e) => {
                failure = Some(err_msg(e));
                break;
            }
        };
        pages += 1;
        marker = resp
            .next_marker()
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        let continue_scan = budget.advance(marker.as_deref());
        for (index, f) in resp.functions().iter().enumerate() {
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
            let configured_group = f
                .logging_config()
                .and_then(|lc| lc.log_group())
                .filter(|g| !g.is_empty());
            let log_group = configured_group
                .map(str::to_string)
                .unwrap_or_else(|| lambda_log_group(&name));
            let source = if configured_group.is_some() {
                Source::LambdaLoggingConfig
            } else {
                Source::LambdaDefaultConvention
            };
            let logs = if f.function_arn().is_some_and(|arn| {
                !handoff::same_context_arn(arn, "lambda", "function:", ctx)
                    || arn
                        .splitn(6, ':')
                        .nth(5)
                        .and_then(|resource| resource.strip_prefix("function:"))
                        != Some(name.as_str())
            }) {
                handoff::unavailable(Source::ContextMismatch)
            } else {
                handoff::logs(&log_group, None, source)
            };
            let row = json!({
                "name": name,
                "arn": f.function_arn().unwrap_or(""),
                "last_modified": f.last_modified().unwrap_or(""),
                "log_group": log_group,
                "handoffs": {"logs":logs},
                "runtime": runtime,
                "handler": f.handler().unwrap_or(""),
                "description": f.description().unwrap_or(""),
                "state": state,
                "memory_mb": f.memory_size().unwrap_or(0),
                "timeout_seconds": f.timeout().unwrap_or(0),
                "code_size": f.code_size(),
                "architectures": architectures,
            });
            if !budget.retain(&row) {
                locally_omitted = true;
                break;
            }
            functions.push(row);
            if functions.len() >= max_functions {
                locally_omitted = index + 1 < resp.functions().len();
                break;
            }
        }
        if !continue_scan || budget.stopped() {
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
    let capped = locally_omitted
        || budget.stopped()
        || (functions.len() >= max_functions && marker.is_some());
    let mut coverage = Coverage::complete(functions.len());
    coverage.count("pages", pages);
    coverage.limit("results", Some(max_functions));
    if locally_omitted || (functions.len() >= max_functions && marker.is_some()) {
        coverage.has_more(Some(true));
        coverage.limited(
            "function_limit",
            "The requested function limit omitted further results.",
        );
    }
    if failure.is_some() {
        coverage.failure(
            "function_page_failed",
            "A function page could not be loaded; retained functions are incomplete.",
            !functions.is_empty(),
        );
    }
    budget.apply(&mut coverage, !functions.is_empty());
    let mut result = json!({
        "ok": failure.is_none(),
        "account_id": ctx.account_id,
        "region": ctx.region,
        "functions": functions,
        "capped": capped,
    });
    if let Some(error) = failure {
        result["error"] = json!(error);
    }
    coverage.attach(result)
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
    let resp = match ctx
        .send(
            "logs",
            "DescribeLogStreams",
            client
                .describe_log_streams()
                .log_group_name(&log_group)
                .order_by(OrderBy::LastEventTime)
                .descending(true)
                .limit(max_streams)
                .send(),
        )
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
            let mut coverage = Coverage::unknown(0);
            coverage.count("pages", 0);
            coverage.limit("results", Some(max_streams as usize));
            coverage.failure(
                "stream_listing_failed",
                "Log streams could not be loaded.",
                false,
            );
            return coverage.attach(json!({
                "ok": false,
                "log_group": log_group,
                "streams": [],
                "error": error,
            }));
        }
    };

    let mut budget = PageBudget::default();
    budget.advance(None);
    let streams: Vec<Value> = resp
        .log_streams()
        .iter()
        .take(max_streams as usize)
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
        .take_while(|row| budget.retain(row))
        .collect();

    let mut coverage = Coverage::complete(streams.len());
    coverage.count("pages", 1);
    coverage.limit("results", Some(max_streams as usize));
    if resp.next_token().is_some_and(|token| !token.is_empty())
        || resp.log_streams().len() > streams.len()
    {
        coverage.has_more(Some(true));
        coverage.limited(
            "single_stream_page",
            "Only the first log stream page was loaded.",
        );
    }
    budget.apply(&mut coverage, !streams.is_empty());
    coverage.attach(json!({"ok": true, "log_group": log_group, "streams": streams}))
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
    let resp = match ctx
        .send(
            "logs",
            "GetLogEvents",
            client
                .get_log_events()
                .log_group_name(&log_group)
                .log_stream_name(&log_stream)
                .start_from_head(false)
                .limit(limit)
                .send(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let mut coverage = Coverage::unknown(0);
            coverage.count("pages", 0);
            coverage.limit("events", Some(limit as usize));
            coverage.failure(
                "event_page_failed",
                "Log events could not be loaded.",
                false,
            );
            return coverage.attach(json!({
                "render": "log_stream",
                "log_group": log_group,
                "log_stream": log_stream,
                "events": [],
                "error": err_msg(e),
            }));
        }
    };

    let mut budget = PageBudget::default();
    budget.advance(None);
    let events: Vec<Value> = resp
        .events()
        .iter()
        .take(limit as usize)
        .map(|ev| {
            let ts = ev.timestamp().unwrap_or(0);
            let msg = ev.message().unwrap_or("").to_string();
            let level = level_for(&msg);
            json!({"ts": ts, "msg": msg, "level": level})
        })
        .take_while(|row| budget.retain(row))
        .collect();

    // GetLogEvents supplies navigation tokens even at a stream boundary. A
    // single request cannot establish that this is the complete stream.
    let mut coverage = Coverage::unknown(events.len());
    coverage.count("pages", 1);
    coverage.limit("events", Some(limit as usize));
    coverage.unknown_reason(
        "single_event_page",
        "One log event page was loaded; complete stream coverage is unknown.",
    );
    if resp.events().len() > events.len() {
        coverage.limited(
            "event_limit",
            "The local event or result byte limit omitted returned events.",
        );
    }
    budget.apply(&mut coverage, !events.is_empty());
    coverage.attach(json!({
        "render": "log_stream",
        "log_group": log_group,
        "log_stream": log_stream,
        "events": events,
    }))
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
    let resp = match ctx.send("logs", "FilterLogEvents", req.send()).await {
        Ok(r) => r,
        Err(e) => {
            let mut coverage = Coverage::unknown(0);
            coverage.count("pages", 0);
            coverage.limit("events", Some(DEFAULT_TAIL_EVENTS as usize));
            coverage.failure(
                "tail_page_failed",
                "Log events for the selected window could not be loaded.",
                false,
            );
            return coverage.attach(json!({
                "render": "log_stream",
                "log_group": log_group,
                "events": [],
                "error": err_msg(e),
            }));
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

    let mut budget = PageBudget::default();
    budget.advance(None);
    let out: Vec<Value> = events
        .into_iter()
        .map(|(ts, msg)| {
            let level = level_for(&msg);
            json!({"ts": ts, "msg": msg, "level": level})
        })
        .take_while(|row| budget.retain(row))
        .collect();

    let mut coverage = Coverage::complete(out.len());
    coverage.count("pages", 1);
    coverage.limit("events", Some(DEFAULT_TAIL_EVENTS as usize));
    if resp.next_token().is_some_and(|token| !token.is_empty()) || resp.events().len() > out.len() {
        coverage.has_more(Some(true));
        coverage.limited(
            "single_tail_page",
            "Only one page of events from the selected time window was loaded.",
        );
    }
    budget.apply(&mut coverage, !out.is_empty());
    coverage.attach(json!({"render": "log_stream", "log_group": log_group, "events": out}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    #[tokio::test]
    async fn lambda_configured_group_and_default_convention_have_distinct_provenance() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![ExpectedRequest::rest(
            "GET",
            "/2015-03-31/functions",
            json!({"Marker":null}),
            json!({"Functions":[
                {"FunctionName":"synthetic-a-custom","FunctionArn":"arn:aws:lambda:us-east-1:acct-producer-fixture:function:synthetic-a-custom","LoggingConfig":{"LogGroup":"/synthetic/custom/exact"}},
                {"FunctionName":"synthetic-b-default"},
                {"FunctionName":"synthetic-c-foreign","FunctionArn":"arn:aws:lambda:eu-west-1:acct-other-fixture:function:synthetic-c-foreign","LoggingConfig":{"LogGroup":"/synthetic/foreign"}},
                {"FunctionName":"synthetic-d-mismatch","FunctionArn":"arn:aws:lambda:us-east-1:acct-producer-fixture:function:synthetic-other"}
            ]}),
        )]);
        let result = fetch(&script.context(&dir, "log-tail", json!({"mode":"list"}))).await;
        script.assert_finished();
        assert_eq!(script.calls(), 1);
        let custom = &result["functions"][0]["handoffs"]["logs"];
        assert_eq!(custom["status"], "available");
        assert_eq!(custom["source"], "lambda_logging_config");
        assert_eq!(
            custom["inputs"],
            json!({"mode":"streams","log_group":"/synthetic/custom/exact"})
        );
        let default = &result["functions"][1]["handoffs"]["logs"];
        assert_eq!(default["status"], "manual");
        assert_eq!(default["source"], "lambda_default_convention");
        assert_eq!(
            default["inputs"]["log_group"],
            "/aws/lambda/synthetic-b-default"
        );
        assert!(default["reason"].as_str().unwrap().contains("not verified"));
        for row in &result["functions"].as_array().unwrap()[2..] {
            assert_eq!(row["handoffs"]["logs"]["status"], "unavailable");
            assert_eq!(row["handoffs"]["logs"]["source"], "context_mismatch");
        }
    }

    #[tokio::test]
    async fn lambda_listing_failure_retains_functions_and_custom_log_group() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::rest(
                "GET",
                "/2015-03-31/functions",
                json!({"Marker":null}),
                json!({"Functions":[{"FunctionName":"synthetic-function", "LoggingConfig":{"LogGroup":"/synthetic/custom"}}], "NextMarker":"synthetic-next"}),
            ),
            ExpectedRequest::rest(
                "GET",
                "/2015-03-31/functions",
                json!({"Marker":"synthetic-next"}),
                json!({"Type":"User", "message":"SYNTHETIC_PRIVATE_FUNCTION_ERROR"}),
            )
            .status(403),
        ]);
        let result = fetch(&script.context(&dir, "log-tail", json!({"mode":"list"}))).await;
        script.assert_finished();
        assert_eq!(result["functions"][0]["name"], "synthetic-function");
        assert_eq!(result["functions"][0]["log_group"], "/synthetic/custom");
        assert_eq!(result["partial"], true);
        assert_eq!(result["coverage"]["counts"]["returned"], 1);
        assert!(!result
            .to_string()
            .contains("SYNTHETIC_PRIVATE_FUNCTION_ERROR"));
    }

    #[tokio::test]
    async fn single_event_page_tokens_do_not_claim_more_events_or_complete_stream() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![ExpectedRequest::json(
            "Logs_20140328.GetLogEvents",
            json!({"logGroupName":"/synthetic/group", "logStreamName":"synthetic-stream", "startFromHead":false}),
            json!({"events":[{"timestamp":2,"message":"synthetic event"}], "nextForwardToken":"synthetic-forward", "nextBackwardToken":"synthetic-backward"}),
        )]);
        let result = fetch(&script.context(&dir, "log-tail", json!({"mode":"events", "log_group":"/synthetic/group", "log_stream":"synthetic-stream"}))).await;
        script.assert_finished();
        assert_eq!(result["events"][0]["msg"], "synthetic event");
        assert_eq!(result["coverage"]["completeness"], "unknown");
        assert!(result["coverage"]["has_more"].is_null());
        assert_eq!(crate::request::outcome(&result), "succeeded");
    }

    #[tokio::test]
    async fn stream_and_filter_tokens_report_unvisited_pages_without_more_requests() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                "Logs_20140328.DescribeLogStreams",
                json!({"logGroupName":"/synthetic/group"}),
                json!({"logStreams":[{"logStreamName":"synthetic-stream"}], "nextToken":"synthetic-more"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.FilterLogEvents",
                json!({"logGroupName":"/synthetic/group", "limit":200}),
                json!({"events":[{"timestamp":1,"message":"synthetic filtered event"}], "nextToken":"synthetic-more"}),
            ),
        ]);
        let streams = fetch(&script.context(
            &dir,
            "log-tail",
            json!({"mode":"streams", "log_group":"/synthetic/group"}),
        ))
        .await;
        let events =
            fetch(&script.context(&dir, "log-tail", json!({"log_group":"/synthetic/group"}))).await;
        script.assert_finished();
        assert_eq!(streams["streams"][0]["name"], "synthetic-stream");
        assert_eq!(events["events"][0]["msg"], "synthetic filtered event");
        for result in [streams, events] {
            assert_eq!(result["coverage"]["completeness"], "limited");
            assert_eq!(result["coverage"]["has_more"], true);
        }
    }
}
