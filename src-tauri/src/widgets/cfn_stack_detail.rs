//! CFN Stack Detail — a stack's resources + recent events, shown inline under a
//! CloudFormation Stacks row. Read-only (DescribeStackResources + DescribeStackEvents).

use serde_json::{json, Value};

use super::{dt_iso, err_msg, WidgetCtx};

const MAX_EVENTS: usize = 60;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let stack = ctx.input_str("stack_name", "");
    if stack.is_empty() {
        return json!({"render": "stack_detail", "stack": "", "resources": [], "events": [], "error": "stack_name is required"});
    }
    let client = aws_sdk_cloudformation::Client::new(&ctx.sdk);

    // Resources (DescribeStackResources is already an allowed op).
    if let Some(denied) = ctx.preflight("cloudformation", "DescribeStackResources") {
        return denied;
    }
    let resources: Vec<Value> = match client
        .describe_stack_resources()
        .stack_name(&stack)
        .send()
        .await
    {
        Ok(r) => r
            .stack_resources()
            .iter()
            .map(|sr| {
                json!({
                    "logical_id": sr.logical_resource_id().unwrap_or(""),
                    "type": sr.resource_type().unwrap_or(""),
                    "status": sr.resource_status().map(|s| s.as_str()).unwrap_or(""),
                    "physical_id": sr.physical_resource_id().unwrap_or(""),
                    "updated": dt_iso(sr.timestamp()),
                })
            })
            .collect(),
        Err(e) => {
            return json!({"render": "stack_detail", "stack": stack, "resources": [], "events": [], "error": err_msg(e)});
        }
    };

    // Recent events (DescribeStackEvents returns most-recent first).
    if let Some(denied) = ctx.preflight("cloudformation", "DescribeStackEvents") {
        return denied;
    }
    let events: Vec<Value> = match client
        .describe_stack_events()
        .stack_name(&stack)
        .send()
        .await
    {
        Ok(r) => r
            .stack_events()
            .iter()
            .take(MAX_EVENTS)
            .map(|ev| {
                json!({
                    "time": dt_iso(ev.timestamp()),
                    "logical_id": ev.logical_resource_id().unwrap_or(""),
                    "type": ev.resource_type().unwrap_or(""),
                    "status": ev.resource_status().map(|s| s.as_str()).unwrap_or(""),
                    "reason": ev.resource_status_reason().unwrap_or(""),
                })
            })
            .collect(),
        Err(e) => {
            return json!({"render": "stack_detail", "stack": stack, "resources": resources, "events": [], "error": err_msg(e)});
        }
    };

    json!({"render": "stack_detail", "stack": stack, "resources": resources, "events": events})
}
