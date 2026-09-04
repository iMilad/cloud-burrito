//! Errors by Stack — count ERROR log lines per log group over a window using
//! CloudWatch Logs Insights. The top 20 most-recently-created matching groups
//! are queried concurrently. A polling timeout does not stop the remote query;
//! failures must not be presented as confirmed zero error counts.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::types::QueryStatus;
use aws_sdk_cloudwatchlogs::Client;
use futures::future::join_all;
use serde_json::{json, Value};

use super::{coverage::Coverage, err_msg, WidgetCtx};

const MAX_GROUPS: usize = 20;
const QUERY_ROW_LIMIT: usize = 100;
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
            let mut coverage = Coverage::unknown(0);
            coverage.count("pages", 0);
            coverage.limit("groups", Some(MAX_GROUPS));
            coverage.failure(
                "group_discovery_failed",
                "Error-count log groups could not be discovered.",
                false,
            );
            return coverage.attach(
                json!({"render": "errors_chart", "hours": hours, "rows": [], "error": err_msg(e)}),
            );
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

    let mut discovery = Coverage::complete(group_names.len());
    discovery.count("pages", 1);
    discovery.count("scanned", groups.len());
    discovery.limit("results", Some(MAX_GROUPS));
    if resp.next_token().is_some_and(|token| !token.is_empty()) || groups.len() > MAX_GROUPS {
        discovery.has_more(Some(true));
        discovery.limited(
            "group_discovery_limit",
            "Only the first discovery page and up to 20 matching groups were queried.",
        );
    }
    let out = errors_chart(hours, results, Some(discovery));
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

struct QueryCount {
    total: i64,
    rows: usize,
    has_more: bool,
}

#[cfg(test)]
impl From<i64> for QueryCount {
    fn from(total: i64) -> Self {
        Self {
            total,
            rows: usize::from(total > 0),
            has_more: false,
        }
    }
}

