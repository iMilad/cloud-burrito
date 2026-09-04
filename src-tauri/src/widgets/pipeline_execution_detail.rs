//! Pipeline Execution Detail — the action-by-action breakdown ("log") of one
//! CodePipeline execution, via ListActionExecutions. Read-only.

use aws_sdk_codepipeline::types::{ActionExecutionDetail, ActionExecutionFilter};
use serde_json::{json, Value};

use super::coverage::Coverage;
use super::handoff::{self, Source};
use super::{dt_iso, dt_secs, err_msg, WidgetCtx};

fn action_handoffs(ctx: &WidgetCtx, execution_id: &str, action: &ActionExecutionDetail) -> Value {
    let unavailable =
        |source| json!({"build":handoff::unavailable(source),"stack":handoff::unavailable(source)});
    let Some(input) = action.input() else {
        return unavailable(Source::UnsupportedAction);
    };
    if action.pipeline_execution_id() != Some(execution_id)
        || input.region().is_some_and(|region| region != ctx.region)
        || input
            .role_arn()
            .is_some_and(|role| !handoff::same_context_arn(role, "iam", "role/", ctx))
    {
        return unavailable(Source::ContextMismatch);
    }
    let Some(kind) = input.action_type_id() else {
        return unavailable(Source::UnsupportedAction);
    };
    if kind.owner().as_str() != "AWS" {
        return unavailable(Source::UnsupportedAction);
    }
    match (kind.category().as_str(), kind.provider()) {
        ("Build", "CodeBuild") => {
            let id = action
                .output()
                .and_then(|output| output.execution_result())
                .and_then(|result| result.external_execution_id())
                .unwrap_or("");
            json!({"build":if handoff::valid_build(id,ctx) { handoff::build(id) } else { handoff::unavailable(Source::IdentifierUnavailable) },"stack":handoff::manual_stack()})
        }
        ("Deploy", "CloudFormation") => {
            // Project only reviewed fields; configuration can contain secrets.
            let selected = |key: &str| {
                input
                    .resolved_configuration()
                    .and_then(|values| values.get(key))
                    .or_else(|| input.configuration().and_then(|values| values.get(key)))
            };
            if selected("RoleArn").is_some_and(|role| {
                !role.is_empty() && !handoff::same_context_arn(role, "iam", "role/", ctx)
            }) {
                return unavailable(Source::ContextMismatch);
            }
            let stack = selected("StackName").map(String::as_str).unwrap_or("");
            json!({"build":handoff::unavailable(Source::UnsupportedAction),"stack":if handoff::valid_stack(stack,ctx) { handoff::stack(stack,Source::PipelineStackConfiguration) } else { handoff::unavailable(Source::IdentifierUnavailable) }})
        }
        _ => unavailable(Source::UnsupportedAction),
    }
}

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let pipeline_name = ctx.input_str("pipeline_name", "");
    let execution_id = ctx.input_str("execution_id", "");
    if pipeline_name.is_empty() || execution_id.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "pipeline_name and execution_id are required"}});
    }

    if let Some(denied) = ctx.preflight("codepipeline", "ListActionExecutions") {
        return denied;
    }

    let client = aws_sdk_codepipeline::Client::new(&ctx.sdk);
    let filter = ActionExecutionFilter::builder()
        .pipeline_execution_id(&execution_id)
        .build();
    let resp = match client
        .list_action_executions()
        .pipeline_name(&pipeline_name)
        .filter(filter)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let mut coverage = Coverage::unknown(0);
            coverage.count("pages", 0);
            coverage.limit("results", None);
            coverage.failure(
                "request_failed",
                "Pipeline actions could not be loaded.",
                false,
            );
            return coverage.attach(json!({
                "render": "execution_detail",
                "pipeline": pipeline_name,
                "execution_id": execution_id,
                "actions": [],
                "error": err_msg(e),
            }));
        }
    };

    let mut actions: Vec<(f64, Value)> = Vec::new();
    for d in resp.action_execution_details() {
        let input = d.input();
        let type_id = input.and_then(|i| i.action_type_id());
        let category = type_id
            .map(|t| t.category().as_str().to_string())
            .unwrap_or_default();
        let provider = type_id
            .map(|t| t.provider().to_string())
            .unwrap_or_default();
        let result = d.output().and_then(|o| o.execution_result());
        let summary = result
            .and_then(|r| r.external_execution_summary())
            .unwrap_or("")
            .to_string();
        let url = result
            .and_then(|r| r.external_execution_url())
            .unwrap_or("")
            .to_string();
        let error = result
            .and_then(|r| r.error_details())
            .map(|e| {
                format!("{} {}", e.code().unwrap_or(""), e.message().unwrap_or(""))
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_default();
        let started = d.start_time();
        actions.push((
            dt_secs(started).unwrap_or(0.0),
            json!({
                "stage": d.stage_name().unwrap_or(""),
                "action": d.action_name().unwrap_or(""),
                "status": d.status().map(|s| s.as_str()).unwrap_or(""),
                "category": category,
                "provider": provider,
                "started": dt_iso(started),
                "last_updated": dt_iso(d.last_update_time()),
                "summary": summary,
                "external_url": url,
                "external_execution_id": result.and_then(|r| r.external_execution_id()).unwrap_or("").to_string(),
                "error": error,
                "handoffs": action_handoffs(ctx,&execution_id,d),
            }),
        ));
    }
    actions.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let out: Vec<Value> = actions.into_iter().map(|(_, v)| v).collect();

    let mut coverage = Coverage::complete(out.len());
    coverage.count("pages", 1);
    coverage.limit("results", None);
    if resp.next_token().is_some_and(|token| !token.is_empty()) {
        coverage.has_more(Some(true));
        coverage.limited("next_page", "Additional pipeline actions were not loaded.");
    }
    coverage.attach(json!({
        "render": "execution_detail",
        "pipeline": pipeline_name,
        "execution_id": execution_id,
        "actions": out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };

    fn action(category: &str, provider: &str) -> Value {
        json!({"pipelineExecutionId":"synthetic-execution","actionName":"synthetic-action","input":{"actionTypeId":{"category":category,"owner":"AWS","provider":provider,"version":"1"},"region":"us-east-1"}})
    }

    async fn fetch_actions(actions: Value) -> Value {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![ExpectedRequest::json(
            "CodePipeline_20150709.ListActionExecutions",
            json!({"pipelineName":"synthetic-pipeline","filter":{"pipelineExecutionId":"synthetic-execution"}}),
            json!({"actionExecutionDetails":actions,"nextToken":"synthetic-unloaded"}),
        )]);
        let result = fetch(&http.context(
            &directory,
            "pipeline-execution-detail",
            json!({"pipeline_name":"synthetic-pipeline","execution_id":"synthetic-execution"}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 1);
        assert_eq!(result["coverage"]["has_more"], true);
        result
    }

    #[tokio::test]
    async fn only_reviewed_execution_fields_become_build_and_configured_stack_targets() {
        let mut build = action("Build", "CodeBuild");
        build["output"] =
            json!({"executionResult":{"externalExecutionId":"synthetic-project:synthetic-run"}});
        build["input"]["configuration"] = json!({"Unreviewed":"SYNTHETIC_PRIVATE_CONFIG_MARKER"});
        let mut stack = action("Deploy", "CloudFormation");
        stack["input"]["configuration"] = json!({"StackName":"#{synthetic.stack}","ParameterOverrides":"SYNTHETIC_PRIVATE_CONFIG_MARKER"});
        stack["input"]["resolvedConfiguration"] = json!({"StackName":"synthetic-resolved-stack","ParameterOverrides":"SYNTHETIC_PRIVATE_CONFIG_MARKER"});
        let mut literal = action("Deploy", "CloudFormation");
        literal["input"]["configuration"] = json!({"StackName":"synthetic-literal-stack"});
        let result = fetch_actions(json!([build, stack, literal])).await;
        assert_eq!(
            result["actions"][0]["handoffs"]["build"]["inputs"],
            json!({"build_id":"synthetic-project:synthetic-run"})
        );
        assert_eq!(
            result["actions"][0]["handoffs"]["stack"]["status"],
            "manual"
        );
        assert_eq!(
            result["actions"][1]["handoffs"]["stack"]["source"],
            "pipeline_stack_configuration"
        );
        assert_eq!(
            result["actions"][1]["handoffs"]["stack"]["inputs"],
            json!({"stack_name":"synthetic-resolved-stack"})
        );
        assert_eq!(
            result["actions"][2]["handoffs"]["stack"]["inputs"],
            json!({"stack_name":"synthetic-literal-stack"})
        );
        assert!(!result
            .to_string()
            .contains("SYNTHETIC_PRIVATE_CONFIG_MARKER"));
        assert!(!result.to_string().contains("ParameterOverrides"));
    }

    #[tokio::test]
    async fn foreign_execution_owner_region_role_and_unresolved_targets_never_link() {
        let mut rows = Vec::new();
        for mutation in [
            "execution",
            "missing_execution",
            "owner",
            "provider",
            "category",
            "region",
            "role",
            "build_arn",
        ] {
            let mut value = action("Build", "CodeBuild");
            value["output"] = json!({"executionResult":{"externalExecutionId":"synthetic-project:synthetic-run"}});
            match mutation {
                "execution" => value["pipelineExecutionId"] = json!("synthetic-other-execution"),
                "missing_execution" => { value.as_object_mut().unwrap().remove("pipelineExecutionId"); },
                "owner" => value["input"]["actionTypeId"]["owner"] = json!("Custom"),
                "provider" => value["input"]["actionTypeId"]["provider"] = json!("SyntheticCustomBuild"),
                "category" => value["input"]["actionTypeId"]["category"] = json!("Test"),
                "region" => value["input"]["region"] = json!("eu-west-1"),
                "role" => value["input"]["roleArn"] = json!("arn:aws:iam::acct-other-fixture:role/synthetic-role"),
                "build_arn" => value["output"]["executionResult"]["externalExecutionId"] = json!("arn:aws:codebuild:us-east-1:acct-other-fixture:build/synthetic-project:synthetic-run"),
                _ => unreachable!(),
            }
            rows.push(value);
        }
        for target in ["#{synthetic.stack}","arn:aws:cloudformation:eu-west-1:acct-producer-fixture:stack/synthetic-stack/synthetic-id"] {
            let mut stack = action("Deploy","CloudFormation");
            stack["input"]["configuration"] = json!({"StackName":target});
            rows.push(stack);
        }
        let mut role = action("Deploy", "CloudFormation");
        role["input"]["configuration"] = json!({"StackName":"synthetic-stack","RoleArn":"arn:aws:iam::acct-other-fixture:role/synthetic-role"});
        rows.push(role);
        let result = fetch_actions(json!(rows)).await;
        for row in result["actions"].as_array().unwrap() {
            assert_ne!(row["handoffs"]["build"]["status"], "available");
            assert_ne!(row["handoffs"]["stack"]["status"], "available");
            assert!(row["handoffs"]["build"]["inputs"]
                .as_object()
                .unwrap()
                .is_empty());
        }
    }
}
