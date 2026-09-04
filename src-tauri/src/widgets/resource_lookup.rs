//! Resource Reverse Lookup — find resources whose ARN matches a substring via
//! the Resource Groups Tagging API, then best-effort enrich the first few with
//! a CloudFormation stack only when exact resource evidence confirms it.

use std::collections::{BTreeSet, HashMap};

use futures::future::join_all;
use serde_json::{json, Map, Value};

use super::coverage::Coverage;
use super::handoff::{self, Source};
use super::{err_msg, WidgetCtx};

const DEFAULT_MAX: i64 = 25;
const STACK_BUDGET: usize = 5;
// Cap how many GetResources pages we scan so a query that matches nothing
// doesn't crawl an entire large account (~100 resources per page).
const MAX_PAGES: usize = 25;

/// A closed mapping from a resource ARN to the physical identifier that
/// CloudFormation reports for that resource type. An arbitrary ARN tail is not
/// evidence: unrelated resource families routinely share the same names.
#[derive(Clone)]
struct OwnershipProbe {
    physical_id: String,
    cfn_type: &'static str,
}

fn name_part(value: &str, max: usize, extra: &str) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || extra.as_bytes().contains(&byte))
}

fn ownership_probe(arn: &str, ctx: &WidgetCtx) -> Result<OwnershipProbe, Source> {
    let parts: Vec<_> = arn.splitn(6, ':').collect();
    if parts.len() != 6 || parts[0] != "arn" || parts[1] != "aws" {
        return Err(Source::OwnershipUnsupported);
    }
    // Regionless/global resources do not establish ownership in this context.
    // They remain useful lookup matches, with an explicitly unknown owner.
    if parts[3].is_empty() || parts[4].is_empty() {
        return Err(Source::OwnershipUnsupported);
    }
    if parts[3] != ctx.region || parts[4] != ctx.account_id {
        return Err(Source::ContextMismatch);
    }
    let (physical_id, cfn_type) = match parts[2] {
        "lambda" => {
            let name = parts[5]
                .strip_prefix("function:")
                .filter(|name| name_part(name, 64, "-_"))
                .ok_or(Source::OwnershipUnsupported)?;
            (name, "AWS::Lambda::Function")
        }
        "logs" => {
            let name = parts[5]
                .strip_prefix("log-group:")
                .map(|name| name.strip_suffix(":*").unwrap_or(name))
                .filter(|name| name_part(name, 512, "._-/#"))
                .ok_or(Source::OwnershipUnsupported)?;
            (name, "AWS::Logs::LogGroup")
        }
        "ec2" => {
            let id = parts[5]
                .strip_prefix("instance/")
                .filter(|id| {
                    id.strip_prefix("i-").is_some_and(|suffix| {
                        matches!(suffix.len(), 8 | 17)
                            && suffix
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                })
                .ok_or(Source::OwnershipUnsupported)?;
            (id, "AWS::EC2::Instance")
        }
        _ => return Err(Source::OwnershipUnsupported),
    };
    Ok(OwnershipProbe {
        physical_id: physical_id.into(),
        cfn_type,
    })
}

fn confirmed_stack(
    rows: &[aws_sdk_cloudformation::types::StackResource],
    probe: &OwnershipProbe,
    ctx: &WidgetCtx,
) -> Result<String, Source> {
    let mut owners = BTreeSet::new();
    for row in rows.iter().filter(|row| {
        row.physical_resource_id() == Some(probe.physical_id.as_str())
            && row.resource_type() == Some(probe.cfn_type)
    }) {
        let name = row.stack_name().ok_or(Source::IdentifierUnavailable)?;
        let id = row.stack_id().ok_or(Source::IdentifierUnavailable)?;
        if !handoff::same_context_arn(id, "cloudformation", "stack/", ctx) {
            return Err(Source::ContextMismatch);
        }
        let resource = id.splitn(6, ':').nth(5).unwrap_or("");
        let parts: Vec<_> = resource.split('/').collect();
        if parts.len() != 3
            || parts[0] != "stack"
            || parts[1] != name
            || !name_part(name, 128, "-")
            || !name.as_bytes()[0].is_ascii_alphabetic()
            || !name_part(parts[2], 128, "-")
        {
            return Err(Source::IdentifierUnavailable);
        }
        owners.insert(id.to_string());
    }
    if owners.len() > 1 {
        return Err(Source::OwnershipAmbiguous);
    }
    owners.into_iter().next().ok_or(Source::OwnershipNotFound)
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

    // Only inspect identifiers whose physical-ID semantics are known. Keep the
    // existing five-match budget; unsupported matches do not expand that budget.
    let probes: HashMap<_, _> = matches_raw
        .iter()
        .take(STACK_BUDGET)
        .map(|(arn, _)| (arn.clone(), ownership_probe(arn, ctx)))
        .collect();
    let mut enrich: Vec<_> = matches_raw
        .iter()
        .take(STACK_BUDGET)
        .filter_map(|(arn, _)| {
            probes
                .get(arn)
                .and_then(|probe| probe.as_ref().ok())
                .map(|probe| (arn.clone(), probe.clone()))
        })
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
    let futs = enrich.iter().map(|(arn, probe)| {
        let cfn = cfn.clone();
        async move {
            let stack = cfn
                .describe_stack_resources()
                .physical_resource_id(&probe.physical_id)
                .send()
                .await
                .map(|response| confirmed_stack(response.stack_resources(), probe, ctx))
                .map_err(|_| Source::OwnershipFailed);
            (arn.clone(), stack)
        }
    });
    let stack_by_arn: HashMap<_, _> = join_all(futs).await.into_iter().collect();

    let mut matches = Vec::new();
    let mut confirmed = 0;
    for (i, (arn, tags)) in matches_raw.iter().enumerate() {
        let association = if let Some(result) = stack_by_arn.get(arn) {
            match result {
                Ok(Ok(stack)) => Ok(stack.clone()),
                Ok(Err(source)) | Err(source) => Err(*source),
            }
        } else if i >= STACK_BUDGET || page_error.is_some() {
            Err(Source::OwnershipNotAttempted)
        } else if let Some(Err(source)) = probes.get(arn) {
            Err(*source)
        } else if enrichment_denied {
            Err(Source::OwnershipDenied)
        } else {
            Err(Source::OwnershipNotAttempted)
        };
        let (stack, target) = match association {
            Ok(stack) => {
                confirmed += 1;
                let target = handoff::stack(&stack, Source::CfnOwnership);
                (Some(stack), target)
            }
            Err(source) => (None, handoff::unavailable(source)),
        };
        matches.push(json!({
            "arn": arn,
            "type": resource_type_from_arn(arn),
            "stack": stack,
            "region": region_from_arn(arn),
            "tags": tags,
            "handoffs": {"stack": target},
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
    let mut enrichment = Coverage::complete(confirmed);
    enrichment.count("confirmed", confirmed);
    enrichment.count("unknown", matches.len().saturating_sub(confirmed));
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
    if confirmed < matches.len() {
        enrichment.unknown_reason(
            "ownership_unknown",
            "Some resources have no confirmed stack association in the selected context.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_aws::{ExpectedRequest as Request, ScriptedHttp};
    use crate::test_support::TestDir;

    const TAGGING: &str = "ResourceGroupsTaggingAPI_20170126.GetResources";
    const PHYSICAL: &str = "synthetic-function";
    const PRIVATE: &str = "synthetic-private-provider-detail";

    fn account() -> String {
        format!("{:012}", 7)
    }

    fn arn(service: &str, resource: &str) -> String {
        format!("arn:aws:{service}:us-east-1:{}:{resource}", account())
    }

    fn stack_id(name: &str) -> String {
        arn(
            "cloudformation",
            &format!("stack/{name}/synthetic-stack-id"),
        )
    }

    fn mapping(resources: &[String], next: Option<&str>) -> Request {
        Request::json(
            TAGGING,
            json!({"PaginationToken":null}),
            json!({"ResourceTagMappingList":resources.iter().map(|arn| json!({"ResourceARN":arn,"Tags":[{"Key":"fixture","Value":"synthetic"}]})).collect::<Vec<_>>(),"PaginationToken":next}),
        )
    }

    fn member(physical: Option<&str>, kind: &str, stack: &str, id: Option<&str>) -> String {
        format!(
            "<member><LogicalResourceId>SyntheticResource</LogicalResourceId>{}<ResourceType>{kind}</ResourceType><StackName>{stack}</StackName>{}</member>",
            physical.map(|value| format!("<PhysicalResourceId>{value}</PhysicalResourceId>")).unwrap_or_default(),
            id.map(|value| format!("<StackId>{value}</StackId>")).unwrap_or_default(),
        )
    }

    fn describe(physical: &str, rows: &[String]) -> Request {
        Request::xml(
            "DescribeStackResources",
            json!({"PhysicalResourceId":physical,"StackName":null}),
            &format!("<DescribeStackResourcesResponse xmlns=\"http://cloudformation.amazonaws.com/doc/2010-05-15/\"><DescribeStackResourcesResult><StackResources>{}</StackResources></DescribeStackResourcesResult></DescribeStackResourcesResponse>", rows.join("")),
        )
    }

    async fn run(http: &ScriptedHttp, directory: &TestDir) -> Value {
        let mut context = http.context(
            directory,
            "resource-lookup",
            json!({"query":"synthetic","max_results":10}),
        );
        context.account_id = account();
        let result = fetch(&context).await;
        http.assert_finished();
        result
    }

    fn unknown(result: &Value, index: usize, source: &str) {
        let item = &result["matches"][index];
        assert!(item["stack"].is_null());
        let target = &item["handoffs"]["stack"];
        assert_eq!(target["status"], "unavailable");
        assert_eq!(target["source"], source);
        assert!(target["widget"].is_null());
        assert_eq!(target["inputs"], json!({}));
        assert_eq!(item["tags"]["fixture"], "synthetic");
    }

    #[tokio::test]
    async fn exact_resource_match_ignores_unrelated_first_row_and_preserves_stack_arn() {
        let directory = TestDir::new();
        let resource = arn("lambda", &format!("function:{PHYSICAL}"));
        let expected_stack = stack_id("synthetic-owner");
        let rows = vec![
            member(
                Some("synthetic-unrelated"),
                "AWS::Lambda::Function",
                "synthetic-wrong",
                Some(&stack_id("synthetic-wrong")),
            ),
            member(
                Some(PHYSICAL),
                "AWS::Lambda::Function",
                "synthetic-owner",
                Some(&expected_stack),
            ),
        ];
        let http = ScriptedHttp::new(vec![
            mapping(std::slice::from_ref(&resource), None),
            describe(PHYSICAL, &rows),
        ]);
        let result = run(&http, &directory).await;
        assert_eq!(http.calls(), 2);
        let item = &result["matches"][0];
        assert_eq!(item["arn"], resource);
        assert_eq!(item["stack"], expected_stack);
        assert_eq!(item["handoffs"]["stack"]["source"], "cfn_ownership");
        assert_eq!(item["handoffs"]["stack"]["status"], "available");
        assert_eq!(
            item["handoffs"]["stack"]["inputs"],
            json!({"stack_name":expected_stack})
        );
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["confirmed"],
            1
        );
    }

    #[tokio::test]
    async fn wrong_type_name_collision_and_missing_physical_id_cannot_establish_ownership() {
        for rows in [
            vec![member(
                Some(PHYSICAL),
                "AWS::SNS::Topic",
                "synthetic-owner",
                Some(&stack_id("synthetic-owner")),
            )],
            vec![member(
                None,
                "AWS::Lambda::Function",
                "synthetic-owner",
                Some(&stack_id("synthetic-owner")),
            )],
            vec![],
        ] {
            let directory = TestDir::new();
            let resource = arn("lambda", &format!("function:{PHYSICAL}"));
            let http =
                ScriptedHttp::new(vec![mapping(&[resource], None), describe(PHYSICAL, &rows)]);
            let result = run(&http, &directory).await;
            unknown(&result, 0, "ownership_not_found");
            assert_eq!(
                result["coverage"]["sections"]["enrichment"]["counts"]["attempted"],
                1
            );
            assert_eq!(crate::request::outcome(&result), "succeeded");
        }
    }

    #[tokio::test]
    async fn inconsistent_or_multiple_stack_owners_do_not_create_a_target() {
        let cases = [
            (
                vec![
                    member(
                        Some(PHYSICAL),
                        "AWS::Lambda::Function",
                        "synthetic-one",
                        Some(&stack_id("synthetic-one")),
                    ),
                    member(
                        Some(PHYSICAL),
                        "AWS::Lambda::Function",
                        "synthetic-two",
                        Some(&stack_id("synthetic-two")),
                    ),
                ],
                "ownership_ambiguous",
            ),
            (
                vec![member(
                    Some(PHYSICAL),
                    "AWS::Lambda::Function",
                    "synthetic-wrong-name",
                    Some(&stack_id("synthetic-one")),
                )],
                "identifier_unavailable",
            ),
            (
                vec![member(
                    Some(PHYSICAL),
                    "AWS::Lambda::Function",
                    "synthetic-one",
                    None,
                )],
                "identifier_unavailable",
            ),
            (
                vec![member(
                    Some(PHYSICAL),
                    "AWS::Lambda::Function",
                    "synthetic-one",
                    Some(&stack_id("synthetic-one").replace("us-east-1", "eu-west-1")),
                )],
                "context_mismatch",
            ),
            (
                vec![member(
                    Some(PHYSICAL),
                    "AWS::Lambda::Function",
                    "synthetic-one",
                    Some(&stack_id("synthetic-one").replace(&account(), &format!("{:012}", 8))),
                )],
                "context_mismatch",
            ),
        ];
        for (rows, reason) in cases {
            let directory = TestDir::new();
            let resource = arn("lambda", &format!("function:{PHYSICAL}"));
            let http =
                ScriptedHttp::new(vec![mapping(&[resource], None), describe(PHYSICAL, &rows)]);
            let result = run(&http, &directory).await;
            unknown(&result, 0, reason);
        }
    }

    #[tokio::test]
    async fn foreign_and_unreviewed_resource_arns_remain_matches_without_stack_requests() {
        let lambda = arn("lambda", &format!("function:{PHYSICAL}"));
        let cases = [
            (lambda.replace("us-east-1", "eu-west-1"), "context_mismatch"),
            (
                lambda.replace(&account(), &format!("{:012}", 8)),
                "context_mismatch",
            ),
            (
                arn("unreviewed", "name/synthetic-function"),
                "ownership_unsupported",
            ),
            (
                "arn:aws:s3:::synthetic-function".into(),
                "ownership_unsupported",
            ),
            (format!("{lambda}:synthetic-alias"), "ownership_unsupported"),
            ("synthetic-not-an-arn".into(), "ownership_unsupported"),
        ];
        for (resource, reason) in cases {
            let directory = TestDir::new();
            let http = ScriptedHttp::new(vec![mapping(std::slice::from_ref(&resource), None)]);
            let result = run(&http, &directory).await;
            assert_eq!(http.calls(), 1);
            assert_eq!(result["matches"][0]["arn"], resource);
            unknown(&result, 0, reason);
            assert_eq!(
                result["coverage"]["sections"]["enrichment"]["counts"]["attempted"],
                0
            );
        }
    }

    #[tokio::test]
    async fn log_group_mapping_preserves_the_full_physical_name_and_ec2_keeps_the_instance_id() {
        let directory = TestDir::new();
        let group = "/synthetic/shared/name";
        let instance = "i-00000000000000007";
        let expected_stack = stack_id("synthetic-owner");
        let resources = vec![
            arn("logs", &format!("log-group:{group}:*")),
            arn("ec2", &format!("instance/{instance}")),
        ];
        // Match both fixture resource families through the shared ARN prefix.
        let http = ScriptedHttp::new(vec![
            mapping(&resources, None),
            describe(
                group,
                &[member(
                    Some(group),
                    "AWS::Logs::LogGroup",
                    "synthetic-owner",
                    Some(&expected_stack),
                )],
            ),
            describe(
                instance,
                &[member(
                    Some(instance),
                    "AWS::EC2::Instance",
                    "synthetic-owner",
                    Some(&expected_stack),
                )],
            ),
        ])
        .unordered();
        let mut context = http.context(
            &directory,
            "resource-lookup",
            json!({"query":"arn:aws:","max_results":10}),
        );
        context.account_id = account();
        let result = fetch(&context).await;
        http.assert_finished();
        assert_eq!(http.calls(), 3);
        assert_eq!(result["matches"][0]["stack"], expected_stack);
        assert_eq!(result["matches"][1]["stack"], expected_stack);
    }

    #[tokio::test]
    async fn ownership_request_failure_retains_resource_and_reports_partial_without_provider_text()
    {
        let directory = TestDir::new();
        let resource = arn("lambda", &format!("function:{PHYSICAL}"));
        let error = Request::xml(
            "DescribeStackResources",
            json!({"PhysicalResourceId":PHYSICAL}),
            &format!("<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>{PRIVATE}</Message></Error></ErrorResponse>"),
        ).status(403);
        let http = ScriptedHttp::new(vec![mapping(&[resource], None), error]);
        let result = run(&http, &directory).await;
        unknown(&result, 0, "ownership_failed");
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["failed"],
            1
        );
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn policy_denial_and_later_listing_failure_skip_ownership_without_discarding_matches() {
        let resource = arn("lambda", &format!("function:{PHYSICAL}"));
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![mapping(std::slice::from_ref(&resource), None)]);
        let mut context = http.context(&directory, "resource-lookup", json!({"query":"synthetic"}));
        context.account_id = account();
        context.policy = crate::aws::policy::Policy::parse("statements:\n  - effect: Allow\n    action: ['sso:GetRoleCredentials', 'resourcegroupstaggingapi:GetResources']\n").map_err(|error| error.message);
        let denied = fetch(&context).await;
        http.assert_finished();
        unknown(&denied, 0, "ownership_denied");
        assert_eq!(denied["partial"], true);

        let http = ScriptedHttp::new(vec![
            mapping(&[resource], Some("synthetic-next")),
            Request::json(
                TAGGING,
                json!({"PaginationToken":"synthetic-next"}),
                json!({"__type":"AccessDeniedException","message":PRIVATE}),
            )
            .status(403),
        ]);
        let partial = run(&http, &directory).await;
        unknown(&partial, 0, "ownership_not_attempted");
        assert_eq!(partial["partial"], true);
        assert_eq!(http.calls(), 2);
        assert!(!partial.to_string().contains(PRIVATE));
    }
}