fn errors_chart(
    hours: i64,
    results: Vec<(String, Result<QueryCount, QueryFailure>)>,
    discovery: Option<Coverage>,
) -> Value {
    let queried = results.len();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut timed_out = 0usize;
    let mut remote_status_unknown = 0usize;
    let mut rows: Vec<(String, i64, bool)> = Vec::new();
    let mut query_rows = 0;
    let mut bounded_counts = 0;
    let mut unvisited_query_pages = 0;
    for (group, result) in results {
        match result {
            Ok(count) => {
                succeeded += 1;
                query_rows += count.rows;
                let bounded = count.has_more || count.rows >= QUERY_ROW_LIMIT;
                bounded_counts += usize::from(bounded);
                unvisited_query_pages += usize::from(count.has_more);
                if count.total > 0 {
                    rows.push((group, count.total, bounded));
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
        .map(|(stack, errors, bounded)| {
            let mut row = json!({"stack": stack, "errors": errors});
            if bounded {
                row["count_is_lower_bound"] = json!(true);
            }
            row
        })
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
    let mut coverage = Coverage::complete(result["rows"].as_array().map_or(0, Vec::len));
    coverage.count("queried", queried);
    coverage.count("succeeded", succeeded);
    coverage.count("failed", failed);
    coverage.count("query_result_rows", query_rows);
    coverage.count("bounded_counts", bounded_counts);
    coverage.limit("query_result_rows", Some(QUERY_ROW_LIMIT));
    if unvisited_query_pages > 0 {
        coverage.has_more(Some(true));
        coverage.limited(
            "query_result_pages",
            "Some error counts include only the first query result page and are lower bounds.",
        );
    } else if bounded_counts > 0 {
        coverage.has_more(None);
        coverage.unknown_reason(
            "query_row_limit",
            "Some error counts reached the 100-row query limit; complete totals are unknown.",
        );
    }
    if failed > 0 {
        coverage.failure(
            "query_results_failed",
            "Some error-count queries did not produce confirmed results.",
            succeeded > 0,
        );
    }
    if let Some(discovery) = discovery {
        coverage.section("discovery", discovery);
    }
    coverage.attach(result)
}

/// Start an Insights query and poll until Complete, summing the `errors` column.
async fn cw_insights_count(
    client: &Client,
    group: &str,
    start: i64,
    end: i64,
) -> Result<QueryCount, QueryFailure> {
    let started = client
        .start_query()
        .log_group_name(group)
        .start_time(start)
        .end_time(end)
        .query_string(INSIGHTS_QUERY)
        .limit(QUERY_ROW_LIMIT as i32)
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
                return Ok(QueryCount {
                    total,
                    rows: resp.results().len(),
                    has_more: resp.next_token().is_some_and(|token| !token.is_empty()),
                });
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
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    #[test]
    fn partial_results_keep_resource_counts_and_exclude_failed_group_diagnostics() {
        let result = errors_chart(
            24,
            vec![
                ("synthetic-resource-low".into(), Ok(2.into())),
                ("synthetic-resource-high".into(), Ok(7.into())),
                ("synthetic-resource-zero".into(), Ok(0.into())),
                (
                    "SYNTHETIC_PRIVATE_GROUP_MARKER".into(),
                    Err(QueryFailure::StartFailed),
                ),
                (
                    "SYNTHETIC_PRIVATE_TIMEOUT_MARKER".into(),
                    Err(QueryFailure::PollTimeout),
                ),
            ],
            None,
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
        let zero = errors_chart(24, vec![("synthetic-zero".into(), Ok(0.into()))], None);
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
            None,
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
            None,
        );
        assert_eq!(result["counts"]["timed_out"], 1);
        assert_eq!(result["counts"]["remote_status_unknown"], 2);
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
    }

    #[tokio::test]
    async fn actual_discovery_page_limit_does_not_present_account_wide_totals() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                "Logs_20140328.DescribeLogGroups",
                json!({}),
                json!({"logGroups":[{"logGroupName":"/synthetic/errors", "creationTime":1}],"nextToken":"synthetic-unread-groups"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.StartQuery",
                json!({"logGroupName":"/synthetic/errors", "limit":100}),
                json!({"queryId":"synthetic-count"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.GetQueryResults",
                json!({"queryId":"synthetic-count"}),
                json!({"status":"Complete", "results":[[{"field":"errors","value":"7"}]]}),
            ),
        ]);
        let result = fetch(&script.context(&dir, "errors-by-stack", json!({}))).await;
        script.assert_finished();
        assert_eq!(
            result["rows"],
            json!([{"stack":"/synthetic/errors","errors":7}])
        );
        assert_eq!(result["coverage"]["completeness"], "limited");
        assert_eq!(
            result["coverage"]["sections"]["discovery"]["has_more"],
            true
        );
        assert_eq!(result["counts"]["queried"], 1);
        assert_eq!(crate::request::outcome(&result), "succeeded");
    }

    #[tokio::test]
    async fn actual_query_row_threshold_marks_counts_as_lower_bounds() {
        for more in [false, true] {
            let dir = TestDir::new();
            let records: Vec<_> = (0..QUERY_ROW_LIMIT)
                .map(|_| json!([{"field":"errors","value":"1"}]))
                .collect();
            let mut response = json!({"status":"Complete","results":records});
            if more {
                response["nextToken"] = json!("synthetic-more-counts");
            }
            let script = ScriptedHttp::new(vec![
                ExpectedRequest::json(
                    "Logs_20140328.DescribeLogGroups",
                    json!({}),
                    json!({"logGroups":[{"logGroupName":"/synthetic/errors"}]}),
                ),
                ExpectedRequest::json(
                    "Logs_20140328.StartQuery",
                    json!({"limit":100}),
                    json!({"queryId":"synthetic-count"}),
                ),
                ExpectedRequest::json(
                    "Logs_20140328.GetQueryResults",
                    json!({"queryId":"synthetic-count"}),
                    response,
                ),
            ]);
            let result = fetch(&script.context(&dir, "errors-by-stack", json!({}))).await;
            script.assert_finished();
            assert_eq!(result["rows"][0]["errors"], 100);
            assert_eq!(result["rows"][0]["count_is_lower_bound"], true);
            assert_eq!(result["coverage"]["counts"]["bounded_counts"], 1);
            assert_eq!(
                result["coverage"]["completeness"],
                if more { "limited" } else { "unknown" }
            );
            assert_eq!(result["cleanup"]["status"], "not_attempted");
        }
    }

    #[tokio::test]
    async fn concurrent_group_failure_keeps_successful_error_count_and_safe_failure_summary() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json("Logs_20140328.DescribeLogGroups", json!({}), json!({"logGroups":[{"logGroupName":"/synthetic/good"},{"logGroupName":"/synthetic/bad"}]})),
            ExpectedRequest::json("Logs_20140328.StartQuery", json!({"logGroupName":"/synthetic/good"}), json!({"queryId":"synthetic-good"})),
            ExpectedRequest::json("Logs_20140328.StartQuery", json!({"logGroupName":"/synthetic/bad"}), json!({"__type":"AccessDeniedException","message":"SYNTHETIC_PRIVATE_COUNT_ERROR"})).status(400),
            ExpectedRequest::json("Logs_20140328.GetQueryResults", json!({"queryId":"synthetic-good"}), json!({"status":"Complete","results":[[{"field":"errors","value":"3"}]]})),
        ]).unordered();
        let result = fetch(&script.context(&dir, "errors-by-stack", json!({}))).await;
        script.assert_finished();
        assert_eq!(
            result["rows"],
            json!([{"stack":"/synthetic/good","errors":3}])
        );
        assert_eq!(result["partial"], true);
        assert_eq!(result["counts"]["failed"], 1);
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE_COUNT_ERROR"));
    }
}
