//! One resource response and a bounded first page of recent stack events.

use serde_json::{json, Value};

use super::coverage::Coverage;
use super::{dt_iso, err_msg, WidgetCtx};

const MAX_EVENTS: usize = 60;
// The locked SDK documents DescribeStackResources as returning at most 100.
const RESOURCE_SERVICE_LIMIT: usize = 100;

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let stack = ctx.input_str("stack_name", "");
    if stack.is_empty() {
        return json!({"render":"stack_detail","stack":"","resources":[],"events":[],"error":"stack_name is required"});
    }
    let client = aws_sdk_cloudformation::Client::new(&ctx.sdk);
    if let Some(denied) = ctx.preflight("cloudformation", "DescribeStackResources") {
        return denied;
    }
    let resources: Vec<Value> = match client
        .describe_stack_resources()
        .stack_name(&stack)
        .send()
        .await
    {
        Ok(response) => response
            .stack_resources()
            .iter()
            .map(|resource| {
                json!({
                    "logical_id":resource.logical_resource_id().unwrap_or(""),
                    "type":resource.resource_type().unwrap_or(""),
                    "status":resource.resource_status().map(|status| status.as_str()).unwrap_or(""),
                    "physical_id":resource.physical_resource_id().unwrap_or(""),
                    "updated":dt_iso(resource.timestamp()),
                })
            })
            .collect(),
        Err(error) => {
            let mut resources = Coverage::unknown(0);
            resources.count("pages", 0);
            resources.limit("service_resources", Some(RESOURCE_SERVICE_LIMIT));
            resources.failure(
                "request_failed",
                "Stack resources could not be loaded.",
                false,
            );
            let mut events = Coverage::unknown(0);
            events.count("pages", 0);
            events.limit("results", Some(MAX_EVENTS));
            events.unknown_reason(
                "not_attempted",
                "Stack events were not requested after the resource request failed.",
            );
            return detail_result(
                &stack,
                Vec::new(),
                Vec::new(),
                resources,
                events,
                Some(err_msg(error)),
            );
        }
    };
    let mut resource_coverage = Coverage::complete(resources.len());
    resource_coverage.count("pages", 1);
    resource_coverage.limit("service_resources", Some(RESOURCE_SERVICE_LIMIT));
    if resources.len() >= RESOURCE_SERVICE_LIMIT {
        resource_coverage.unknown_reason(
            "service_limit",
            "The resource response reached the service limit; additional resources may exist.",
        );
    }
    let mut event_coverage = Coverage::unknown(0);
    event_coverage.limit("results", Some(MAX_EVENTS));
    event_coverage.count("pages", 0);
    if ctx
        .preflight("cloudformation", "DescribeStackEvents")
        .is_some()
    {
        event_coverage.failure("policy_denied", "Stack events were not permitted.", false);
        return detail_result(
            &stack,
            resources,
            Vec::new(),
            resource_coverage,
            event_coverage,
            Some("Stack events were not permitted; the available resources are retained.".into()),
        );
    }
    let response = match client
        .describe_stack_events()
        .stack_name(&stack)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            event_coverage.failure("request_failed", "Stack events could not be loaded.", false);
            return detail_result(
                &stack,
                resources,
                Vec::new(),
                resource_coverage,
                event_coverage,
                Some(err_msg(error)),
            );
        }
    };
    let events: Vec<Value> = response
        .stack_events()
        .iter()
        .take(MAX_EVENTS)
        .map(|event| {
            json!({
                "time":dt_iso(event.timestamp()),
                "logical_id":event.logical_resource_id().unwrap_or(""),
                "type":event.resource_type().unwrap_or(""),
                "status":event.resource_status().map(|status| status.as_str()).unwrap_or(""),
                "reason":event.resource_status_reason().unwrap_or(""),
            })
        })
        .collect();
    event_coverage = Coverage::complete(events.len());
    event_coverage.count("pages", 1);
    event_coverage.limit("results", Some(MAX_EVENTS));
    if response.stack_events().len() > MAX_EVENTS
        || response.next_token().is_some_and(|token| !token.is_empty())
    {
        event_coverage.has_more(Some(true));
        event_coverage.limited(
            "event_limit",
            "Only the bounded first page of recent stack events is shown.",
        );
    }
    detail_result(
        &stack,
        resources,
        events,
        resource_coverage,
        event_coverage,
        None,
    )
}

fn detail_result(
    stack: &str,
    resources: Vec<Value>,
    events: Vec<Value>,
    resource_coverage: Coverage,
    event_coverage: Coverage,
    error: Option<String>,
) -> Value {
    let mut coverage = Coverage::complete(resources.len() + events.len());
    coverage.count("resources", resources.len());
    coverage.count("events", events.len());
    coverage.section("resources", resource_coverage);
    coverage.section("events", event_coverage);
    let mut result =
        json!({"render":"stack_detail","stack":stack,"resources":resources,"events":events});
    if let Some(error) = error {
        result["error"] = json!(error);
    }
    coverage.attach(result)
}
