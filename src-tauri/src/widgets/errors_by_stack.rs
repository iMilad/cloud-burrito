//! Errors by Stack — count ERROR log lines per log group over a window using
//! CloudWatch Logs Insights. The top 20 most-recently-created matching groups
//! are queried concurrently; wall time is bounded by the slowest single query.

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

    let client = Client::new(&ctx.sdk);
    if let Some(denied) = ctx.preflight("logs", "DescribeLogGroups") {
        return denied;
    }
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

    if !group_names.is_empty() {
        if let Some(denied) = ctx.preflight("logs", "StartQuery") {
            return denied;
        }
        if let Some(denied) = ctx.preflight("logs", "GetQueryResults") {
            return denied;
        }
    }
    let futs = group_names.iter().map(|g| {
        let client = client.clone();
        let g = g.clone();
        async move {
            let total = cw_insights_count(&client, &g, start, now).await;
            (g, total)
        }
    });
    let results = join_all(futs).await;

    let mut rows: Vec<(String, i64)> = Vec::new();
    for (group, total) in results {
        match total {
            Ok(t) if t > 0 => rows.push((group, t)),
            Ok(_) => {}
            Err(err) => ctx.log("cw_insights failed", json!({"log_group": group, "error": err})),
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(stack, errors)| json!({"stack": stack, "errors": errors}))
        .collect();

    json!({"render": "errors_chart", "hours": hours, "rows": out})
}

/// Start an Insights query and poll until Complete, summing the `errors` column.
async fn cw_insights_count(
    client: &Client,
    group: &str,
    start: i64,
    end: i64,
) -> Result<i64, String> {
    let started = client
        .start_query()
        .log_group_name(group)
        .start_time(start)
        .end_time(end)
        .query_string(INSIGHTS_QUERY)
        .limit(100)
        .send()
        .await
        .map_err(err_msg)?;
    let query_id = started
        .query_id()
        .ok_or_else(|| "no queryId returned".to_string())?
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let resp = client
            .get_query_results()
            .query_id(&query_id)
            .send()
            .await
            .map_err(err_msg)?;
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
            Some(QueryStatus::Failed) | Some(QueryStatus::Cancelled) | Some(QueryStatus::Timeout) => {
                return Err(format!("query terminated: {:?}", resp.status()));
            }
            _ => {
                if Instant::now() >= deadline {
                    return Err("query timeout (30s budget exhausted)".to_string());
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
}
