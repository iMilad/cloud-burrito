//! Additive evidence coverage. A bounded success is not an execution failure.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Completeness {
    Complete,
    Limited,
    Unknown,
}

#[derive(Serialize)]
struct Reason {
    code: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
pub(crate) struct Coverage {
    completeness: Completeness,
    has_more: Option<bool>,
    counts: BTreeMap<&'static str, usize>,
    limits: BTreeMap<&'static str, Option<usize>>,
    reasons: Vec<Reason>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    sections: BTreeMap<&'static str, Coverage>,
    #[serde(skip)]
    failed: Option<bool>,
    #[serde(skip)]
    truncated: bool,
}

impl Coverage {
    pub(crate) fn complete(returned: usize) -> Self {
        Self {
            completeness: Completeness::Complete,
            has_more: Some(false),
            counts: BTreeMap::from([("returned", returned)]),
            limits: BTreeMap::new(),
            reasons: Vec::new(),
            sections: BTreeMap::new(),
            failed: None,
            truncated: false,
        }
    }

    pub(crate) fn unknown(returned: usize) -> Self {
        let mut result = Self::complete(returned);
        result.completeness = Completeness::Unknown;
        result.has_more = None;
        result
    }

    pub(crate) fn unmeasured() -> Self {
        let mut result = Self::unknown(0);
        result.counts.remove("returned");
        result
    }

    pub(crate) fn count(&mut self, key: &'static str, value: usize) {
        self.counts.insert(key, value);
    }
    pub(crate) fn limit(&mut self, key: &'static str, value: Option<usize>) {
        self.limits.insert(key, value);
    }
    pub(crate) fn has_more(&mut self, value: Option<bool>) {
        self.has_more = value;
    }

    pub(crate) fn limited(&mut self, code: &'static str, message: &'static str) {
        self.completeness = Completeness::Limited;
        self.truncated = true;
        if self.has_more == Some(false) {
            self.has_more = None;
        }
        self.reason(code, message);
    }

    pub(crate) fn unknown_reason(&mut self, code: &'static str, message: &'static str) {
        if matches!(self.completeness, Completeness::Complete) {
            self.completeness = Completeness::Unknown;
        }
        if self.has_more == Some(false) {
            self.has_more = None;
        }
        self.reason(code, message);
    }

    pub(crate) fn failure(
        &mut self,
        code: &'static str,
        message: &'static str,
        retained_evidence: bool,
    ) {
        self.unknown_reason(code, message);
        self.has_more = None;
        self.failed = Some(
            self.failed.unwrap_or(false)
                || retained_evidence
                || self.counts.get("returned").copied().unwrap_or(0) > 0,
        );
    }

    pub(crate) fn section(&mut self, key: &'static str, value: Coverage) {
        if matches!(value.completeness, Completeness::Limited) {
            self.completeness = Completeness::Limited;
        } else if matches!(value.completeness, Completeness::Unknown)
            && matches!(self.completeness, Completeness::Complete)
        {
            self.completeness = Completeness::Unknown;
        }
        self.truncated |= value.truncated;
        if value.has_more == Some(true) {
            self.has_more = Some(true);
        } else if value.has_more.is_none() && self.has_more == Some(false) {
            self.has_more = None;
        }
        if let Some(retained) = value.failed {
            self.failed = Some(
                self.failed.unwrap_or(false)
                    || retained
                    || self.counts.get("returned").copied().unwrap_or(0) > 0,
            );
        }
        self.sections.insert(key, value);
    }

    fn reason(&mut self, code: &'static str, message: &'static str) {
        if !self.reasons.iter().any(|reason| reason.code == code) {
            self.reasons.push(Reason { code, message });
        }
    }

