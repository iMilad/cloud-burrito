//! CloudWatch Logs — search log groups, browse their streams, view events.
//!
//! Streams and events reuse the Lambda Logs implementation: those modes are
//! log-group-generic and never cared about Lambda.

use serde_json::{json, Value};

use super::{err_msg, log_tail, WidgetCtx};

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
    let max_groups = ctx
        .input_i64("max_groups", 500)
        .clamp(1, MAX_GROUPS as i64) as usize;
    // Optional server-side narrowing (case-insensitive substring match) for
    // accounts with more groups than the cap.
    let pattern = ctx.input_str("name_pattern", "");

    if let Some(denied) = ctx.preflight("logs", "DescribeLogGroups") {
        return denied;
    }
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);

    let mut groups: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
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
            Err(e) => return json!({"ok": false, "error": err_msg(e)}),
        };
        for g in resp.log_groups() {
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
    let capped = groups.len() >= max_groups;
    json!({
        "ok": true,
        "account_id": ctx.account_id,
        "region": ctx.region,
        "groups": groups,
        "capped": capped,
    })
}
