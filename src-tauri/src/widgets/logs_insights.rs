//! Logs Insights Query — run a user-written CloudWatch Logs Insights query
//! against one log group and render the result rows as a table.
//!
//! The query text is the user's own; genericity comes for free. StartQuery /
//! GetQueryResults follow the polling pattern established by errors_by_stack.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{
    cloudwatch_logs,
    coverage::Coverage,
    query::{self, QuerySpec, QueryTiming},
    WidgetCtx,
};

/// Insights queries cost by data scanned — cap the window at 7 days.
const MAX_RANGE_SECONDS: i64 = 7 * 86_400;
const RESULT_LIMIT: i32 = 1000;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    match ctx.input_str("mode", "query").as_str() {
        "groups" => cloudwatch_logs::fetch_groups(ctx).await,
        "query" => run_query(ctx).await,
        _ => json!({"ok": false, "error": "Unsupported query mode."}),
    }
}

async fn run_query(ctx: &WidgetCtx) -> Value {
    for operation in ["StartQuery", "GetQueryResults", "StopQuery"] {
        if let Some(denied) = ctx.preflight("logs", operation) {
            return denied;
        }
    }
    let log_group = ctx.input_str("log_group", "");
    if log_group.is_empty() {
        return json!({"ok":false,"error":"log_group is required"});
    }
    let query_text = ctx.input_str("query", "");
    if query_text.trim().is_empty() {
        return json!({"ok":false,"error":"query is required"});
    }
    let range = ctx
        .input_i64("range_seconds", 3600)
        .clamp(60, MAX_RANGE_SECONDS);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let spec = QuerySpec {
        group: &log_group,
        query: &query_text,
        start: now - range,
        end: now,
        limit: RESULT_LIMIT,
    };
    let outcome = query::run(ctx, &spec, QueryTiming::default()).await;
    let Some(response) = outcome.result.as_ref() else {
        return outcome.error_result();
    };
    let rows: Vec<Vec<(String, String)>> = response
        .results()
        .iter()
        .map(|record| {
            record
                .iter()
                .map(|field| {
                    (
                        field.field().unwrap_or("").to_string(),
                        field.value().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .collect();
    let mut out = results_table(&rows);
    if let Some(stats) = response.statistics() {
        out["stats"] = json!({"records_matched":stats.records_matched(),"records_scanned":stats.records_scanned(),"bytes_scanned":stats.bytes_scanned()});
    }
    out["account_id"] = json!(ctx.account_id);
    out["region"] = json!(ctx.region);
    out["query_state"] = json!(outcome.state);
    out["cleanup"] = outcome.cleanup.json();
    out["recovery_required"] = json!(false);
    let mut coverage = Coverage::complete(rows.len());
    coverage.count("pages", 1);
    coverage.count("polls", outcome.polls);
    coverage.limit("results", Some(RESULT_LIMIT as usize));
    if response.next_token().is_some_and(|token| !token.is_empty()) {
        coverage.has_more(Some(true));
        coverage.limited(
            "query_result_page",
            "The query completed, but only the first result page was loaded.",
        );
    } else if rows.len() >= RESULT_LIMIT as usize {
        coverage.has_more(None);
        coverage.unknown_reason(
            "query_result_limit",
            "The query reached its result limit; additional matching records may exist.",
        );
    }
    coverage.attach(out)
}

fn results_table(rows: &[Vec<(String, String)>]) -> Value {
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
    async fn polling_failure_uses_shared_cleanup_and_reports_accepted_stop() {
        let dir = TestDir::new();
        crate::aws::policy::write_text(
            &dir.paths(),
            "statements:\n - effect: Allow\n   action: ['*']\n",
        )
        .unwrap();
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
            ExpectedRequest::json(
                "Logs_20140328.StopQuery",
                json!({"queryId":"synthetic-query"}),
                json!({"success":true}),
            ),
        ]);
        let result = fetch(&script.context(
            &dir,
            "logs-insights",
            json!({"log_group":"/synthetic/query","query":"fields @message"}),
        ))
        .await;
        script.assert_finished();
        assert_eq!(script.calls(), 3);
        assert_eq!(result["cleanup"]["status"], "stopped");
        assert_eq!(result["cleanup"]["remote_stop_confirmed"], true);
        assert_eq!(result["cleanup"]["remote_queries_may_still_run"], false);
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE_QUERY_ERROR"));
    }
}
