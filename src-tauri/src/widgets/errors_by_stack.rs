//! Errors by Stack — count ERROR log lines per log group over a window using
//! CloudWatch Logs Insights. The top 20 most-recently-created matching groups
//! are queried concurrently. A polling timeout does not stop the remote query;
//! failures must not be presented as confirmed zero error counts.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::types::QueryStatus;
use aws_sdk_cloudwatchlogs::Client;
use futures::future::join_all;
use serde_json::{json, Value};

use super::{err_msg, WidgetCtx};

const MAX_GROUPS: usize = 20;
const INSIGHTS_QUERY: &str = "fields @timestamp, @message\n| filter @message like /ERROR/\n| stats count() as errors by @logStream";

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let hours = ctx.input_i64("hours", 24).max(1);
    let pattern = ctx.input_str("log_group_pattern", "/aws/lambda/");

    // Authorize the complete workflow before discovery or SDK construction.
    // StartQuery also requires the local StopQuery cleanup capability.
    for operation in ["DescribeLogGroups", "StartQuery", "GetQueryResults"] {
        if let Some(denied) = ctx.preflight("logs", operation) {
            return denied;
        }
    }
    let client = Client::new(&ctx.sdk);
    let mut req = client.describe_log_groups();
    if !pattern.is_empty() {
        req = req.log_group_name_prefix(&pattern);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return json!({"render": "errors_chart", "hours": hours, "rows": [], "error": err_msg(e)});
        }
    };

    let mut groups = resp.log_groups().to_vec();
    groups.sort_by(|a, b| {
        b.creation_time()
            .unwrap_or(0)
            .cmp(&a.creation_time().unwrap_or(0))
    });
    let group_names: Vec<String> = groups
        .iter()
        .take(MAX_GROUPS)
        .filter_map(|g| g.log_group_name().map(str::to_string))
        .collect();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let start = now - hours * 3600;

    let futs = group_names.iter().map(|g| {
        let client = client.clone();
        let g = g.clone();
        async move {
            let total = cw_insights_count(&client, &g, start, now).await;
            (g, total)
        }
    });
    let results = join_all(futs).await;

    let out = errors_chart(hours, results);
    if out["status"] != "complete" {
        ctx.log("query results incomplete", out["counts"].clone());
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryFailure {
    StartFailed,
    MissingQueryId,
    PollFailed,
    Failed,
    Cancelled,
    RemoteTimeout,
    PollTimeout,
}

fn errors_chart(hours: i64, results: Vec<(String, Result<i64, QueryFailure>)>) -> Value {
    let queried = results.len();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut timed_out = 0usize;
    let mut remote_status_unknown = 0usize;
    let mut rows: Vec<(String, i64)> = Vec::new();
    for (group, result) in results {
        match result {
            Ok(total) => {
                succeeded += 1;
                if total > 0 {
                    rows.push((group, total));
                }
            }
            Err(error) => {
                failed += 1;
                if matches!(
                    error,
                    QueryFailure::RemoteTimeout | QueryFailure::PollTimeout
                ) {
                    timed_out += 1;
                }
                if matches!(
                    error,
                    QueryFailure::MissingQueryId
                        | QueryFailure::PollFailed
                        | QueryFailure::PollTimeout
                ) {
                    remote_status_unknown += 1;
                }
            }
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(stack, errors)| json!({"stack": stack, "errors": errors}))
        .collect();

    let mut result = json!({
        "render": "errors_chart", "hours": hours, "rows": out,
        "ok": failed == 0,
        "status": if failed == 0 { "complete" } else if succeeded > 0 { "partial" } else { "failed" },
        "partial": failed > 0 && succeeded > 0,
        "counts": { "queried": queried, "succeeded": succeeded, "failed": failed, "timed_out": timed_out, "remote_status_unknown": remote_status_unknown },
        "cleanup": { "status": "not_attempted", "remote_queries_may_still_run": remote_status_unknown > 0 },
    });
    if failed > 0 {
        result["error_type"] = json!(if succeeded > 0 {
            "PartialFailure"
        } else {
            "QueryFailed"
        });
        let mut message = if succeeded > 0 {
            "Some query results are unavailable; displayed error counts are incomplete."
        } else {
            "No query results were confirmed; error counts are unavailable."
        }
        .to_string();
        if remote_status_unknown > 0 {
            message.push_str(" Some remote queries may still be running; no stop was attempted.");
        }
        result["error"] = json!(message);
    }
    result
}

/// Start an Insights query and poll until Complete, summing the `errors` column.
async fn cw_insights_count(
    client: &Client,
    group: &str,
    start: i64,
    end: i64,
) -> Result<i64, QueryFailure> {
    let started = client
        .start_query()
        .log_group_name(group)
        .start_time(start)
        .end_time(end)
        .query_string(INSIGHTS_QUERY)
        .limit(100)
        .send()
        .await
        .map_err(|_| QueryFailure::StartFailed)?;
    let query_id = started
        .query_id()
        .ok_or(QueryFailure::MissingQueryId)?
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let resp = client
            .get_query_results()
            .query_id(&query_id)
            .send()
            .await
            .map_err(|_| QueryFailure::PollFailed)?;
        match resp.status() {
            Some(QueryStatus::Complete) => {
                let mut total = 0i64;
                for record in resp.results() {
                    for field in record {
                        if field.field() == Some("errors") {
                            if let Some(v) = field.value() {
                                total += v.trim().parse::<i64>().unwrap_or(0);
                            }
                        }
                    }
                }
                return Ok(total);
            }
            Some(QueryStatus::Failed) => return Err(QueryFailure::Failed),
            Some(QueryStatus::Cancelled) => return Err(QueryFailure::Cancelled),
            Some(QueryStatus::Timeout) => return Err(QueryFailure::RemoteTimeout),
            _ => {
                if Instant::now() >= deadline {
                    return Err(QueryFailure::PollTimeout);
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_results_keep_resource_counts_and_exclude_failed_group_diagnostics() {
        let result = errors_chart(
            24,
            vec![
                ("synthetic-resource-low".into(), Ok(2)),
                ("synthetic-resource-high".into(), Ok(7)),
                ("synthetic-resource-zero".into(), Ok(0)),
                (
                    "SYNTHETIC_PRIVATE_GROUP_MARKER".into(),
                    Err(QueryFailure::StartFailed),
                ),
                (
                    "SYNTHETIC_PRIVATE_TIMEOUT_MARKER".into(),
                    Err(QueryFailure::PollTimeout),
                ),
            ],
        );
        assert_eq!(result["status"], "partial");
        assert_eq!(result["partial"], true);
        assert_eq!(result["ok"], false);
        assert_eq!(result["error_type"], "PartialFailure");
        assert_eq!(
            result["counts"],
            json!({"queried": 5, "succeeded": 3, "failed": 2, "timed_out": 1, "remote_status_unknown": 1})
        );
        assert_eq!(
            result["rows"],
            json!([
                {"stack": "synthetic-resource-high", "errors": 7},
                {"stack": "synthetic-resource-low", "errors": 2},
            ])
        );
        assert_eq!(result["cleanup"]["status"], "not_attempted");
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("may still be running"));
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE"));
    }

    #[test]
    fn complete_zero_results_and_total_failure_are_distinct() {
        let zero = errors_chart(24, vec![("synthetic-zero".into(), Ok(0))]);
        assert_eq!(zero["status"], "complete");
        assert_eq!(zero["ok"], true);
        assert_eq!(zero["rows"], json!([]));
        assert!(zero.get("error").is_none());

        let failed = errors_chart(
            24,
            vec![(
                "SYNTHETIC_PRIVATE_GROUP_MARKER".into(),
                Err(QueryFailure::Failed),
            )],
        );
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["partial"], false);
        assert_eq!(failed["rows"], json!([]));
        assert_eq!(failed["counts"]["succeeded"], 0);
        assert_eq!(failed["cleanup"]["remote_queries_may_still_run"], false);
        assert!(failed["error"].as_str().unwrap().contains("unavailable"));
        assert!(!failed.to_string().contains("SYNTHETIC_PRIVATE"));
    }

    #[test]
    fn service_timeout_and_unknown_remote_state_are_counted_separately() {
        let result = errors_chart(
            24,
            vec![
                ("synthetic-one".into(), Err(QueryFailure::RemoteTimeout)),
                ("synthetic-two".into(), Err(QueryFailure::PollFailed)),
                ("synthetic-three".into(), Err(QueryFailure::MissingQueryId)),
            ],
        );
        assert_eq!(result["counts"]["timed_out"], 1);
        assert_eq!(result["counts"]["remote_status_unknown"], 2);
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
    }
}
