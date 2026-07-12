//! Pipeline Execution Detail — the action-by-action breakdown ("log") of one
//! CodePipeline execution, via ListActionExecutions. Read-only.

use aws_sdk_codepipeline::types::ActionExecutionFilter;
use serde_json::{json, Value};

use super::{dt_iso, dt_secs, err_msg, WidgetCtx};

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
            return json!({
                "render": "execution_detail",
                "pipeline": pipeline_name,
                "execution_id": execution_id,
                "actions": [],
                "error": err_msg(e),
            });
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
            }),
        ));
    }
    actions.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let out: Vec<Value> = actions.into_iter().map(|(_, v)| v).collect();

    json!({
        "render": "execution_detail",
        "pipeline": pipeline_name,
        "execution_id": execution_id,
        "actions": out,
    })
}