    pub(crate) fn attach(self, mut response: Value) -> Value {
        if self.truncated {
            response["truncated"] = json!(true);
        }
        if let Some(retained) = self.failed {
            response["ok"] = json!(false);
            response["partial"] = json!(retained);
            response["status"] = json!(if retained { "partial" } else { "failed" });
            if response.get("error_type").is_none() {
                response["error_type"] = json!(if retained {
                    "PartialFailure"
                } else {
                    "RequestFailed"
                });
            }
        }
        response["coverage"] =
            serde_json::to_value(self).expect("coverage contains only static text and counts");
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_success_retains_rows_without_becoming_a_request_failure() {
        let mut coverage = Coverage::complete(1);
        coverage.limit("results", Some(1));
        coverage.has_more(Some(true));
        coverage.limited("result_limit", "The requested result limit was reached.");
        let response =
            coverage.attach(json!({"render":"table", "rows":[{"value":"synthetic-evidence"}]}));
        assert_eq!(response["coverage"]["completeness"], "limited");
        assert_eq!(response["truncated"], true);
        assert_eq!(response["rows"][0]["value"], "synthetic-evidence");
        assert_eq!(crate::request::outcome(&response), "succeeded");
    }

    #[test]
    fn failed_section_does_not_discard_successful_sibling_evidence() {
        let mut coverage = Coverage::complete(1);
        let mut events = Coverage::unknown(0);
        events.limit("results", None);
        events.failure("request_failed", "Events could not be loaded.", false);
        coverage.section("events", events);
        let response = coverage.attach(json!({"render":"stack_detail","resources":[{"logical_id":"synthetic-resource"}],"events":[]}));
        assert_eq!(response["partial"], true);
        assert_eq!(response["error_type"], "PartialFailure");
        assert_eq!(response["resources"].as_array().unwrap().len(), 1);
        assert!(response["coverage"]["sections"]["events"]["limits"]["results"].is_null());
        assert_eq!(crate::request::outcome(&response), "failed");
    }
}

#[cfg(test)]
mod producer_tests {
    use crate::test_aws::{ExpectedRequest as Request, ScriptedHttp};
    use crate::test_support::TestDir;
    use crate::widgets::{
        cfn_stack_detail, cfn_stacks, codeartifact_packages, pipeline_execution_detail,
        pipeline_runs, resource_lookup,
    };
    use serde_json::{json, Value};

    const PRIVATE: &str = "CB_SYNTHETIC_PRIVATE_SERVICE_DETAIL";
    const TAGGING: &str = "ResourceGroupsTaggingAPI_20170126.GetResources";
    fn xml(action: &str, body: &str) -> String {
        format!("<{action}Response xmlns=\"http://cloudformation.amazonaws.com/doc/2010-05-15/\"><{action}Result>{body}</{action}Result><ResponseMetadata><RequestId>synthetic-request</RequestId></ResponseMetadata></{action}Response>")
    }
    fn xml_failure(action: &'static str, fields: Value) -> Request {
        Request::xml(action, fields, &format!("<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>{PRIVATE}</Message></Error><RequestId>synthetic-error</RequestId></ErrorResponse>")).status(403)
    }
    fn stack_member(name: &str) -> String {
        format!("<member><StackName>{name}</StackName><StackId>synthetic-stack-id</StackId><CreationTime>2026-01-01T00:00:00Z</CreationTime><StackStatus>CREATE_COMPLETE</StackStatus></member>")
    }
    fn resource_member(index: usize) -> String {
        format!("<member><StackName>synthetic-stack</StackName><StackId>synthetic-stack-id</StackId><LogicalResourceId>synthetic-resource-{index}</LogicalResourceId><PhysicalResourceId>synthetic-physical-{index}</PhysicalResourceId><ResourceType>AWS::S3::Bucket</ResourceType><ResourceStatus>CREATE_COMPLETE</ResourceStatus><Timestamp>2026-01-01T00:00:00Z</Timestamp></member>")
    }
    fn resource_response(count: usize) -> String {
        xml(
            "DescribeStackResources",
            &format!(
                "<StackResources>{}</StackResources>",
                (0..count).map(resource_member).collect::<String>()
            ),
        )
    }
    fn event_member(index: usize) -> String {
        format!("<member><StackId>synthetic-stack-id</StackId><StackName>synthetic-stack</StackName><EventId>synthetic-event-{index}</EventId><LogicalResourceId>synthetic-resource-{index}</LogicalResourceId><ResourceType>AWS::S3::Bucket</ResourceType><ResourceStatus>CREATE_COMPLETE</ResourceStatus><Timestamp>2026-01-01T00:00:00Z</Timestamp></member>")
    }
    fn package_inputs(max: usize) -> Value {
        json!({"domain":"synthetic-domain","repository":"synthetic-repo","package_prefix":"synthetic","max_packages":max})
    }
    fn package_page(
        token: Option<&str>,
        max: usize,
        packages: Value,
        next: Option<&str>,
    ) -> Request {
        Request::rest(
            "POST",
            "/v1/packages",
            json!({"domain":"synthetic-domain","repository":"synthetic-repo","format":"pypi","max-results":max.to_string(),"next-token":token}),
            json!({"packages":packages,"nextToken":next}),
        )
    }
    fn version_page(package: &str, next: Option<&str>) -> Request {
        Request::rest(
            "POST",
            "/v1/package/versions",
            json!({"package":package,"max-results":"10"}),
            json!({"versions":[{"version":"1.0.0","status":"Published"}],"nextToken":next}),
        )
    }
    fn version_failure(package: &str) -> Request {
        Request::rest(
            "GET",
            "/v1/package/version",
            json!({"package":package,"version":"1.0.0"}),
            json!({"__type":"AccessDeniedException","message":PRIVATE}),
        )
        .status(403)
    }

