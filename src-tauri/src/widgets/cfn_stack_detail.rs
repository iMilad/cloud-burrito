//! One resource response and a bounded first page of recent stack events.

use serde_json::{json, Value};

use super::coverage::Coverage;
use super::handoff::{self, Source};
use super::{dt_iso, err_msg, WidgetCtx};

const MAX_EVENTS: usize = 60;
// The locked SDK documents DescribeStackResources as returning at most 100.
const RESOURCE_SERVICE_LIMIT: usize = 100;

fn resource_handoff(
    ctx: &WidgetCtx,
    stack: &str,
    resource: &aws_sdk_cloudformation::types::StackResource,
) -> Value {
    let stack_name = if stack.starts_with("arn:") {
        stack
            .splitn(6, ':')
            .nth(5)
            .and_then(|value| value.split('/').nth(1))
            .unwrap_or("")
    } else {
        stack
    };
    let context_matches = handoff::valid_stack(stack, ctx)
        && resource.stack_name().is_none_or(|name| name == stack_name)
        && resource.stack_id().is_none_or(|id| {
            handoff::valid_stack(id, ctx)
                && if stack.starts_with("arn:") {
                    id == stack
                } else {
                    id.splitn(6, ':')
                        .nth(5)
                        .and_then(|value| value.split('/').nth(1))
                        == Some(stack)
                }
        });
    let logs = if !context_matches {
        handoff::unavailable(Source::ContextMismatch)
    } else if resource.resource_type() == Some("AWS::Logs::LogGroup") {
        handoff::logs(
            resource.physical_resource_id().unwrap_or(""),
            None,
            Source::CfnLogGroup,
        )
    } else {
        // A Lambda physical name does not establish its LoggingConfig.
        handoff::unavailable(Source::IdentifierUnavailable)
    };
    json!({"logs":logs})
}

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
                    "handoffs":resource_handoff(ctx,&stack,resource),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    fn xml(action: &str, body: &str) -> String {
        format!("<{action}Response xmlns=\"http://cloudformation.amazonaws.com/doc/2010-05-15/\"><{action}Result>{body}</{action}Result></{action}Response>")
    }

    #[tokio::test]
    async fn only_same_stack_log_group_physical_ids_offer_log_navigation() {
        let dir = TestDir::new();
        let mut members = String::new();
        for (name, kind, physical, region) in [
            (
                "synthetic-stack",
                "AWS::Logs::LogGroup",
                "/synthetic/exact/slash/group",
                "us-east-1",
            ),
            (
                "synthetic-stack",
                "AWS::Lambda::Function",
                "synthetic-function",
                "us-east-1",
            ),
            (
                "synthetic-other",
                "AWS::Logs::LogGroup",
                "/synthetic/wrong/stack",
                "us-east-1",
            ),
            (
                "synthetic-stack",
                "AWS::Logs::LogGroup",
                "/synthetic/wrong/region",
                "eu-west-1",
            ),
        ] {
            members.push_str(&format!("<member><StackName>{name}</StackName><StackId>arn:aws:cloudformation:{region}:acct-producer-fixture:stack/{name}/synthetic-id</StackId><LogicalResourceId>SyntheticResource</LogicalResourceId><PhysicalResourceId>{physical}</PhysicalResourceId><ResourceType>{kind}</ResourceType><ResourceStatus>CREATE_COMPLETE</ResourceStatus><Timestamp>2026-01-01T00:00:00Z</Timestamp></member>"));
        }
        let http = ScriptedHttp::new(vec![
            ExpectedRequest::xml("DescribeStackResources",json!({"StackName":"synthetic-stack"}),&xml("DescribeStackResources",&format!("<StackResources>{members}</StackResources>"))),
            ExpectedRequest::xml("DescribeStackEvents",json!({"StackName":"synthetic-stack"}),"<ErrorResponse><Error><Code>AccessDenied</Code><Message>SYNTHETIC_PRIVATE_STACK_ERROR</Message></Error></ErrorResponse>").status(403),
        ]);
        let result = fetch(&http.context(
            &dir,
            "cfn-stack-detail",
            json!({"stack_name":"synthetic-stack"}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(
            result["resources"][0]["handoffs"]["logs"]["inputs"],
            json!({"mode":"streams","log_group":"/synthetic/exact/slash/group"})
        );
        assert_eq!(
            result["resources"][0]["handoffs"]["logs"]["source"],
            "cfn_log_group"
        );
        for resource in &result["resources"].as_array().unwrap()[1..] {
            assert_eq!(resource["handoffs"]["logs"]["status"], "unavailable");
        }
        assert_eq!(result["partial"], true);
        assert!(!result.to_string().contains("SYNTHETIC_PRIVATE_STACK_ERROR"));
        assert!(!result
            .to_string()
            .contains("/aws/lambda/synthetic-function"));
    }
}
