//! CloudFormation Stacks — list active stacks, enrich the top 20 with a
//! resource count (concurrently). The enrichment is best-effort and bounded so
//! the widget never explodes into thousands of API calls.

use std::collections::HashMap;

use aws_sdk_cloudformation::types::StackStatus;
use futures::future::join_all;
use serde_json::{json, Value};

use super::{dt_iso, err_msg, WidgetCtx};

const DEFAULT_STATUS: &[&str] = &[
    "CREATE_COMPLETE",
    "UPDATE_COMPLETE",
    "ROLLBACK_COMPLETE",
    "UPDATE_ROLLBACK_COMPLETE",
];
const ENRICH_LIMIT: usize = 20;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let name_prefix = ctx.input_str("name_prefix", "");
    let status_filter: Vec<String> = match ctx.input_value("status_filter") {
        Some(Value::Array(a)) if !a.is_empty() => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => DEFAULT_STATUS.iter().map(|s| s.to_string()).collect(),
    };

    let client = aws_sdk_cloudformation::Client::new(&ctx.sdk);
    let filters: Vec<StackStatus> = status_filter
        .iter()
        .map(|s| StackStatus::from(s.as_str()))
        .collect();

    if let Some(denied) = ctx.preflight("cloudformation", "ListStacks") {
        return denied;
    }
    let resp = match client
        .list_stacks()
        .set_stack_status_filter(Some(filters))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "render": "table",
                "columns": ["stack", "status", "resources", "last_updated"],
                "rows": [],
                "error": err_msg(e),
            });
        }
    };

    let mut summaries = resp.stack_summaries().to_vec();
    if !name_prefix.is_empty() {
        summaries.retain(|s| {
            s.stack_name()
                .map(|n| n.starts_with(&name_prefix))
                .unwrap_or(false)
        });
    }

    // Enrich the first 20 stacks with their resource count, concurrently.
    let enrich_targets: Vec<String> = summaries
        .iter()
        .take(ENRICH_LIMIT)
        .filter_map(|s| s.stack_name().map(str::to_string))
        .collect();
    if !enrich_targets.is_empty() {
        if let Some(denied) = ctx.preflight("cloudformation", "DescribeStackResources") {
            return denied;
        }
    }
    let count_futs = enrich_targets.iter().map(|name| {
        let client = client.clone();
        let name = name.clone();
        async move {
            let n = client
                .describe_stack_resources()
                .stack_name(&name)
                .send()
                .await
                .ok()
                .map(|r| r.stack_resources().len());
            (name, n)
        }
    });
    let counts: HashMap<String, Option<usize>> = join_all(count_futs).await.into_iter().collect();

    let mut rows = Vec::new();
    for (i, s) in summaries.iter().enumerate() {
        let stack_name = s.stack_name().unwrap_or("").to_string();
        let resources: Value = if i < ENRICH_LIMIT {
            match counts.get(&stack_name) {
                Some(Some(c)) => json!(c),
                _ => json!(""),
            }
        } else {
            json!("")
        };
        let last = s.last_updated_time().or_else(|| s.creation_time());
        rows.push(json!({
            "stack": stack_name,
            "status": s.stack_status().map(|x| x.as_str()).unwrap_or(""),
            "resources": resources,
            "last_updated": dt_iso(last),
        }));
    }

    json!({
        "render": "table",
        "columns": ["stack", "status", "resources", "last_updated"],
        "rows": rows,
    })
}