    #[tokio::test]
    async fn later_stack_page_failure_retains_stacks_without_new_enrichment_requests() {
        let directory = TestDir::new();
        let first = xml(
            "ListStacks",
            &format!(
                "<StackSummaries>{}</StackSummaries><NextToken>synthetic-next</NextToken>",
                stack_member("synthetic-keep")
            ),
        );
        let http = ScriptedHttp::new(vec![
            Request::xml("ListStacks", json!({"NextToken":null}), &first),
            xml_failure("ListStacks", json!({"NextToken":"synthetic-next"})),
        ]);
        let result = cfn_stacks::fetch(&http.context(&directory, "cfn-stacks", json!({}))).await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["rows"][0]["stack"], "synthetic-keep");
        assert_eq!(result["rows"][0]["resources"], "");
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"]["sections"]["stacks"]["counts"]["pages"],
            1
        );
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["attempted"],
            0
        );
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn stack_enrichment_failures_and_twenty_stack_limit_remain_distinct() {
        let directory = TestDir::new();
        let names: Vec<_> = (0..21).map(|i| format!("synthetic-stack-{i}")).collect();
        let body = xml(
            "ListStacks",
            &format!(
                "<StackSummaries>{}</StackSummaries>",
                names
                    .iter()
                    .map(|name| stack_member(name))
                    .collect::<String>()
            ),
        );
        let mut expected = vec![Request::xml("ListStacks", json!({}), &body)];
        for (index, name) in names.iter().take(20).enumerate() {
            expected.push(if index == 0 {
                xml_failure("DescribeStackResources", json!({"StackName":name}))
            } else {
                Request::xml(
                    "DescribeStackResources",
                    json!({"StackName":name}),
                    &resource_response(1),
                )
            });
        }
        let http = ScriptedHttp::new(expected).unordered();
        let result = cfn_stacks::fetch(&http.context(&directory, "cfn-stacks", json!({}))).await;
        http.assert_finished();
        assert_eq!(http.calls(), 21);
        assert_eq!(result["rows"].as_array().unwrap().len(), 21);
        let enrichment = &result["coverage"]["sections"]["enrichment"];
        assert_eq!(enrichment["counts"]["attempted"], 20);
        assert_eq!(enrichment["counts"]["failed"], 1);
        assert_eq!(enrichment["counts"]["skipped"], 1);
        assert_eq!(result["partial"], true);
        assert_eq!(result["truncated"], true);
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn stack_event_failure_and_denial_keep_the_successful_resources() {
        for denied in [false, true] {
            let directory = TestDir::new();
            let mut expected = vec![Request::xml(
                "DescribeStackResources",
                json!({"StackName":"synthetic-stack"}),
                &resource_response(1),
            )];
            if !denied {
                expected.push(xml_failure(
                    "DescribeStackEvents",
                    json!({"StackName":"synthetic-stack"}),
                ));
            }
            let http = ScriptedHttp::new(expected);
            let mut context = http.context(
                &directory,
                "cfn-stack-detail",
                json!({"stack_name":"synthetic-stack"}),
            );
            if denied {
                context.policy = crate::aws::policy::Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: ['cloudformation:DescribeStackEvents']\n").map_err(|error| error.message);
            }
            let result = cfn_stack_detail::fetch(&context).await;
            http.assert_finished();
            assert_eq!(http.calls(), if denied { 1 } else { 2 });
            assert_eq!(result["resources"][0]["logical_id"], "synthetic-resource-0");
            assert_eq!(result["events"], json!([]));
            assert_eq!(result["partial"], true);
            assert_eq!(
                result["coverage"]["sections"]["events"]["reasons"][0]["code"],
                if denied {
                    "policy_denied"
                } else {
                    "request_failed"
                }
            );
            assert!(!result.to_string().contains(PRIVATE));
        }
    }

    #[tokio::test]
    async fn stack_event_cap_and_resource_service_boundary_are_reported_without_more_calls() {
        let directory = TestDir::new();
        let events = xml(
            "DescribeStackEvents",
            &format!(
                "<StackEvents>{}</StackEvents><NextToken>synthetic-next</NextToken>",
                (0..61).map(event_member).collect::<String>()
            ),
        );
        let http = ScriptedHttp::new(vec![
            Request::xml("DescribeStackResources", json!({}), &resource_response(100)),
            Request::xml("DescribeStackEvents", json!({}), &events),
        ]);
        let result = cfn_stack_detail::fetch(&http.context(
            &directory,
            "cfn-stack-detail",
            json!({"stack_name":"synthetic-stack"}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["resources"].as_array().unwrap().len(), 100);
        assert_eq!(result["events"].as_array().unwrap().len(), 60);
        assert_eq!(
            result["coverage"]["sections"]["resources"]["completeness"],
            "unknown"
        );
        assert_eq!(result["coverage"]["sections"]["events"]["has_more"], true);
        assert_eq!(crate::request::outcome(&result), "succeeded");
    }

    #[tokio::test]
    async fn pipeline_one_page_tokens_report_unloaded_evidence_and_preserve_build_ids() {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![
            Request::json(
                "CodePipeline_20150709.ListPipelineExecutions",
                json!({"pipelineName":"synthetic-pipeline","maxResults":1}),
                json!({"pipelineExecutionSummaries":[{"pipelineExecutionId":"synthetic-execution","status":"Failed"}],"nextToken":"synthetic-next"}),
            ),
            Request::json(
                "CodePipeline_20150709.ListActionExecutions",
                json!({"pipelineName":"synthetic-pipeline","filter":{"pipelineExecutionId":"synthetic-execution"}}),
                json!({"actionExecutionDetails":[{"stageName":"Build","actionName":"synthetic-build","status":"Failed","output":{"executionResult":{"externalExecutionId":"synthetic-project:synthetic-build"}}}],"nextToken":"synthetic-actions"}),
            ),
        ]);
        let runs = pipeline_runs::fetch(&http.context(
            &directory,
            "pipeline-runs",
            json!({"pipeline_name":"synthetic-pipeline","max_results":1}),
        ))
        .await;
        let actions = pipeline_execution_detail::fetch(&http.context(
            &directory,
            "pipeline-execution-detail",
            json!({"pipeline_name":"synthetic-pipeline","execution_id":"synthetic-execution"}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(runs["rows"][0]["execution_id"], "synthetic-execution");
        assert_eq!(runs["coverage"]["limits"]["results"], 1);
        assert_eq!(runs["coverage"]["has_more"], true);
        assert_eq!(
            actions["actions"][0]["external_execution_id"],
            "synthetic-project:synthetic-build"
        );
        assert!(actions["coverage"]["limits"]["results"].is_null());
        assert_eq!(actions["coverage"]["has_more"], true);
        assert_eq!(crate::request::outcome(&runs), "succeeded");
    }

    #[tokio::test]
    async fn lookup_failed_second_page_keeps_matches_and_does_not_start_enrichment() {
        let directory = TestDir::new();
        let arn = "arn:aws:s3:::synthetic-bucket";
        let http = ScriptedHttp::new(vec![
            Request::json(
                TAGGING,
                json!({"PaginationToken":null}),
                json!({"ResourceTagMappingList":[{"ResourceARN":arn,"Tags":[]}],"PaginationToken":"synthetic-next"}),
            ),
            Request::json(
                TAGGING,
                json!({"PaginationToken":"synthetic-next"}),
                json!({"__type":"AccessDeniedException","message":PRIVATE}),
            )
            .status(403),
        ]);
        let result = resource_lookup::fetch(&http.context(
            &directory,
            "resource-lookup",
            json!({"query":"synthetic","max_results":10}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["matches"][0]["arn"], arn);
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"]["sections"]["matches"]["counts"]["pages"],
            1
        );
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["attempted"],
            0
        );
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn lookup_result_limit_counts_the_page_and_distinguishes_exact_end_from_unscanned_items()
    {
        for extra in [false, true] {
            let directory = TestDir::new();
            let mut mappings = vec![json!({"ResourceARN":"arn:aws:s3:::synthetic-one","Tags":[]})];
            if extra {
                mappings.push(json!({"ResourceARN":"arn:aws:s3:::synthetic-two","Tags":[]}));
            }
            let http = ScriptedHttp::new(vec![
                Request::json(
                    TAGGING,
                    json!({}),
                    json!({"ResourceTagMappingList":mappings}),
                ),
                Request::xml(
                    "DescribeStackResources",
                    json!({"PhysicalResourceId":"synthetic-one"}),
                    &resource_response(0),
                ),
            ]);
            let result = resource_lookup::fetch(&http.context(
                &directory,
                "resource-lookup",
                json!({"query":"synthetic","max_results":1}),
            ))
            .await;
            http.assert_finished();
            let listing = &result["coverage"]["sections"]["matches"];
            assert_eq!(listing["counts"]["pages"], 1);
            assert_eq!(listing["counts"]["scanned"], 1);
            assert_eq!(listing["has_more"], extra);
            assert_eq!(
                listing["completeness"],
                if extra { "limited" } else { "complete" }
            );
            assert_eq!(crate::request::outcome(&result), "succeeded");
        }
    }

    #[tokio::test]
    async fn lookup_page_budget_is_visible_even_with_no_matching_rows() {
        let directory = TestDir::new();
        let expected = (0..25).map(|index| Request::json(TAGGING,
            json!({"PaginationToken":if index == 0 { None } else { Some(format!("synthetic-page-{index}")) }}),
            json!({"ResourceTagMappingList":[],"PaginationToken":format!("synthetic-page-{}",index+1)}))).collect();
        let http = ScriptedHttp::new(expected);
        let result = resource_lookup::fetch(&http.context(
            &directory,
            "resource-lookup",
            json!({"query":"synthetic-no-match"}),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 25);
        assert_eq!(result["matches"], json!([]));
        assert_eq!(
            result["coverage"]["sections"]["matches"]["counts"]["pages"],
            25
        );
        assert_eq!(result["coverage"]["sections"]["matches"]["has_more"], true);
        assert_eq!(result["truncated"], true);
    }

    #[tokio::test]
    async fn package_failed_second_page_keeps_names_without_new_version_requests() {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![
            package_page(
                None,
                3,
                json!([{"package":"synthetic-package","format":"pypi"}]),
                Some("synthetic-next"),
            ),
            Request::rest(
                "POST",
                "/v1/packages",
                json!({"next-token":"synthetic-next","max-results":"2"}),
                json!({"__type":"AccessDeniedException","message":PRIVATE}),
            )
            .status(403),
        ]);
        let result = codeartifact_packages::fetch(&http.context(
            &directory,
            "codeartifact-packages",
            package_inputs(3),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["rows"][0]["package"], "synthetic-package");
        assert_eq!(result["rows"][0]["latest_version"], "");
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"]["sections"]["packages"]["counts"]["pages"],
            1
        );
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["attempted"],
            0
        );
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn package_and_version_caps_keep_versions_when_date_enrichment_fails() {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![
            package_page(
                None,
                1,
                json!([{"package":"synthetic-package","format":"pypi"}]),
                Some("synthetic-next"),
            ),
            version_page("synthetic-package", Some("synthetic-more-versions")),
            version_failure("synthetic-package"),
        ]);
        let result = codeartifact_packages::fetch(&http.context(
            &directory,
            "codeartifact-packages",
            package_inputs(1),
        ))
        .await;
        http.assert_finished();
        assert_eq!(http.calls(), 3);
        assert_eq!(result["rows"][0]["latest_version"], "1.0.0");
        assert_eq!(result["rows"][0]["versions"], json!(["1.0.0"]));
        assert_eq!(result["partial"], true);
        assert_eq!(result["coverage"]["sections"]["packages"]["has_more"], true);
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["limited_version_lists"],
            1
        );
        assert!(!result.to_string().contains(PRIVATE));
    }

    #[tokio::test]
    async fn package_date_denial_retains_fetched_versions_and_stops_further_enrichment() {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![
            package_page(
                None,
                2,
                json!([{"package":"synthetic-one","format":"pypi"},{"package":"synthetic-two","format":"pypi"}]),
                None,
            ),
            version_page("synthetic-one", None),
        ]);
        let mut context = http.context(&directory, "codeartifact-packages", package_inputs(2));
        context.policy = crate::aws::policy::Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: ['codeartifact:DescribePackageVersion']\n").map_err(|error| error.message);
        let result = codeartifact_packages::fetch(&context).await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["rows"][0]["versions"], json!(["1.0.0"]));
        assert_eq!(result["rows"][1]["package"], "synthetic-two");
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"]["sections"]["enrichment"]["counts"]["skipped"],
            1
        );
    }

    #[tokio::test]
    async fn version_history_real_date_failure_keeps_known_and_failed_version_rows() {
        let directory = TestDir::new();
        let http = ScriptedHttp::new(vec![version_failure("synthetic-package")]);
        let context = http.context(&directory, "codeartifact-packages", json!({"mode":"versions","domain":"synthetic-domain","repository":"synthetic-repo","package":"synthetic-package",
            "versions":[{"version":"2.0.0","published":"2026-01-01T00:00:00Z"},"1.0.0"]}));
        let result = codeartifact_packages::fetch_version_history(&context).await;
        http.assert_finished();
        assert_eq!(http.calls(), 1);
        assert_eq!(result["versions"][0]["version"], "2.0.0");
        assert_eq!(result["versions"][0]["published"], "2026-01-01T00:00:00Z");
        assert_eq!(result["versions"][1]["version"], "1.0.0");
        assert_eq!(result["partial"], true);
        assert_eq!(result["coverage"]["counts"]["failed"], 1);
        assert!(!result.to_string().contains(PRIVATE));
    }
}
