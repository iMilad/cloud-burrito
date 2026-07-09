//! Pipeline Runs — recent executions of one CodePipeline as a status table.

use serde_json::{json, Value};

use super::{dt_iso, err_msg, WidgetCtx};

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let pipeline_name = ctx.input_str("pipeline_name", "");
    if pipeline_name.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "pipeline_name input is required"}});
    }
    let max_results = ctx.input_i64("max_results", 10).clamp(1, 100) as i32;

    let client = aws_sdk_codepipeline::Client::new(&ctx.sdk);
    if let Some(denied) = ctx.preflight("codepipeline", "ListPipelineExecutions") {
        return denied;
    }
    let resp = match client
        .list_pipeline_executions()
        .pipeline_name(&pipeline_name)
        .max_results(max_results)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return json!({"render": "raw_json", "data": {"error": err_msg(e)}});
        }
    };

    let rows: Vec<Value> = resp
        .pipeline_execution_summaries()
        .iter()
        .map(|r| {
            let trigger = r.trigger();
            let ttype = trigger
                .and_then(|t| t.trigger_type())
                .map(|t| t.as_str())
                .unwrap_or("");
            let tdetail = trigger.and_then(|t| t.trigger_detail()).unwrap_or("");
            json!({
                "execution_id": r.pipeline_execution_id().unwrap_or(""),
                "status": r.status().map(|s| s.as_str()).unwrap_or(""),
                "last_updated": dt_iso(r.last_update_time()),
                "trigger": format!("{ttype}: {tdetail}"),
            })
        })
        .collect();

    json!({
        "render": "table",
        "columns": ["execution_id", "status", "last_updated", "trigger"],
        "rows": rows,
    })
}
