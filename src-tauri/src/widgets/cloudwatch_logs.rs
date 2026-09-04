//! CloudWatch Logs — search log groups, browse their streams, view events.
//!
//! Streams and events reuse the Lambda Logs implementation: those modes are
//! log-group-generic and never cared about Lambda.

use serde_json::{json, Value};

use super::{coverage::Coverage, err_msg, log_tail, WidgetCtx};

const MAX_GROUPS: usize = 1_000;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    match ctx.input_str("mode", "groups").as_str() {
        "groups" => fetch_groups(ctx).await,
        "streams" => log_tail::fetch_streams(ctx).await,
        "events" => log_tail::fetch_stream_events(ctx).await,
        other => json!({"ok": false, "error": format!("unknown mode: {other}")}),
    }
}

pub(super) async fn fetch_groups(ctx: &WidgetCtx) -> Value {
    let max_groups = ctx.input_i64("max_groups", 500).clamp(1, MAX_GROUPS as i64) as usize;
    // Optional server-side narrowing (case-insensitive substring match) for
    // accounts with more groups than the cap.
    let pattern = ctx.input_str("name_pattern", "");

    if let Some(denied) = ctx.preflight("logs", "DescribeLogGroups") {
        return denied;
    }
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);

    let mut groups: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    let mut pages = 0;
    let mut failure = None;
    let mut locally_omitted = false;
    loop {
        let remaining = max_groups.saturating_sub(groups.len());
        if remaining == 0 {
            break;
        }
        let mut req = client.describe_log_groups().limit(remaining.min(50) as i32);
        if !pattern.is_empty() {
            req = req.log_group_name_pattern(&pattern);
        }
        if let Some(t) = &token {
            req = req.next_token(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                failure = Some(err_msg(e));
                break;
            }
        };
        pages += 1;
        for (index, g) in resp.log_groups().iter().enumerate() {
            let name = g.log_group_name().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            groups.push(json!({
                "name": name,
                "arn": g.arn().unwrap_or(""),
                "creation_time": g.creation_time().unwrap_or(0),
                "retention_days": g.retention_in_days().unwrap_or(0),
                "stored_bytes": g.stored_bytes().unwrap_or(0),
            }));
            if groups.len() >= max_groups {
                locally_omitted = index + 1 < resp.log_groups().len();
                break;
            }
        }
        token = resp
            .next_token()
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        if token.is_none() {
            break;
        }
    }

    groups.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_ascii_lowercase())
    });
    let capped = locally_omitted || (groups.len() >= max_groups && token.is_some());
    let mut coverage = Coverage::complete(groups.len());
    coverage.count("pages", pages);
    coverage.limit("results", Some(max_groups));
    if capped {
        coverage.has_more(Some(true));
        coverage.limited(
            "group_limit",
            "The requested log group limit omitted further results.",
        );
    }
    if failure.is_some() {
        coverage.failure(
            "group_page_failed",
            "A log group page could not be loaded; retained groups are incomplete.",
            !groups.is_empty(),
        );
    }
    let mut result = json!({
        "ok": failure.is_none(),
        "account_id": ctx.account_id,
        "region": ctx.region,
        "groups": groups,
        "capped": capped,
    });
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

    #[tokio::test]
    async fn later_group_page_failure_retains_discovered_groups() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                "Logs_20140328.DescribeLogGroups",
                json!({"nextToken":null}),
                json!({"logGroups":[{"logGroupName":"/synthetic/group"}],"nextToken":"synthetic-next"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.DescribeLogGroups",
                json!({"nextToken":"synthetic-next"}),
                json!({"__type":"AccessDeniedException","message":"SYNTHETIC_PRIVATE_GROUP_ERROR"}),
            )
            .status(400),
        ]);
        let result =
            fetch(&script.context(&dir, "cloudwatch-logs", json!({"mode":"groups"}))).await;
        script.assert_finished();
        assert_eq!(result["groups"][0]["name"], "/synthetic/group");
        assert_eq!(result["partial"], true);
        assert_eq!(result["coverage"]["counts"]["returned"], 1);
        assert_eq!(result["coverage"]["counts"]["pages"], 1);
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE_GROUP_ERROR"));
    }

    #[tokio::test]
    async fn exact_limit_is_complete_only_when_service_has_no_remaining_page() {
        for next in [None, Some("synthetic-next")] {
            let dir = TestDir::new();
            let mut response = json!({"logGroups":[{"logGroupName":"/synthetic/group"}]});
            if let Some(next) = next {
                response["nextToken"] = json!(next);
            }
            let script = ScriptedHttp::new(vec![ExpectedRequest::json(
                "Logs_20140328.DescribeLogGroups",
                json!({"limit":1}),
                response,
            )]);
            let result =
                fetch(&script.context(&dir, "cloudwatch-logs", json!({"max_groups":1}))).await;
            script.assert_finished();
            assert_eq!(result["capped"], next.is_some());
            assert_eq!(
                result["coverage"]["completeness"],
                if next.is_some() {
                    "limited"
                } else {
                    "complete"
                }
            );
            assert_eq!(crate::request::outcome(&result), "succeeded");
        }
    }
}
