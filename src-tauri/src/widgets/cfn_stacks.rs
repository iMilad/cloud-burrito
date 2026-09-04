//! CloudFormation Stacks — list every non-deleted stack, enrich the first 20
//! with a resource count (concurrently). The enrichment is best-effort and
//! bounded so the widget never explodes into thousands of API calls.

use std::collections::HashMap;

use aws_sdk_cloudformation::types::StackStatus;
use futures::future::join_all;
use serde_json::{json, Value};

use super::coverage::Coverage;
use super::{dt_iso, err_msg, WidgetCtx};

const ENRICH_LIMIT: usize = 20;

fn include_stack_status(status: Option<&str>, has_explicit_filter: bool) -> bool {
    has_explicit_filter || status != Some("DELETE_COMPLETE")
}

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let name_prefix = ctx.input_str("name_prefix", "");
    let status_filters: Option<Vec<StackStatus>> = match ctx.input_value("status_filter") {
        Some(Value::Array(a)) if !a.is_empty() => Some(
            a.iter()
                .filter_map(|v| v.as_str().map(StackStatus::from))
                .collect(),
        ),
        _ => None,
    };

    let client = aws_sdk_cloudformation::Client::new(&ctx.sdk);
    let mut summaries = Vec::new();
    let mut token: Option<String> = None;
    let mut pages = 0usize;
    let mut page_error = None;
    loop {
        if let Some(denied) = ctx.preflight("cloudformation", "ListStacks") {
            if pages == 0 {
                return denied;
            }
            page_error = Some("Additional stack pages were not permitted.".to_string());
            break;
        }
        let mut req = client
            .list_stacks()
            .set_stack_status_filter(status_filters.clone());
        if let Some(next_token) = &token {
            req = req.next_token(next_token);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                page_error = Some(err_msg(e));
                break;
            }
        };
        pages += 1;
        summaries.extend(
            resp.stack_summaries()
                .iter()
                .filter(|summary| {
                    include_stack_status(
                        summary.stack_status().map(StackStatus::as_str),
                        status_filters.is_some(),
                    )
                })
                .cloned(),
        );
        token = resp
            .next_token()
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if token.is_none() {
            break;
        }
    }

    if !name_prefix.is_empty() {
        summaries.retain(|s| {
            s.stack_name()
                .map(|n| n.starts_with(&name_prefix))
                .unwrap_or(false)
        });
    }

    // Enrich the first 20 stacks with their resource count, concurrently.
    let mut enrich_targets: Vec<String> = summaries
        .iter()
        .take(ENRICH_LIMIT)
        .filter_map(|s| s.stack_name().map(str::to_string))
        .collect();
    let mut enrichment_denied = false;
    if page_error.is_some() {
        enrich_targets.clear();
    }
    if !enrich_targets.is_empty()
        && ctx
            .preflight("cloudformation", "DescribeStackResources")
            .is_some()
    {
        enrichment_denied = true;
        enrich_targets.clear();
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
                .map(|r| r.stack_resources().len())
                .map_err(err_msg);
            (name, n)
        }
    });
    let counts: HashMap<String, Result<usize, String>> =
        join_all(count_futs).await.into_iter().collect();

    let mut rows = Vec::new();
    for (i, s) in summaries.iter().enumerate() {
        let stack_name = s.stack_name().unwrap_or("").to_string();
        let resources: Value = if i < ENRICH_LIMIT {
            match counts.get(&stack_name) {
                Some(Ok(c)) => json!(c),
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

    let mut listing = Coverage::complete(rows.len());
    listing.count("pages", pages);
    listing.limit("results", None);
    if page_error.is_some() {
        listing.failure(
            "request_failed",
            "A stack-list page could not be loaded; earlier matching stacks are retained.",
            pages > 0,
        );
    }
    let failed = counts.values().filter(|result| result.is_err()).count();
    let mut enrichment = Coverage::complete(counts.len() - failed);
    enrichment.count("attempted", enrich_targets.len());
    enrichment.count("failed", failed);
    enrichment.count(
        "skipped",
        summaries.len().saturating_sub(enrich_targets.len()),
    );
    enrichment.limit("stacks", Some(ENRICH_LIMIT));
    enrichment.limit("service_resources_per_stack", Some(100));
    if failed > 0 {
        enrichment.failure(
            "enrichment_failed",
            "Some stack resource counts could not be loaded.",
            true,
        );
    }
    if enrichment_denied {
        enrichment.failure(
            "policy_denied",
            "Stack resource counts were not permitted.",
            !rows.is_empty(),
        );
    } else if page_error.is_some() {
        enrichment.unknown_reason(
            "not_attempted",
            "Resource-count enrichment was not requested after the listing failed.",
        );
    } else if summaries.len() > ENRICH_LIMIT {
        enrichment.limited(
            "enrichment_limit",
            "Resource counts are requested only for the first 20 matching stacks.",
        );
    }
    if counts
        .values()
        .any(|result| result.as_ref().is_ok_and(|count| *count >= 100))
    {
        enrichment.unknown_reason("service_limit", "A resource count reached the service limit; that stack may contain additional resources.");
    }
    let mut coverage = Coverage::complete(rows.len());
    coverage.section("stacks", listing);
    coverage.section("enrichment", enrichment);
    let mut result = json!({
        "render": "table",
        "columns": ["stack", "status", "resources", "last_updated"],
        "rows": rows,
    });
    if let Some(error) = page_error {
        result["error"] = json!(error);
    }
    coverage.attach(result)
}

#[cfg(test)]
mod tests {
    use super::include_stack_status;

    #[test]
    fn default_view_keeps_active_and_failed_stacks_but_hides_deleted_stacks() {
        for status in [
            "CREATE_IN_PROGRESS",
            "UPDATE_FAILED",
            "IMPORT_COMPLETE",
            "DELETE_FAILED",
        ] {
            assert!(include_stack_status(Some(status), false), "{status}");
        }
        assert!(!include_stack_status(Some("DELETE_COMPLETE"), false));
    }

    #[test]
    fn explicit_status_filter_is_preserved() {
        assert!(include_stack_status(Some("DELETE_COMPLETE"), true));
    }
}
