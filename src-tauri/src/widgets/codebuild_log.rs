//! CodeBuild Log — CloudWatch log lines for one CodeBuild build, shown inline
//! under a pipeline action. Read-only (BatchGetBuilds + GetLogEvents).

use serde_json::{json, Value};

use super::handoff::{self, Source};
use super::{coverage::Coverage, err_msg, WidgetCtx};

const PAGE_LIMIT: i32 = 10_000; // events per GetLogEvents call (API cap ~10k/1MB)
const MAX_EVENTS: usize = 10_000; // stop after a whole page reaches this threshold

fn with_handoffs(mut result: Value) -> Value {
    let group = result["log_group"].as_str().unwrap_or("");
    let stream = result["log_stream"].as_str().unwrap_or("");
    let logs = if group.is_empty() || stream.is_empty() {
        handoff::unavailable(Source::IdentifierUnavailable)
    } else {
        handoff::logs(group, Some(stream), Source::CodebuildLogs)
    };
    result["handoffs"] = json!({"logs":logs,"stack":handoff::manual_stack()});
    result
}

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
        return with_handoffs(denied);
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
            return with_handoffs(coverage.attach(json!({"render": "log_stream", "build_id":build_id, "log_group": "", "events": [], "error": err_msg(e)})));
        }
    };
    let build = builds.builds().iter().find(|build| {
        (build.id() == Some(build_id.as_str()) || build.arn() == Some(build_id.as_str()))
            && build.arn().is_none_or(|arn| {
                handoff::valid_build(arn, ctx)
                    && build.id().is_none_or(|id| {
                        arn.splitn(6, ':')
                            .nth(5)
                            .and_then(|resource| resource.strip_prefix("build/"))
                            == Some(id)
                    })
            })
    });
    let Some(build) = build else {
        let mut coverage = Coverage::unknown(0);
        coverage.failure(
            "build_not_returned",
            "The requested build was not returned by AWS.",
            false,
        );
        return with_handoffs(coverage.attach(json!({"render":"log_stream", "build_id":build_id, "log_group":"", "log_stream":"", "events":[], "error":"The requested build was not returned by AWS."})));
    };
    let logs = build.logs();
    let group = logs.and_then(|l| l.group_name()).unwrap_or("").to_string();
    let stream = logs.and_then(|l| l.stream_name()).unwrap_or("").to_string();
    if logs
        .and_then(|logs| logs.cloud_watch_logs_arn())
        .is_some_and(|arn| {
            !handoff::same_context_arn(arn, "logs", "log-group:", ctx)
                || arn.splitn(6, ':').nth(5)
                    != Some(format!("log-group:{group}:log-stream:{stream}").as_str())
        })
    {
        let mut coverage = Coverage::unknown(0);
        coverage.failure(
            "log_context_mismatch",
            "The build did not establish a log stream in the current context.",
            false,
        );
        let mut result = with_handoffs(coverage.attach(json!({"render":"log_stream","build_id":build_id,"log_group":"","log_stream":"","events":[],"error":"The build did not establish a log stream in the current context."})));
        result["handoffs"]["logs"] = handoff::unavailable(Source::ContextMismatch);
        return result;
    }
    if handoff::logs(&group, Some(&stream), Source::CodebuildLogs)["status"] != "available" {
        let mut coverage = Coverage::unknown(0);
        coverage.unknown_reason(
            "cloudwatch_logs_unavailable",
            "The build has no available CloudWatch log group and stream.",
        );
        return with_handoffs(coverage.attach(json!({
            "render": "log_stream", "build_id":build_id, "log_group": group, "log_stream":stream, "events": [],
            "error": "No CloudWatch logs for this build (it may use S3 logs, or logs are disabled)."
        })));
    }

    if let Some(mut denied) = ctx.preflight("logs", "GetLogEvents") {
        denied["build_id"] = json!(build_id);
        denied["log_group"] = json!(group);
        denied["log_stream"] = json!(stream);
        return with_handoffs(denied);
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
    with_handoffs(coverage.attach(result))
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
        assert_eq!(
            result["handoffs"]["logs"]["inputs"],
            json!({"mode":"events","log_group":"/synthetic/build","log_stream":"synthetic-stream"})
        );
        assert_eq!(result["handoffs"]["stack"]["status"], "manual");
        assert_eq!(
            result["handoffs"]["stack"]["source"],
            "relationship_unknown"
        );
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
        assert_eq!(result["handoffs"]["logs"]["status"], "unavailable");
    }

    #[tokio::test]
    async fn matching_build_id_with_foreign_arn_never_fetches_or_offers_logs() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![ExpectedRequest::json(
            "CodeBuild_20161006.BatchGetBuilds",
            json!({"ids":["synthetic-build:run"]}),
            json!({"builds":[{"id":"synthetic-build:run","arn":"arn:aws:codebuild:eu-west-1:acct-other-fixture:build/synthetic-build:run","logs":{"groupName":"/synthetic/foreign","streamName":"synthetic-foreign-stream"}}]}),
        )]);
        let result = fetch(&script.context(
            &dir,
            "codebuild-log",
            json!({"build_id":"synthetic-build:run"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(script.calls(), 1);
        assert_eq!(result["handoffs"]["logs"]["status"], "unavailable");
        assert!(!result.to_string().contains("synthetic-foreign-stream"));
    }

    #[tokio::test]
    async fn contradictory_cloudwatch_log_arns_block_reads_and_navigation() {
        for arn in [
            "arn:aws:logs:eu-west-1:acct-producer-fixture:log-group:/synthetic/build:log-stream:synthetic-stream",
            "arn:aws:logs:us-east-1:acct-other-fixture:log-group:/synthetic/build:log-stream:synthetic-stream",
            "arn:aws:logs:us-east-1:acct-producer-fixture:log-group:/synthetic/other:log-stream:synthetic-stream",
        ] {
            let dir = TestDir::new();
            let script = ScriptedHttp::new(vec![ExpectedRequest::json(
                "CodeBuild_20161006.BatchGetBuilds",json!({"ids":["synthetic-build:run"]}),
                json!({"builds":[{"id":"synthetic-build:run","logs":{"groupName":"/synthetic/build","streamName":"synthetic-stream","cloudWatchLogsArn":arn}}]}),
            )]);
            let result = fetch(&script.context(&dir,"codebuild-log",json!({"build_id":"synthetic-build:run"}))).await;
            script.assert_finished();
            assert_eq!(script.calls(),1);
            assert_eq!(result["handoffs"]["logs"]["status"],"unavailable");
            assert_eq!(result["handoffs"]["logs"]["source"],"context_mismatch");
            assert_eq!(result["log_group"],"");
        }
    }
}
