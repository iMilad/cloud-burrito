//! CodeBuild Log — CloudWatch log lines for one CodeBuild build, shown inline
//! under a pipeline action. Read-only (BatchGetBuilds + GetLogEvents).

use serde_json::{json, Value};

use super::{err_msg, WidgetCtx};

const PAGE_LIMIT: i32 = 10_000; // events per GetLogEvents call (API cap ~10k/1MB)
const MAX_EVENTS: usize = 10_000; // safety cap on total lines pulled into one view

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
        return denied;
    }
    let cb = aws_sdk_codebuild::Client::new(&ctx.sdk);
    let builds = match cb.batch_get_builds().ids(&build_id).send().await {
        Ok(r) => r,
        Err(e) => {
            return json!({"render": "log_stream", "log_group": build_id, "events": [], "error": err_msg(e)});
        }
    };
    let logs = builds.builds().first().and_then(|b| b.logs());
    let group = logs.and_then(|l| l.group_name()).unwrap_or("").to_string();
    let stream = logs.and_then(|l| l.stream_name()).unwrap_or("").to_string();
    if group.is_empty() || stream.is_empty() {
        return json!({
            "render": "log_stream", "log_group": "", "events": [],
            "error": "No CloudWatch logs for this build (it may use S3 logs, or logs are disabled)."
        });
    }

    if let Some(denied) = ctx.preflight("logs", "GetLogEvents") {
        return denied;
    }
    let cw = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);

    // Page forward through the whole stream to return the COMPLETE log (a single
    // GetLogEvents call is capped at ~10k events / 1MB). GetLogEvents returns the
    // same nextForwardToken once no more events remain — that's our stop signal.
    let mut events: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    let mut truncated = false;
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
                if events.is_empty() {
                    return json!({"render": "log_stream", "log_group": group, "events": [], "error": err_msg(e)});
                }
                truncated = true; // surface whatever we managed to pull
                break;
            }
        };
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

    if truncated {
        let last_ts = events
            .last()
            .and_then(|e| e.get("ts").and_then(|t| t.as_i64()))
            .unwrap_or(0);
        let n = events.len();
        events.push(json!({
            "ts": last_ts,
            "msg": format!("— log truncated at {n} lines; open in AWS Console for the full log —"),
            "level": "warn",
        }));
    }

    json!({"render": "log_stream", "log_group": group, "events": events})
}
