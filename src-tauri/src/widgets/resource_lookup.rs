//! Resource Reverse Lookup — find resources whose ARN matches a substring via
//! the Resource Groups Tagging API, then best-effort enrich the first few with
//! their owning CloudFormation stack.

use std::collections::HashMap;

use futures::future::join_all;
use serde_json::{json, Map, Value};

use super::coverage::Coverage;
use super::{err_msg, WidgetCtx};

const DEFAULT_MAX: i64 = 25;
const STACK_BUDGET: usize = 5;
// Cap how many GetResources pages we scan so a query that matches nothing
// doesn't crawl an entire large account (~100 resources per page).
const MAX_PAGES: usize = 25;

fn physical_id_from_arn(arn: &str) -> String {
    if arn.is_empty() {
        return String::new();
    }
    if arn.contains('/') {
        if let Some(tail) = arn.rsplit('/').next() {
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }
    if arn.contains(':') {
        if let Some(tail) = arn.rsplit(':').next() {
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }
    arn.to_string()
}

fn resource_type_from_arn(arn: &str) -> String {
    if !arn.starts_with("arn:") {
        return String::new();
    }
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    if parts.len() < 6 {
        return String::new();
    }
    let service = parts[2];
    let resource = parts[5];
    let kind = if resource.contains('/') {
        resource.split('/').next().unwrap_or("")
    } else if resource.contains(':') {
        resource.split(':').next().unwrap_or("")
    } else {
        ""
    };
    if kind.is_empty() {
        service.to_string()
    } else {
        format!("{service}:{kind}")
    }
}

fn region_from_arn(arn: &str) -> String {
    if !arn.starts_with("arn:") {
        return String::new();
    }
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    if parts.len() >= 4 {
        parts[3].to_string()
    } else {
        String::new()
    }
}

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let query = ctx.input_str("query", "").trim().to_string();
    let mut max_results = ctx.input_i64("max_results", DEFAULT_MAX);
    if max_results <= 0 {
        max_results = DEFAULT_MAX;
    }
    let max_results = max_results as usize;

    if query.is_empty() {
        return json!({"render": "reverse_lookup", "query": "", "matches": []});
    }

    let tagging = aws_sdk_resourcegroupstagging::Client::new(&ctx.sdk);
    let cfn = aws_sdk_cloudformation::Client::new(&ctx.sdk);
    let needle = query.to_lowercase();

    // Paginate get_resources, collecting ARN-substring matches with their tags.
    let mut matches_raw: Vec<(String, Value)> = Vec::new();
    let mut token: Option<String> = None;
    let mut scanned: usize = 0;
    let mut pages: usize = 0;
    let mut capped = false;
    let mut result_limited = false;
    let mut page_error = None;
    if let Some(denied) = ctx.preflight("resourcegroupstaggingapi", "GetResources") {
        return denied;
    }
    'pages: loop {
        let mut req = tagging.get_resources();
        if let Some(t) = &token {
            req = req.pagination_token(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                page_error = Some(err_msg(e));
                break;
            }
        };
        pages += 1;
        token = resp
            .pagination_token()
            .filter(|token| !token.is_empty())
            .map(str::to_string);
        for (index, rec) in resp.resource_tag_mapping_list().iter().enumerate() {
            scanned += 1;
            let arn = rec.resource_arn().unwrap_or("").to_string();
            if arn.is_empty() {
                continue;
            }
            if arn.to_lowercase().contains(&needle) {
                let mut tags = Map::new();
                for t in rec.tags() {
                    // resourcegroupstagging Tag has required key/value -> &str.
                    tags.insert(t.key().to_string(), json!(t.value()));
                }
                matches_raw.push((arn, Value::Object(tags)));
                if matches_raw.len() >= max_results {
                    result_limited =
                        index + 1 < resp.resource_tag_mapping_list().len() || token.is_some();
                    break 'pages;
                }
            }
        }
        if token.is_none() {
            break;
        }
        if pages >= MAX_PAGES {
            capped = true;
            break;
        }
    }

    // Enrich the first few matches with their owning stack, concurrently.
    let mut enrich: Vec<String> = matches_raw
        .iter()
        .take(STACK_BUDGET)
        .map(|(arn, _)| arn.clone())
        .collect();
    let mut enrichment_denied = false;
    if page_error.is_some() {
        enrich.clear();
    }
    if !enrich.is_empty()
        && ctx
            .preflight("cloudformation", "DescribeStackResources")
            .is_some()
    {
        enrichment_denied = true;
        enrich.clear();
    }
    let futs = enrich.iter().map(|arn| {
        let cfn = cfn.clone();
        let arn = arn.clone();
        async move {
            let pid = physical_id_from_arn(&arn);
            let stack = if pid.is_empty() {
                Ok(None)
            } else {
                cfn.describe_stack_resources()
                    .physical_resource_id(&pid)
                    .send()
                    .await
                    .map(|r| {
                        r.stack_resources()
                            .first()
                            .and_then(|sr| sr.stack_name())
                            .map(str::to_string)
                    })
                    .map_err(err_msg)
            };
            (arn, stack)
        }
    });
    let stack_by_arn: HashMap<String, Result<Option<String>, String>> =
        join_all(futs).await.into_iter().collect();

    let mut matches = Vec::new();
    for (i, (arn, tags)) in matches_raw.iter().enumerate() {
        let stack = if i < STACK_BUDGET {
            stack_by_arn
                .get(arn)
                .and_then(|result| result.as_ref().ok())
                .cloned()
                .flatten()
        } else {
            None
        };
        matches.push(json!({
            "arn": arn,
            "type": resource_type_from_arn(arn),
            "stack": stack,
            "region": region_from_arn(arn),
            "tags": tags,
        }));
    }

    let mut listing = Coverage::complete(matches.len());
    listing.count("pages", pages);
    listing.count("scanned", scanned);
    listing.limit("results", Some(max_results));
    listing.limit("pages", Some(MAX_PAGES));
    if capped {
        listing.has_more(Some(true));
        listing.limited(
            "page_limit",
            "The resource scan stopped at its page limit; additional resources were not scanned.",
        );
    }
    if result_limited {
        listing.has_more(Some(true));
        listing.limited(
            "result_limit",
            "The match limit was reached before all available resources were scanned.",
        );
    }
    if page_error.is_some() {
        listing.failure(
            "request_failed",
            "A resource-list page could not be loaded; earlier matches are retained.",
            pages > 0,
        );
    }
    let failed = stack_by_arn
        .values()
        .filter(|result| result.is_err())
        .count();
    let mut enrichment = Coverage::complete(stack_by_arn.len() - failed);
    enrichment.count("attempted", enrich.len());
    enrichment.count("failed", failed);
    enrichment.count("skipped", matches.len().saturating_sub(enrich.len()));
    enrichment.limit("matches", Some(STACK_BUDGET));
    if failed > 0 {
        enrichment.failure(
            "enrichment_failed",
            "Some stack associations could not be checked.",
            true,
        );
    }
    if enrichment_denied {
        enrichment.failure(
            "policy_denied",
            "Stack association lookup was not permitted.",
            !matches.is_empty(),
        );
    } else if page_error.is_some() {
        enrichment.unknown_reason(
            "not_attempted",
            "Stack association lookup was not requested after the listing failed.",
        );
    } else if matches.len() > STACK_BUDGET {
        enrichment.limited(
            "enrichment_limit",
            "Stack associations are checked only for the first five matches.",
        );
    }
    let mut coverage = Coverage::complete(matches.len());
    coverage.section("matches", listing);
    coverage.section("enrichment", enrichment);
    let mut result = json!({
        "render": "reverse_lookup",
        "query": query,
        "matches": matches,
        "region": ctx.region,
        "scanned": scanned,
        "capped": capped,
    });
    if let Some(error) = page_error {
        result["error"] = json!(error);
    }
    coverage.attach(result)
}
