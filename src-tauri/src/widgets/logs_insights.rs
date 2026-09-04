//! Logs Insights Query — run a user-written CloudWatch Logs Insights query
//! against one log group and render the result rows as a table.
//!
//! The query text is the user's own; genericity comes for free. StartQuery /
//! GetQueryResults follow the polling pattern established by errors_by_stack.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::operation::stop_query::StopQueryOutput;
use aws_sdk_cloudwatchlogs::types::QueryStatus;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use serde_json::{json, Value};

use super::{cloudwatch_logs, coverage::Coverage, err_msg, WidgetCtx};

/// Insights queries cost by data scanned — cap the window at 7 days.
const MAX_RANGE_SECONDS: i64 = 7 * 86_400;
/// How long we poll before stopping the query and telling the user to narrow it.
const POLL_BUDGET: Duration = Duration::from_secs(25);

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    match ctx.input_str("mode", "query").as_str() {
        "groups" => cloudwatch_logs::fetch_groups(ctx).await,
        "query" => run_query(ctx).await,
        _ => json!({"ok": false, "error": "Unsupported query mode."}),
    }
}

async fn run_query(ctx: &WidgetCtx) -> Value {
    let log_group = ctx.input_str("log_group", "");
    if log_group.is_empty() {
        return json!({"ok": false, "error": "log_group is required"});
    }
    let query = ctx.input_str("query", "");
    if query.trim().is_empty() {
        return json!({"ok": false, "error": "query is required"});
    }
    let range = ctx
        .input_i64("range_seconds", 3600)
        .clamp(60, MAX_RANGE_SECONDS);

    if let Some(denied) = ctx.preflight("logs", "StartQuery") {
        return denied;
    }
    if let Some(denied) = ctx.preflight("logs", "GetQueryResults") {
        return denied;
    }
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let started = match client
        .start_query()
        .log_group_name(&log_group)
        .start_time(now - range)
        .end_time(now)
        .query_string(&query)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": err_msg(e)}),
    };
    let Some(query_id) = started.query_id().map(str::to_string) else {
        return unknown_query_failure(
            "AWS did not return a query identifier; the remote query state is unknown.",
        );
    };

    let deadline = Instant::now() + POLL_BUDGET;
    loop {
        let resp = match client.get_query_results().query_id(&query_id).send().await {
            Ok(r) => r,
            Err(e) => return unknown_query_failure(&err_msg(e)),
        };
        match resp.status() {
            Some(QueryStatus::Complete) => {
                let rows: Vec<Vec<(String, String)>> = resp
                    .results()
                    .iter()
                    .map(|record| {
                        record
                            .iter()
                            .map(|f| {
                                (
                                    f.field().unwrap_or("").to_string(),
                                    f.value().unwrap_or("").to_string(),
                                )
                            })
                            .collect()
                    })
                    .collect();
                let mut out = results_table(&rows);
                if let Some(stats) = resp.statistics() {
                    out["stats"] = json!({
                        "records_matched": stats.records_matched(),
                        "records_scanned": stats.records_scanned(),
                        "bytes_scanned": stats.bytes_scanned(),
                    });
                }
                out["account_id"] = json!(ctx.account_id);
                out["region"] = json!(ctx.region);
                let mut coverage = Coverage::complete(rows.len());
                coverage.count("pages", 1);
                coverage.limit("results", None);
                if resp.next_token().is_some_and(|token| !token.is_empty()) {
                    coverage.has_more(Some(true));
                    coverage.limited(
                        "query_result_page",
                        "The query completed, but only the first result page was loaded.",
                    );
                }
                return coverage.attach(out);
            }
            Some(QueryStatus::Failed) => {
                return json!({"ok": false, "error_type": "QueryFailed", "error": "The query failed."})
            }
            Some(QueryStatus::Cancelled) => {
                return json!({"ok": false, "error_type": "QueryCancelled", "error": "The query was cancelled."})
            }
            Some(QueryStatus::Timeout) => {
                return json!({"ok": false, "error_type": "QueryTimeout", "error": "AWS reported that the query timed out."})
            }
            _ => {
                if Instant::now() >= deadline {
                    let cleanup = stop_query_best_effort(ctx, &client, &query_id).await;
                    return timeout_result(cleanup);
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
        }
    }
}

fn unknown_query_failure(message: &str) -> Value {
    let mut coverage = Coverage::unknown(0);
    coverage.count("pages", 0);
    coverage.limit("results", None);
    coverage.failure("query_state_unknown", "Query results could not be confirmed; the remote query may still be running and no stop was attempted.", false);
    coverage.attach(json!({"ok":false, "error":message,
        "cleanup":{"status":"not_attempted", "remote_stop_confirmed":false, "remote_queries_may_still_run":true}}))
}

/// Attempt to stop a timed-out query. Startup requires the local cleanup
/// capability; recheck here before sending. AWS can still reject the request.
async fn stop_query_best_effort(
    ctx: &WidgetCtx,
    client: &aws_sdk_cloudwatchlogs::Client,
    query_id: &str,
) -> QueryCleanup {
    let outcome = if ctx.preflight("logs", "StopQuery").is_some() {
        QueryCleanup::Denied
    } else {
        cleanup_result(client.stop_query().query_id(query_id).send().await)
    };
    ctx.log("query cleanup", json!({"cleanup_status": outcome.status()}));
    outcome
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryCleanup {
    Stopped,
    NotConfirmed,
    Denied,
    Failed,
}

impl QueryCleanup {
    fn status(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::NotConfirmed => "not_confirmed",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

fn cleanup_result<E: ProvideErrorMetadata>(result: Result<StopQueryOutput, E>) -> QueryCleanup {
    match result {
        Ok(output) if output.success() => QueryCleanup::Stopped,
        Ok(_) => QueryCleanup::NotConfirmed,
        Err(error)
            if matches!(
                error.code(),
                Some(
                    "AccessDenied"
                        | "AccessDeniedException"
                        | "UnauthorizedException"
                        | "UnauthorizedOperation"
                )
            ) =>
        {
            QueryCleanup::Denied
        }
        Err(_) => QueryCleanup::Failed,
    }
}

fn timeout_result(cleanup: QueryCleanup) -> Value {
    let message = match cleanup {
        QueryCleanup::Stopped => "Query polling reached its time limit. AWS confirmed that the query was stopped. Narrow the time range or query before retrying.",
        QueryCleanup::NotConfirmed => "Query polling reached its time limit. AWS did not confirm that the query was stopped; it may still be running.",
        QueryCleanup::Denied => "Query polling reached its time limit. The stop request was denied; the query may still be running.",
        QueryCleanup::Failed => "Query polling reached its time limit. The stop request failed; the query may still be running.",
    };
    json!({
        "ok": false,
        "error_type": "QueryTimeout",
        "error": message,
        "cleanup": {
            "status": cleanup.status(),
            "remote_stop_confirmed": cleanup == QueryCleanup::Stopped,
        },
    })
}

/// Map Logs Insights result rows (field/value pairs per row) onto the closed
/// `table` render shape. Columns are the union of field names in first-seen
/// order; the opaque `@ptr` field is dropped.
pub fn results_table(rows: &[Vec<(String, String)>]) -> Value {
    let mut columns: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (field, value) in row {
            if field == "@ptr" {
                continue;
            }
            if !columns.contains(field) {
                columns.push(field.clone());
            }
            obj.insert(field.clone(), json!(value));
        }
        out.push(Value::Object(obj));
    }
    json!({"render": "table", "columns": columns, "rows": out})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    fn row(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(f, v)| (f.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn union_columns_in_first_seen_order_and_ptr_dropped() {
        let rows = vec![
            row(&[("@timestamp", "t1"), ("@message", "m1"), ("@ptr", "p")]),
            row(&[("@timestamp", "t2"), ("level", "ERROR")]),
        ];
        let t = results_table(&rows);
        assert_eq!(t["render"], "table");
        assert_eq!(t["columns"], json!(["@timestamp", "@message", "level"]));
        assert_eq!(t["rows"][0]["@message"], "m1");
        assert_eq!(t["rows"][1]["level"], "ERROR");
        assert!(t["rows"][0].get("@ptr").is_none());
    }

    #[test]
    fn empty_results_render_as_empty_table() {
        let t = results_table(&[]);
        assert_eq!(t["render"], "table");
        assert_eq!(t["rows"], json!([]));
    }

    #[test]
    fn timeout_reports_only_confirmed_stop_as_stopped() {
        use aws_smithy_types::error::metadata::ErrorMetadata;

        for (success, expected) in [
            (true, QueryCleanup::Stopped),
            (false, QueryCleanup::NotConfirmed),
        ] {
            let output = StopQueryOutput::builder().success(success).build();
            let cleanup = cleanup_result::<ErrorMetadata>(Ok(output));
            assert_eq!(cleanup, expected);
            let result = timeout_result(cleanup);
            assert_eq!(result["ok"], false);
            assert_eq!(result["error_type"], "QueryTimeout");
            assert_eq!(result["cleanup"]["remote_stop_confirmed"], success);
            if !success {
                assert!(result["error"]
                    .as_str()
                    .unwrap()
                    .contains("may still be running"));
            }
        }
    }

    #[test]
    fn failed_or_denied_stop_never_leaks_service_metadata_or_claims_completion() {
        use aws_smithy_types::error::metadata::ErrorMetadata;

        for (code, expected) in [
            ("AccessDeniedException", QueryCleanup::Denied),
            ("SYNTHETIC_PRIVATE_CODE_MARKER", QueryCleanup::Failed),
        ] {
            let error = ErrorMetadata::builder()
                .code(code)
                .message("SYNTHETIC_PRIVATE_MESSAGE_MARKER")
                .build();
            let cleanup = cleanup_result(Err(error));
            assert_eq!(cleanup, expected);
            let result = timeout_result(cleanup);
            assert_eq!(result["cleanup"]["status"], expected.status());
            assert_eq!(result["cleanup"]["remote_stop_confirmed"], false);
            assert!(result["error"]
                .as_str()
                .unwrap()
                .contains("may still be running"));
            assert!(!result.to_string().contains("SYNTHETIC_PRIVATE"));
        }
    }

    #[tokio::test]
    async fn completed_query_with_unread_result_page_retains_rows_and_reports_limit() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                "Logs_20140328.StartQuery",
                json!({"logGroupName":"/synthetic/query", "queryString":"fields @message"}),
                json!({"queryId":"synthetic-query"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.GetQueryResults",
                json!({"queryId":"synthetic-query"}),
                json!({"status":"Complete", "results":[[{"field":"@message","value":"synthetic evidence"}]],"nextToken":"synthetic-more"}),
            ),
        ]);
        let result = fetch(&script.context(
            &dir,
            "logs-insights",
            json!({"log_group":"/synthetic/query","query":"fields @message"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(result["rows"][0]["@message"], "synthetic evidence");
        assert_eq!(result["coverage"]["completeness"], "limited");
        assert_eq!(result["coverage"]["has_more"], true);
        assert_eq!(crate::request::outcome(&result), "succeeded");
    }

    #[tokio::test]
    async fn polling_failure_preserves_unknown_remote_state_without_an_extra_stop_request() {
        let dir = TestDir::new();
        let script = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                "Logs_20140328.StartQuery",
                json!({}),
                json!({"queryId":"synthetic-query"}),
            ),
            ExpectedRequest::json(
                "Logs_20140328.GetQueryResults",
                json!({"queryId":"synthetic-query"}),
                json!({"__type":"AccessDeniedException","message":"SYNTHETIC_PRIVATE_QUERY_ERROR"}),
            )
            .status(400),
        ]);
        let result = fetch(&script.context(
            &dir,
            "logs-insights",
            json!({"log_group":"/synthetic/query","query":"fields @message"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(script.calls(), 2);
        assert_eq!(result["cleanup"]["status"], "not_attempted");
        assert_eq!(result["cleanup"]["remote_stop_confirmed"], false);
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE_QUERY_ERROR"));
    }
}
