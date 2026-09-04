//! Exact operation registry shared by SDK policy gates and CLI command parsing.
//!
//! Operation names do not establish safety. Only reviewed service/operation
//! pairs are admitted, and query control and credentials remain app-owned paths.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEffect {
    ResourceRead,
    QueryStart,
    QueryStop,
    Credential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationSpec {
    pub service: &'static str,
    pub operation: &'static str,
    pub effect: OperationEffect,
    pub cli_command: Option<&'static str>,
}

const fn spec(
    service: &'static str,
    operation: &'static str,
    effect: OperationEffect,
    cli_command: Option<&'static str>,
) -> OperationSpec {
    OperationSpec {
        service,
        operation,
        effect,
        cli_command,
    }
}

use OperationEffect::{Credential, QueryStart, QueryStop, ResourceRead};

/// The existing 21 application operations, in default-policy action order.
/// Credential acquisition is a provider preflight proxy, not an interception of
/// every provider request. CLI mappings exist only for the 18 resource reads.
pub const APP_OPS: &[OperationSpec] = &[
    spec("sso", "GetRoleCredentials", Credential, None),
    spec(
        "sts",
        "GetCallerIdentity",
        ResourceRead,
        Some("get-caller-identity"),
    ),
    spec(
        "cloudformation",
        "ListStacks",
        ResourceRead,
        Some("list-stacks"),
    ),
    spec(
        "cloudformation",
        "DescribeStackResources",
        ResourceRead,
        Some("describe-stack-resources"),
    ),
    spec(
        "cloudformation",
        "DescribeStackEvents",
        ResourceRead,
        Some("describe-stack-events"),
    ),
    spec(
        "logs",
        "DescribeLogGroups",
        ResourceRead,
        Some("describe-log-groups"),
    ),
    spec("logs", "StartQuery", QueryStart, None),
    spec(
        "logs",
        "GetQueryResults",
        ResourceRead,
        Some("get-query-results"),
    ),
    spec("logs", "StopQuery", QueryStop, None),
    spec(
        "logs",
        "FilterLogEvents",
        ResourceRead,
        Some("filter-log-events"),
    ),
    spec(
        "codepipeline",
        "ListPipelines",
        ResourceRead,
        Some("list-pipelines"),
    ),
    spec(
        "codepipeline",
        "ListPipelineExecutions",
        ResourceRead,
        Some("list-pipeline-executions"),
    ),
    spec(
        "codepipeline",
        "ListActionExecutions",
        ResourceRead,
        Some("list-action-executions"),
    ),
    spec(
        "codebuild",
        "BatchGetBuilds",
        ResourceRead,
        Some("batch-get-builds"),
    ),
    spec(
        "codeartifact",
        "ListPackages",
        ResourceRead,
        Some("list-packages"),
    ),
    spec(
        "codeartifact",
        "ListPackageVersions",
        ResourceRead,
        Some("list-package-versions"),
    ),
    spec(
        "codeartifact",
        "DescribePackageVersion",
        ResourceRead,
        Some("describe-package-version"),
    ),
    spec(
        "lambda",
        "ListFunctions",
        ResourceRead,
        Some("list-functions"),
    ),
    spec("logs", "GetLogEvents", ResourceRead, Some("get-log-events")),
    spec(
        "logs",
        "DescribeLogStreams",
        ResourceRead,
        Some("describe-log-streams"),
    ),
    spec(
        "resourcegroupstaggingapi",
        "GetResources",
        ResourceRead,
        Some("get-resources"),
    ),
];

/// SDK services retain case-insensitive matching; operation spelling is exact.
pub fn operation(service: &str, operation: &str) -> Option<&'static OperationSpec> {
    APP_OPS
        .iter()
        .find(|spec| spec.service.eq_ignore_ascii_case(service) && spec.operation == operation)
}

/// CLI service and subcommand spellings must exactly match a reviewed mapping.
pub fn cli_operation(service: &str, subcommand: &str) -> Option<&'static OperationSpec> {
    APP_OPS.iter().find(|spec| {
        spec.service == service
            && spec.effect == ResourceRead
            && spec.cli_command == Some(subcommand)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn manifest_has_unique_operations_and_only_read_cli_mappings() {
        assert_eq!(APP_OPS.len(), 21);
        let mut operations = HashSet::new();
        let mut cli_commands = HashSet::new();
        let mut counts = [0; 4];
        for spec in APP_OPS {
            assert_eq!(spec.service, spec.service.to_ascii_lowercase());
            assert!(operations.insert((spec.service, spec.operation)));
            let index = match spec.effect {
                ResourceRead => 0,
                QueryStart => 1,
                QueryStop => 2,
                Credential => 3,
            };
            counts[index] += 1;
            match (spec.effect, spec.cli_command) {
                (ResourceRead, Some(command)) => {
                    assert!(command.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
                    assert!(cli_commands.insert((spec.service, command)));
                    assert_eq!(cli_operation(spec.service, command), Some(spec));
                }
                (ResourceRead, None) => panic!("every registered read must have its CLI mapping"),
                (_, Some(_)) => panic!("query control and credentials must stay outside CLI"),
                (_, None) => {}
            }
        }
        assert_eq!(counts, [18, 1, 1, 1]);
        assert_eq!(cli_commands.len(), 18);
    }

    #[test]
    fn lookup_requires_exact_pairs_and_explicit_cli_spellings() {
        assert_eq!(
            operation("CloudFormation", "ListStacks"),
            operation("cloudformation", "ListStacks")
        );
        assert!(operation("cloudformation", "liststacks").is_none());
        assert!(operation("logs", "ListStacks").is_none());
        assert!(operation("ecr", "BatchDeleteImage").is_none());
        assert!(operation("ec2", "DescribeInstances").is_none());
        assert!(operation("logs", "StartQueryExtra").is_none());
        assert!(operation("", "").is_none());
        assert!(cli_operation("codebuild", "batch-get-builds").is_some());
        assert!(cli_operation("CodeBuild", "batch-get-builds").is_none());
        assert!(cli_operation("codebuild", "BatchGetBuilds").is_none());
        assert!(cli_operation("codebuild", "batch-get-builds-extra").is_none());
        assert!(cli_operation("logs", "start-query").is_none());
        assert!(cli_operation("logs", "stop-query").is_none());
        assert!(cli_operation("sso", "get-role-credentials").is_none());
    }
}
