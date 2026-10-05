//! Errors by Stack — count ERROR log lines per log group over a window using
//! CloudWatch Logs Insights. The top 20 most-recently-created matching groups
//! are queried through the shared two-slot lifecycle and bounded cleanup.
//! Failed queries never become confirmed zero error counts.

use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::Client;
use futures::future::join_all;
use serde_json::{json, Value};

use super::{
    coverage::Coverage,
    err_msg,
    query::{self, Cleanup, QuerySpec, QueryTiming},
    WidgetCtx,
};

const MAX_GROUPS: usize = 20;
const QUERY_ROW_LIMIT: usize = 100;
const INSIGHTS_QUERY: &str = "fields @timestamp, @message\n| filter @message like /ERROR/\n| stats count() as errors by @logStream";

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let hours = ctx.input_i64("hours", 24).clamp(1, 168);
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
    let resp = match ctx.send("logs", "DescribeLogGroups", req.send()).await {
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

    let fallback;
    let scope = if let Some(scope) = &ctx.runtime.work {
        scope
    } else {
        fallback = ctx.fallback_scope();
        &fallback
    };
    let mut query_ctx = ctx.clone();
    if let Some(token) = ctx.inputs.get("acknowledge_query").and_then(Value::as_str) {
        if ctx
            .inputs
            .get("acknowledge_unknown")
            .and_then(Value::as_bool)
            != Some(true)
            || ctx
                .runtime
                .queries
                .acknowledge_selected(scope, token)
                .is_err()
        {
            let mut outcome =
                query::QueryOutcome::failed("recovery_mismatch", Cleanup::NotNeeded, 0);
            outcome.recovery_pending = ctx.runtime.queries.pending_for(scope);
            return outcome.error_result();
        }
        query_ctx
            .inputs
            .as_object_mut()
            .expect("validated object")
            .remove("acknowledge_query");
        query_ctx.inputs["acknowledge_unknown"] = json!(false);
    }
    let futs = group_names.iter().enumerate().map(|(index, g)| {
        let g = g.clone();
        let mut scoped = query_ctx.clone();
        // Without a selected token, acknowledge at most one same-query retry.
        // The reviewed selector supplies a token for any other pending group.
        if index > 0 {
            scoped.inputs["acknowledge_unknown"] = json!(false);
        }
        async move {
            let total = cw_insights_count(&scoped, &g, start, now).await;
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
    let mut out = errors_chart(hours, results, Some(discovery));
    let fallback;
    let scope = if let Some(scope) = &ctx.runtime.work {
        scope
    } else {
        fallback = ctx.fallback_scope();
        &fallback
    };
    let pending = ctx.runtime.queries.pending_for(scope);
    out["can_acknowledge_unknown"] = json!(!pending.is_empty());
    if !pending.is_empty() {
        out["recovery_required"] = json!(true);
        out["recovery_action"] = json!("select_unknown_query");
    }
    out["recovery_pending"] = json!(pending);
    if out["status"] != "complete" {
        ctx.log("query results incomplete", out["counts"].clone());
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QueryFailure {
    state: &'static str,
    cleanup: Cleanup,
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
    let mut recovery_required = false;
    let mut can_acknowledge_unknown = false;
    let mut cleanup_counts = std::collections::BTreeMap::<&str, usize>::new();
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
                if error.state == "timed_out" {
                    timed_out += 1;
                }
                if error.cleanup.remote_unknown() {
                    remote_status_unknown += 1;
                    recovery_required = true;
                    can_acknowledge_unknown = true;
                }
                if matches!(error.state, "query_capacity_unknown" | "recovery_capacity") {
                    recovery_required = true;
                }
                *cleanup_counts.entry(error.cleanup.status()).or_default() += 1;
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
        "cleanup": { "status": if remote_status_unknown > 0 { "unknown" } else if cleanup_counts.contains_key("stopped") { "stopped" } else { "not_needed" },
            "outcomes":cleanup_counts, "remote_queries_may_still_run":remote_status_unknown > 0, "recovery_required":remote_status_unknown > 0 },
        "recovery_required":recovery_required,
        "can_acknowledge_unknown":can_acknowledge_unknown,
        "recovery_action":if can_acknowledge_unknown {"acknowledge_this_query"} else {"review_outstanding_queries"},
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
            message.push_str(" Some remote queries may still be running. Review the cleanup outcomes before explicitly authorizing replacement queries.");
        }
        if recovery_required && remote_status_unknown == 0 {
            message.push_str(" Outstanding unknown queries occupy the query budget; review those original queries before authorizing more work.");
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
    ctx: &WidgetCtx,
    group: &str,
    start: i64,
    end: i64,
) -> Result<QueryCount, QueryFailure> {
    let spec = QuerySpec {
        group,
        query: INSIGHTS_QUERY,
        start,
        end,
        limit: QUERY_ROW_LIMIT as i32,
    };
    let outcome = query::run(ctx, &spec, QueryTiming::default()).await;
    let Some(response) = outcome.result else {
        return Err(QueryFailure {
            state: outcome.state,
            cleanup: outcome.cleanup,
        });
    };
    let mut total = 0i64;
    for record in response.results() {
        for field in record {
            if field.field() == Some("errors") {
                if let Some(value) = field.value() {
                    total = total.saturating_add(value.trim().parse::<i64>().unwrap_or(0));
                }
            }
        }
    }
    Ok(QueryCount {
        total,
        rows: response.results().len(),
        has_more: response.next_token().is_some_and(|token| !token.is_empty()),
    })
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
                    Err(QueryFailure {
                        state: "start_failed",
                        cleanup: Cleanup::NotNeeded,
                    }),
                ),
                (
                    "SYNTHETIC_PRIVATE_TIMEOUT_MARKER".into(),
                    Err(QueryFailure {
                        state: "timed_out",
                        cleanup: Cleanup::Unknown,
                    }),
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
        assert_eq!(result["cleanup"]["status"], "unknown");
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
                Err(QueryFailure {
                    state: "failed",
                    cleanup: Cleanup::NotNeeded,
                }),
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
                (
                    "synthetic-one".into(),
                    Err(QueryFailure {
                        state: "timed_out",
                        cleanup: Cleanup::NotNeeded,
                    }),
                ),
                (
                    "synthetic-two".into(),
                    Err(QueryFailure {
                        state: "poll_failed",
                        cleanup: Cleanup::Unknown,
                    }),
                ),
                (
                    "synthetic-three".into(),
                    Err(QueryFailure {
                        state: "start_unknown",
                        cleanup: Cleanup::Unknown,
                    }),
                ),
            ],
            None,
        );
        assert_eq!(result["counts"]["timed_out"], 1);
        assert_eq!(result["counts"]["remote_status_unknown"], 2);
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
    }

    #[tokio::test]
    async fn security_extreme_hours_are_clamped_to_the_seven_day_query_budget() {
        // Security Cloud: csf_b4f1dfa21c0bace5bfb5b767.
        for (hours, expected) in [
            (i64::MIN, 1),
            (0, 1),
            (1, 1),
            (168, 168),
            (169, 168),
            (i64::MAX, 168),
        ] {
            let dir = TestDir::new();
            let script = ScriptedHttp::new(vec![ExpectedRequest::json(
                "Logs_20140328.DescribeLogGroups",
                json!({}),
                json!({"logGroups": []}),
            )]);
            let result =
                fetch(&script.context(&dir, "errors-by-stack", json!({"hours": hours}))).await;
            script.assert_finished();
            assert_eq!(result["hours"], expected, "unbounded input: {hours}");
            assert_eq!(result["counts"]["queried"], 0);
            assert_eq!(script.calls(), 1);
        }
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
            assert_eq!(result["cleanup"]["status"], "not_needed");
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
