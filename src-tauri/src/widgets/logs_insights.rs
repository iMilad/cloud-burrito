//! Logs Insights Query — run a user-written CloudWatch Logs Insights query
//! against one log group and render the result rows as a table.
//!
//! The query text is the user's own; genericity comes for free. StartQuery /
//! GetQueryResults follow the polling pattern established by errors_by_stack.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::types::QueryStatus;
use serde_json::{json, Value};

use super::{cloudwatch_logs, err_msg, WidgetCtx};

/// Insights queries cost by data scanned — cap the window at 7 days.
const MAX_RANGE_SECONDS: i64 = 7 * 86_400;
/// How long we poll before stopping the query and telling the user to narrow it.
const POLL_BUDGET: Duration = Duration::from_secs(25);

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    match ctx.input_str("mode", "query").as_str() {
        "groups" => cloudwatch_logs::fetch_groups(ctx).await,
        "query" => run_query(ctx).await,
        other => json!({"ok": false, "error": format!("unknown mode: {other}")}),
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
    let range = ctx.input_i64("range_seconds", 3600).clamp(60, MAX_RANGE_SECONDS);

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
        return json!({"ok": false, "error": "no queryId returned"});
    };

    let deadline = Instant::now() + POLL_BUDGET;
    loop {
        let resp = match client.get_query_results().query_id(&query_id).send().await {
            Ok(r) => r,
            Err(e) => return json!({"ok": false, "error": err_msg(e)}),
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
                return out;
            }
            Some(QueryStatus::Failed) | Some(QueryStatus::Cancelled) | Some(QueryStatus::Timeout) => {
                return json!({"ok": false, "error": format!("query terminated: {:?}", resp.status())});
            }
            _ => {
                if Instant::now() >= deadline {
                    stop_query_best_effort(ctx, &client, &query_id).await;
                    return json!({
                        "ok": false,
                        "error": "query still running after 25s — narrow the time range or the query",
                    });
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
        }
    }
}

/// Stop a timed-out query so it stops scanning (and billing). Denial by
/// policy just means the query expires server-side instead.
async fn stop_query_best_effort(
    ctx: &WidgetCtx,
    client: &aws_sdk_cloudwatchlogs::Client,
    query_id: &str,
) {
    if ctx.preflight("logs", "StopQuery").is_some() {
        return;
    }
    if let Err(e) = client.stop_query().query_id(query_id).send().await {
        ctx.log("stop_query failed", json!({"error": err_msg(e)}));
    }
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

    fn row(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(f, v)| (f.to_string(), v.to_string())).collect()
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
}
