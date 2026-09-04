//! CodeBuild Log — CloudWatch log lines for one CodeBuild build, shown inline
//! under a pipeline action. Read-only (BatchGetBuilds + GetLogEvents).

use serde_json::{json, Value};

use super::{coverage::Coverage, err_msg, WidgetCtx};

const PAGE_LIMIT: i32 = 10_000; // events per GetLogEvents call (API cap ~10k/1MB)
const MAX_EVENTS: usize = 10_000; // stop after a whole page reaches this threshold

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

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let build_id = ctx.input_str("build_id", "");
    if build_id.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "build_id is required"}});
    }

    if let Some(denied) = ctx.preflight("codebuild", "BatchGetBuilds") {
        return denied;
    }
    let cb = aws_sdk_codebuild::Client::new(&ctx.sdk);
    let builds = match cb.batch_get_builds().ids(&build_id).send().await {
        Ok(r) => r,
        Err(e) => {
            let mut coverage = Coverage::unknown(0);
            coverage.failure(
                "build_lookup_failed",
                "Build log metadata could not be loaded.",
                false,
            );
            return coverage.attach(json!({"render": "log_stream", "build_id":build_id, "log_group": "", "events": [], "error": err_msg(e)}));
        }
    };
    let build = builds.builds().iter().find(|build| {
        build.id() == Some(build_id.as_str()) || build.arn() == Some(build_id.as_str())
    });
    let Some(build) = build else {
        let mut coverage = Coverage::unknown(0);
        coverage.failure(
            "build_not_returned",
            "The requested build was not returned by AWS.",
            false,
        );
        return coverage.attach(json!({"render":"log_stream", "build_id":build_id, "log_group":"", "log_stream":"", "events":[], "error":"The requested build was not returned by AWS."}));
    };
    let logs = build.logs();
    let group = logs.and_then(|l| l.group_name()).unwrap_or("").to_string();
    let stream = logs.and_then(|l| l.stream_name()).unwrap_or("").to_string();
    if group.is_empty() || stream.is_empty() {
        let mut coverage = Coverage::unknown(0);
        coverage.unknown_reason(
            "cloudwatch_logs_unavailable",
            "The build has no available CloudWatch log group and stream.",
        );
        return coverage.attach(json!({
            "render": "log_stream", "build_id":build_id, "log_group": group, "log_stream":stream, "events": [],
            "error": "No CloudWatch logs for this build (it may use S3 logs, or logs are disabled)."
        }));
    }

    if let Some(mut denied) = ctx.preflight("logs", "GetLogEvents") {
        denied["build_id"] = json!(build_id);
        denied["log_group"] = json!(group);
        denied["log_stream"] = json!(stream);
        return denied;
    }
    let cw = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);

    // Retain the current page-stop threshold: a whole final page can exceed it.
    // Repeated nextForwardToken confirms the end observed by these requests.
    let mut events: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    let mut truncated = false;
    let mut pages = 0;
    let mut failure = None;
    loop {
        let mut req = cw
            .get_log_events()
            .log_group_name(&group)
            .log_stream_name(&stream)
            .limit(PAGE_LIMIT)
            .start_from_head(true);
        if let Some(t) = &token {
            req = req.next_token(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                failure = Some(err_msg(e));
                truncated = !events.is_empty();
                break;
            }
        };
        pages += 1;
        for ev in resp.events() {
            let ts = ev.timestamp().unwrap_or(0);
            let msg = ev.message().unwrap_or("").to_string();
            let level = level_for(&msg);
            events.push(json!({"ts": ts, "msg": msg, "level": level}));
        }
        let next = resp.next_forward_token().map(str::to_string);
        if next.is_none() || next == token {
            break; // reached the end of the stream
        }
        token = next;
        if events.len() >= MAX_EVENTS {
            truncated = true;
            break;
        }
    }

    let mut coverage = Coverage::complete(events.len());
    coverage.count("pages", pages);
    coverage.limit("stop_after_events", Some(MAX_EVENTS));
    if failure.is_some() {
        coverage.failure(
            "log_page_failed",
            "A log page could not be loaded; retained events are incomplete.",
            !events.is_empty(),
        );
    } else if truncated {
        coverage.has_more(None);
        coverage.limited("event_stop_threshold", "Reading stopped after the event threshold; the final page can exceed that threshold and more events may exist.");
    }

    let mut result = json!({"render": "log_stream", "build_id":build_id, "log_group": group, "log_stream":stream, "events": events});
    if let Some(error) = failure {
        result["error"] = json!(error);
    }
    coverage.attach(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    fn build_response() -> ExpectedRequest {
        ExpectedRequest::json(
            "CodeBuild_20161006.BatchGetBuilds",
            json!({"ids":["synthetic-build:run"]}),
            json!({"builds":[{"id":"synthetic-build:run", "logs":{"groupName":"/synthetic/build", "streamName":"synthetic-stream"}}]}),
        )
    }

    fn page(token: Value, response: Value) -> ExpectedRequest {
        ExpectedRequest::json(
            "Logs_20140328.GetLogEvents",
            json!({"logGroupName":"/synthetic/build", "logStreamName":"synthetic-stream", "startFromHead":true, "nextToken":token}),
            response,
        )
    }

    #[tokio::test]
    async fn later_page_failure_retains_only_real_events_and_reports_partial_failure() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![build_response(),
            page(Value::Null, json!({"events":[{"timestamp":1,"message":"synthetic first event"}],"nextForwardToken":"synthetic-next"})),
            page(json!("synthetic-next"), json!({"__type":"AccessDeniedException","message":"SYNTHETIC_PRIVATE_SERVICE_MARKER"})).status(400),
        ]);
        let result = fetch(&script.context(
            &dir,
            "codebuild-log",
            json!({"build_id":"synthetic-build:run"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(
            result["events"],
            json!([{"ts":1,"msg":"synthetic first event","level":"info"}])
        );
        assert_eq!(result["build_id"], "synthetic-build:run");
        assert_eq!(result["log_stream"], "synthetic-stream");
        assert_eq!(result["partial"], true);
        assert_eq!(result["error_type"], "PartialFailure");
        assert_eq!(result["coverage"]["counts"]["returned"], 1);
        assert_eq!(result["coverage"]["counts"]["pages"], 1);
        assert!(!result
            .to_string()
            .contains("SYNTHETIC_PRIVATE_SERVICE_MARKER"));
    }

    #[tokio::test]
    async fn whole_final_page_overshoot_is_reported_as_threshold_not_hard_cap() {
        let dir = TestDir::new();
        let events: Vec<_> = (0..9_999)
            .map(|i| json!({"timestamp":i,"message":"synthetic event"}))
            .collect();
        let script = ScriptedHttp::new(vec![
            build_response(),
            page(
                Value::Null,
                json!({"events":events,"nextForwardToken":"synthetic-next"}),
            ),
            page(
                json!("synthetic-next"),
                json!({"events":[{"message":"synthetic a"},{"message":"synthetic b"},{"message":"synthetic c"}],"nextForwardToken":"synthetic-final"}),
            ),
        ]);
        let result = fetch(&script.context(
            &dir,
            "codebuild-log",
            json!({"build_id":"synthetic-build:run"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(script.calls(), 3);
        assert_eq!(result["events"].as_array().unwrap().len(), 10_002);
        assert_eq!(result["coverage"]["counts"]["returned"], 10_002);
        assert_eq!(result["coverage"]["limits"]["stop_after_events"], 10_000);
        assert_eq!(result["coverage"]["completeness"], "limited");
        assert!(result["coverage"]["has_more"].is_null());
        assert_eq!(crate::request::outcome(&result), "succeeded");
    }

    #[tokio::test]
    async fn repeated_forward_token_confirms_observed_end_without_warning_events() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            build_response(),
            page(
                Value::Null,
                json!({"events":[{"message":"synthetic event"}],"nextForwardToken":"synthetic-end"}),
            ),
            page(
                json!("synthetic-end"),
                json!({"events":[],"nextForwardToken":"synthetic-end"}),
            ),
        ]);
        let result = fetch(&script.context(
            &dir,
            "codebuild-log",
            json!({"build_id":"synthetic-build:run"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(result["events"].as_array().unwrap().len(), 1);
        assert_eq!(result["coverage"]["completeness"], "complete");
        assert_eq!(result["coverage"]["has_more"], false);
        assert_eq!(result["coverage"]["counts"]["pages"], 2);
    }

    #[tokio::test]
    async fn unrelated_build_metadata_never_selects_a_log_stream() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![ExpectedRequest::json(
            "CodeBuild_20161006.BatchGetBuilds",
            json!({"ids":["synthetic-build:run"]}),
            json!({"builds":[{"id":"synthetic-other:run","logs":{"groupName":"/synthetic/unrelated","streamName":"unrelated-stream"}}]}),
        )]);
        let result = fetch(&script.context(
            &dir,
            "codebuild-log",
            json!({"build_id":"synthetic-build:run"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(script.calls(), 1);
        assert_eq!(result["ok"], false);
        assert_eq!(result["log_group"], "");
        assert!(!result.to_string().contains("unrelated-stream"));
    }
}
