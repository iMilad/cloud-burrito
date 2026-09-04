//! Fixed local producer bounds. Tokens and returned payloads never enter diagnostics.

use std::{
    collections::HashSet,
    io::{self, Write},
};

use serde_json::Value;

use super::coverage::Coverage;

pub(crate) const MAX_PAGES: usize = 25;
pub(crate) const MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;

#[derive(Default)]
pub(crate) struct PageBudget {
    pages: usize,
    bytes: usize,
    tokens: HashSet<String>,
    stopped: Option<Stop>,
}

#[derive(Clone, Copy)]
enum Stop {
    Pages,
    Token,
    Bytes,
}

// Count the encoded value without allocating a second payload-sized buffer.
struct ByteCounter {
    written: usize,
    remaining: usize,
}
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "result byte limit",
            ));
        }
        self.written += bytes.len();
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PageBudget {
    /// Record one completed response, then decide whether another page may run.
    /// Empty pages consume a page just as a non-empty response does.
    pub(crate) fn advance(&mut self, next: Option<&str>) -> bool {
        self.pages += 1;
        if self.stopped.is_some() {
            return false;
        }
        let Some(token) = next.filter(|token| !token.is_empty()) else {
            return false;
        };
        if token.len() > MAX_TOKEN_BYTES || !self.tokens.insert(token.to_string()) {
            self.stopped = Some(Stop::Token);
            return false;
        }
        if self.pages >= MAX_PAGES {
            self.stopped = Some(Stop::Pages);
            return false;
        }
        self.stopped.is_none()
    }

    /// Bound retained JSON values without altering an individual value. The SDK
    /// response and its decode allocation have separate transport bounds.
    pub(crate) fn retain(&mut self, item: &Value) -> bool {
        let mut writer = ByteCounter {
            written: 0,
            remaining: MAX_RETAINED_BYTES.saturating_sub(self.bytes),
        };
        if serde_json::to_writer(&mut writer, item).is_err() {
            self.stopped = Some(Stop::Bytes);
            return false;
        }
        self.bytes += writer.written;
        true
    }

    pub(crate) fn apply(&self, coverage: &mut Coverage, retained: bool) {
        coverage.count("pages", self.pages);
        coverage.count("retained_item_bytes", self.bytes);
        coverage.limit("pages", Some(MAX_PAGES));
        coverage.limit("retained_item_bytes", Some(MAX_RETAINED_BYTES));
        match self.stopped {
            Some(Stop::Pages) => coverage.limited(
                "page_limit", "The local page limit stopped this scan; additional results may exist.",
            ),
            Some(Stop::Token) => coverage.failure(
                "pagination_stalled", "Pagination returned a repeated or oversized continuation token; earlier results are retained.", retained,
            ),
            Some(Stop::Bytes) => coverage.limited(
                "result_byte_limit", "The local result byte limit stopped this scan; additional results may exist.",
            ),
            None => {},
        }
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_pages_and_non_adjacent_token_cycles_are_bounded() {
        let mut budget = PageBudget::default();
        assert!(budget.advance(Some("fixture-a")));
        assert!(budget.advance(Some("fixture-b")));
        assert!(!budget.advance(Some("fixture-a")));
        let mut coverage = Coverage::complete(0);
        budget.apply(&mut coverage, false);
        let result = coverage.attach(json!({"rows":[]}));
        assert_eq!(result["coverage"]["counts"]["pages"], 3);
        assert_eq!(result["status"], "failed");
        assert!(!result.to_string().contains("fixture-a"));

        let mut budget = PageBudget::default();
        for page in 1..=MAX_PAGES {
            assert_eq!(
                budget.advance(Some(&format!("fixture-{page}"))),
                page < MAX_PAGES
            );
        }
        assert!(budget.stopped());
    }

    #[test]
    fn retained_values_are_not_truncated_and_oversized_tokens_are_not_kept() {
        let mut budget = PageBudget::default();
        let item = json!("x".repeat(MAX_RETAINED_BYTES - 2));
        assert!(budget.retain(&item));
        assert!(!budget.retain(&json!(1)));
        assert!(budget.stopped());
        assert!(!PageBudget::default().advance(Some(&"x".repeat(MAX_TOKEN_BYTES + 1))));
    }
}

#[cfg(test)]
mod producer_tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest as Request, ScriptedHttp},
        test_support::TestDir,
        widgets,
    };
    use serde_json::json;

    fn xml(action: &str, body: &str) -> String {
        format!("<{action}Response xmlns=\"http://cloudformation.amazonaws.com/doc/2010-05-15/\"><{action}Result>{body}</{action}Result></{action}Response>")
    }

    #[tokio::test]
    async fn list_producers_stop_on_repeated_empty_pages() {
        for widget in [
            "cfn-stacks",
            "cloudwatch-logs",
            "log-tail",
            "codeartifact-packages",
            "resource-lookup",
        ] {
            let dir = TestDir::new();
            let expected = (0..2)
                .map(|index| {
                    let token = if index == 0 {
                        Value::Null
                    } else {
                        json!("synthetic-next")
                    };
                    match widget {
                        "cfn-stacks" => Request::xml(
                            "ListStacks",
                            json!({"NextToken":token}),
                            &xml(
                                "ListStacks",
                                "<StackSummaries/><NextToken>synthetic-next</NextToken>",
                            ),
                        ),
                        "cloudwatch-logs" => Request::json(
                            "Logs_20140328.DescribeLogGroups",
                            json!({"nextToken":token}),
                            json!({"logGroups":[],"nextToken":"synthetic-next"}),
                        ),
                        "log-tail" => Request::rest(
                            "GET",
                            "/2015-03-31/functions",
                            json!({"Marker":token}),
                            json!({"Functions":[],"NextMarker":"synthetic-next"}),
                        ),
                        "codeartifact-packages" => Request::rest(
                            "POST",
                            "/v1/packages",
                            json!({"next-token":token}),
                            json!({"packages":[],"nextToken":"synthetic-next"}),
                        ),
                        _ => Request::json(
                            "ResourceGroupsTaggingAPI_20170126.GetResources",
                            json!({"PaginationToken":token}),
                            json!({"ResourceTagMappingList":[],"PaginationToken":"synthetic-next"}),
                        ),
                    }
                })
                .collect();
            let script = ScriptedHttp::new(expected);
            let inputs = match widget {
                "log-tail" => json!({"mode":"list"}),
                "resource-lookup" => json!({"query":"synthetic-no-match"}),
                _ => json!({}),
            };
            let result = widgets::fetch(widget, &script.context(&dir, widget, inputs)).await;
            script.assert_finished();
            assert_eq!(script.calls(), 2);
            assert_eq!(result["ok"], false, "{widget}");
            assert!(
                result.to_string().contains("pagination_stalled"),
                "{widget}"
            );
            assert!(!result.to_string().contains("synthetic-next"));
        }
    }

    #[tokio::test]
    async fn stack_list_has_a_strict_result_cap_even_if_a_response_exceeds_it() {
        let dir = TestDir::new();
        let body = format!("<StackSummaries>{}</StackSummaries>", (0..1001).map(|i| format!("<member><StackName>synthetic-{i}</StackName><CreationTime>2026-01-01T00:00:00Z</CreationTime><StackStatus>CREATE_COMPLETE</StackStatus></member>")).collect::<String>());
        let script = ScriptedHttp::new(vec![Request::xml(
            "ListStacks",
            json!({}),
            &xml("ListStacks", &body),
        )]);
        let mut ctx = script.context(&dir, "cfn-stacks", json!({}));
        ctx.policy = crate::aws::policy::Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: ['cloudformation:DescribeStackResources']\n").map_err(|error| error.message);
        let result = widgets::fetch("cfn-stacks", &ctx).await;
        script.assert_finished();
        assert_eq!(result["rows"].as_array().unwrap().len(), 1000);
        assert_eq!(
            result["coverage"]["sections"]["stacks"]["limits"]["results"],
            1000
        );
        assert_eq!(
            result["coverage"]["sections"]["stacks"]["completeness"],
            "limited"
        );
    }

    #[tokio::test]
    async fn log_event_result_byte_cap_keeps_whole_earlier_events() {
        let dir = TestDir::new();
        let message = "x".repeat(700_000);
        let script = ScriptedHttp::new(vec![Request::json(
            "Logs_20140328.GetLogEvents",
            json!({"logGroupName":"/synthetic/group"}),
            json!({"events":[{"message":"synthetic-small"},{"message":message},{"message":message},{"message":message}]}),
        )]);
        let result = widgets::fetch("log-tail", &script.context(&dir,"log-tail",json!({"mode":"events","log_group":"/synthetic/group","log_stream":"synthetic-stream"}))).await;
        script.assert_finished();
        assert_eq!(result["events"].as_array().unwrap().len(), 3);
        assert_eq!(result["events"][0]["msg"], "synthetic-small");
        assert_eq!(result["events"][1]["msg"].as_str().unwrap().len(), 700_000);
        assert!(
            result["coverage"]["counts"]["retained_item_bytes"]
                .as_u64()
                .unwrap()
                <= MAX_RETAINED_BYTES as u64
        );
        assert!(result.to_string().contains("result_byte_limit"));
    }

    #[tokio::test]
    async fn codebuild_non_adjacent_empty_token_cycle_is_not_a_complete_stream() {
        let dir = TestDir::new();
        let mut expected = vec![Request::json(
            "CodeBuild_20161006.BatchGetBuilds",
            json!({"ids":["synthetic-build:run"]}),
            json!({"builds":[{"id":"synthetic-build:run","logs":{"groupName":"/synthetic/build","streamName":"synthetic-stream"}}]}),
        )];
        for (current, next) in [
            (None, "synthetic-a"),
            (Some("synthetic-a"), "synthetic-b"),
            (Some("synthetic-b"), "synthetic-a"),
        ] {
            expected.push(Request::json(
                "Logs_20140328.GetLogEvents",
                json!({"nextToken":current}),
                json!({"events":[],"nextForwardToken":next}),
            ));
        }
        let script = ScriptedHttp::new(expected);
        let result = widgets::fetch(
            "codebuild-log",
            &script.context(
                &dir,
                "codebuild-log",
                json!({"build_id":"synthetic-build:run"}),
            ),
        )
        .await;
        script.assert_finished();
        assert_eq!(result["coverage"]["counts"]["pages"], 3);
        assert_eq!(result["ok"], false);
        assert!(result.to_string().contains("pagination_stalled"));
    }
}
