//! Resource Reverse Lookup — find resources whose ARN matches a substring via
//! the Resource Groups Tagging API, then best-effort enrich the first few with
//! their owning CloudFormation stack.

use std::collections::HashMap;

use futures::future::join_all;
use serde_json::{json, Map, Value};

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
                return json!({"render": "raw_json", "data": {"error": err_msg(e), "query": query}});
            }
        };
        for rec in resp.resource_tag_mapping_list() {
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
                    break 'pages;
                }
            }
        }
        pages += 1;
        token = resp
            .pagination_token()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if token.is_none() {
            break;
        }
        if pages >= MAX_PAGES {
            capped = true;
            break;
        }
    }

    // Enrich the first few matches with their owning stack, concurrently.
    let enrich: Vec<String> = matches_raw
        .iter()
        .take(STACK_BUDGET)
        .map(|(arn, _)| arn.clone())
        .collect();
    if !enrich.is_empty() {
        if let Some(denied) = ctx.preflight("cloudformation", "DescribeStackResources") {
            return denied;
        }
    }
    let futs = enrich.iter().map(|arn| {
        let cfn = cfn.clone();
        let arn = arn.clone();
        async move {
            let pid = physical_id_from_arn(&arn);
            let stack = if pid.is_empty() {
                None
            } else {
                cfn.describe_stack_resources()
                    .physical_resource_id(&pid)
                    .send()
                    .await
                    .ok()
                    .and_then(|r| {
                        r.stack_resources()
                            .first()
                            .and_then(|sr| sr.stack_name())
                            .map(str::to_string)
                    })
            };
            (arn, stack)
        }
    });
    let stack_by_arn: HashMap<String, Option<String>> = join_all(futs).await.into_iter().collect();

    let mut matches = Vec::new();
    for (i, (arn, tags)) in matches_raw.iter().enumerate() {
        let stack = if i < STACK_BUDGET {
            stack_by_arn.get(arn).cloned().flatten()
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

    json!({
        "render": "reverse_lookup",
        "query": query,
        "matches": matches,
        "region": ctx.region,
        "scanned": scanned,
        "capped": capped,
    })
}
