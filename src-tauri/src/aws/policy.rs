//! User-editable read-only allowlist (`~/.cloud_burrito/policy.yaml`).
//!
//! IAM-statement-style YAML that can only narrow the exact operation registry.
//! Evaluation: explicit Deny > matching Allow > default-deny. Query starts also
//! require the user's policy to allow the registered cleanup operation.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use super::guard::{self, OperationEffect};
use crate::paths::AppPaths;

/// Glob match supporting `*` (any run, incl. empty) and `?` (one char).
/// Case-sensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub effect: Effect,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub statements: Vec<Statement>,
}

/// Human-facing parse/validation failure.
#[derive(Debug, Clone)]
pub struct PolicyError {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatement {
    effect: String,
    #[serde(default)]
    action: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    #[serde(default)]
    statements: Vec<RawStatement>,
}

impl Policy {
    /// Parse YAML text into a validated Policy. Empty document => empty policy
    /// (which denies everything by default). A bare `*` action means "all
    /// operations"; otherwise actions are `service:Action` (globs allowed).
    pub fn parse(text: &str) -> Result<Policy, PolicyError> {
        let yaml = if text.trim().is_empty() {
            "statements: []"
        } else {
            text
        };
        let raw: RawPolicy = serde_yaml::from_str(yaml).map_err(|_| PolicyError {
            message: "Policy text or storage could not be processed".into(),
        })?;
        let mut statements = Vec::new();
        for (i, st) in raw.statements.into_iter().enumerate() {
            let effect = match st.effect.to_ascii_lowercase().as_str() {
                "allow" => Effect::Allow,
                "deny" => Effect::Deny,
                _ => {
                    return Err(PolicyError {
                        message: format!("statement {}: effect must be Allow or Deny", i + 1),
                    })
                }
            };
            for a in &st.action {
                if a != "*" && !a.contains(':') {
                    return Err(PolicyError {
                        message: format!(
                            "statement {}: action must be 'service:Action' or '*'",
                            i + 1
                        ),
                    });
                }
            }
            statements.push(Statement {
                effect,
                actions: st.action,
            });
        }
        Ok(Policy { statements })
    }

    /// Decide one request action. Explicit Deny wins; else Allow if matched;
    /// else default Deny. Service segment compared case-insensitively, the
    /// operation case-sensitively (write service prefixes lowercase, as in IAM).
    pub fn decision(&self, service: &str, operation: &str) -> Effect {
        let request = format!("{}:{}", service.to_ascii_lowercase(), operation);
        let mut allowed = false;
        for st in &self.statements {
            for pat in &st.actions {
                let pat = lower_service_segment(pat);
                if glob_match(&pat, &request) {
                    match st.effect {
                        Effect::Deny => return Effect::Deny,
                        Effect::Allow => allowed = true,
                    }
                }
            }
        }
        if allowed {
            Effect::Allow
        } else {
            Effect::Deny
        }
    }

    /// Flat list of actions across Allow statements (for the Settings summary).
    pub fn allow_summary(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .statements
            .iter()
            .filter(|s| s.effect == Effect::Allow)
            .flat_map(|s| s.actions.iter().cloned())
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Existing credential-provider preflight proxy. This does not enumerate or
/// intercept all SDK credential-provider activity.
pub const CREDENTIAL_OP: (&str, &str) = ("sso", "GetRoleCredentials");

/// Previous generated default policy. This is migration-only: if a user still
/// has this exact unedited file, replace it with `default_yaml()`. These are
/// not active callable operations.
const LEGACY_DEFAULT_OPS: &[(&str, &str)] = &[
    ("cloudformation", "ListStacks"),
    ("cloudformation", "DescribeStackResources"),
    ("cloudformation", "DescribeStackEvents"),
    ("logs", "DescribeLogGroups"),
    ("logs", "StartQuery"),
    ("logs", "GetQueryResults"),
    ("logs", "FilterLogEvents"),
    ("codepipeline", "ListPipelineExecutions"),
    ("codepipeline", "GetPipelineExecution"),
    ("codepipeline", "ListActionExecutions"),
    ("codebuild", "BatchGetBuilds"),
    ("logs", "GetLogEvents"),
    ("resourcegroupstaggingapi", "GetResources"),
];

/// Generated default policy immediately before the CodeArtifact widget existed.
/// If this exact unedited file is present, upgrade it to include the new
/// read-only CodeArtifact operations.
const PRE_CODEARTIFACT_DEFAULT_OPS: &[(&str, &str)] = &[
    CREDENTIAL_OP,
    ("sts", "GetCallerIdentity"),
    ("cloudformation", "ListStacks"),
    ("cloudformation", "DescribeStackResources"),
    ("cloudformation", "DescribeStackEvents"),
    ("logs", "DescribeLogGroups"),
    ("logs", "StartQuery"),
    ("logs", "GetQueryResults"),
    ("logs", "FilterLogEvents"),
    ("codepipeline", "ListPipelines"),
    ("codepipeline", "ListPipelineExecutions"),
    ("codepipeline", "ListActionExecutions"),
    ("codebuild", "BatchGetBuilds"),
    ("logs", "GetLogEvents"),
    ("resourcegroupstaggingapi", "GetResources"),
];

/// Generated default policy immediately before the Lambda Logs browser added
/// Lambda function discovery and explicit log-stream selection.
const PRE_LAMBDA_LOG_BROWSER_DEFAULT_OPS: &[(&str, &str)] = &[
    CREDENTIAL_OP,
    ("sts", "GetCallerIdentity"),
    ("cloudformation", "ListStacks"),
    ("cloudformation", "DescribeStackResources"),
    ("cloudformation", "DescribeStackEvents"),
    ("logs", "DescribeLogGroups"),
    ("logs", "StartQuery"),
    ("logs", "GetQueryResults"),
    ("logs", "FilterLogEvents"),
    ("codepipeline", "ListPipelines"),
    ("codepipeline", "ListPipelineExecutions"),
    ("codepipeline", "ListActionExecutions"),
    ("codebuild", "BatchGetBuilds"),
    ("codeartifact", "ListPackages"),
    ("codeartifact", "ListPackageVersions"),
    ("codeartifact", "DescribePackageVersion"),
    ("logs", "GetLogEvents"),
    ("resourcegroupstaggingapi", "GetResources"),
];

/// Generated default policy immediately before the "bring your own service"
/// widgets (Logs Insights query + AWS CLI table) added logs:StopQuery.
const PRE_BYO_WIDGETS_DEFAULT_OPS: &[(&str, &str)] = &[
    CREDENTIAL_OP,
    ("sts", "GetCallerIdentity"),
    ("cloudformation", "ListStacks"),
    ("cloudformation", "DescribeStackResources"),
    ("cloudformation", "DescribeStackEvents"),
    ("logs", "DescribeLogGroups"),
    ("logs", "StartQuery"),
    ("logs", "GetQueryResults"),
    ("logs", "FilterLogEvents"),
    ("codepipeline", "ListPipelines"),
    ("codepipeline", "ListPipelineExecutions"),
    ("codepipeline", "ListActionExecutions"),
    ("codebuild", "BatchGetBuilds"),
    ("codeartifact", "ListPackages"),
    ("codeartifact", "ListPackageVersions"),
    ("codeartifact", "DescribePackageVersion"),
    ("lambda", "ListFunctions"),
    ("logs", "GetLogEvents"),
    ("logs", "DescribeLogStreams"),
    ("resourcegroupstaggingapi", "GetResources"),
];

pub fn is_credential_op(service: &str, operation: &str) -> bool {
    service.eq_ignore_ascii_case(CREDENTIAL_OP.0) && operation == CREDENTIAL_OP.1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialPreflight {
    NotRequired,
    Allowed {
        service: &'static str,
        operation: &'static str,
        reason: String,
    },
    Denied {
        service: &'static str,
        operation: &'static str,
        reason: String,
    },
}

pub fn credential_preflight(
    policy: &Result<Policy, String>,
    service: &str,
    operation: &str,
) -> CredentialPreflight {
    if is_credential_op(service, operation) {
        return CredentialPreflight::NotRequired;
    }

    let (credential_service, credential_operation) = CREDENTIAL_OP;
    match gate(policy, credential_service, credential_operation) {
        Ok(()) => CredentialPreflight::Allowed {
            service: credential_service,
            operation: credential_operation,
            reason: format!("credential preflight for {service}:{operation}"),
        },
        Err(reason) => CredentialPreflight::Denied {
            service: credential_service,
            operation: credential_operation,
            reason: format!("required before {service}:{operation}: {reason}"),
        },
    }
}

/// The tightest policy that still yields a working dashboard: an Allow for every
/// op the app can issue. Written on first run; the user deletes lines to scope down.
pub fn default_yaml() -> String {
    let mut s = String::from(
        "# Cloud Burrito read-only policy.\n\
         # IAM-style: explicit Deny > Allow > default-deny. Globs * and ? work.\n\
         # This file can only NARROW what the app calls; it can never enable a write.\n\
         # Delete lines to scope down; everything not allowed shows as a locked tile.\n\
         statements:\n  - effect: Allow\n    action:\n",
    );
    for spec in guard::APP_OPS {
        s.push_str(&format!("      - {}:{}\n", spec.service, spec.operation));
    }
    s
}

fn sorted_actions(ops: &[(&str, &str)]) -> Vec<String> {
    let mut actions: Vec<String> = ops.iter().map(|(svc, op)| format!("{svc}:{op}")).collect();
    actions.sort();
    actions
}

fn upgrade_legacy_default_text(text: &str) -> Option<String> {
    let policy = Policy::parse(text).ok()?;
    if policy.statements.len() != 1 || policy.statements[0].effect != Effect::Allow {
        return None;
    }
    let mut actions = policy.statements[0].actions.clone();
    actions.sort();
    actions.dedup();
    if actions == sorted_actions(LEGACY_DEFAULT_OPS)
        || actions == sorted_actions(PRE_CODEARTIFACT_DEFAULT_OPS)
        || actions == sorted_actions(PRE_LAMBDA_LOG_BROWSER_DEFAULT_OPS)
        || actions == sorted_actions(PRE_BYO_WIDGETS_DEFAULT_OPS)
    {
        Some(default_yaml())
    } else {
        None
    }
}

pub fn policy_path(paths: &AppPaths) -> PathBuf {
    paths.data_file("policy.yaml")
}

fn ensure_parent(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Read the raw policy text, creating the default file only when it is missing.
pub fn raw_text(paths: &AppPaths) -> Result<String, PolicyError> {
    let path = policy_path(paths);
    match fs::read_to_string(&path) {
        Ok(text) => {
            if let Some(upgraded) = upgrade_legacy_default_text(&text) {
                fs::write(&path, &upgraded).map_err(|_| PolicyError {
                    message: "could not upgrade default policy".into(),
                })?;
                Ok(upgraded)
            } else {
                Ok(text)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let default = default_yaml();
            ensure_parent(&path).map_err(|_| PolicyError {
                message: "could not create policy directory".into(),
            })?;
            fs::write(&path, &default).map_err(|_| PolicyError {
                message: "could not write default policy".into(),
            })?;
            Ok(default)
        }
        Err(_) => Err(PolicyError {
            message: "could not read policy file".into(),
        }),
    }
}

/// Read + parse the active policy (creating the default on first run).
pub fn load(paths: &AppPaths) -> Result<Policy, PolicyError> {
    Policy::parse(&raw_text(paths)?)
}

/// Validate + write candidate text. Does not write when invalid.
pub fn write_text(paths: &AppPaths, text: &str) -> Result<Policy, PolicyError> {
    let policy = Policy::parse(text)?;
    let path = policy_path(paths);
    ensure_parent(&path).map_err(|_| PolicyError {
        message: "Policy text or storage could not be processed".into(),
    })?;
    fs::write(&path, text).map_err(|_| PolicyError {
        message: "Policy text or storage could not be processed".into(),
    })?;
    Ok(policy)
}

/// Intersect the exact registry with user policy, including query cleanup.
/// `Ok(())` = allowed; `Err(reason)` = denied (reason is user-facing).
pub fn gate(policy: &Result<Policy, String>, service: &str, operation: &str) -> Result<(), String> {
    let spec = guard::operation(service, operation)
        .ok_or_else(|| "not in the compiled AWS call registry".to_string())?;
    let policy = policy
        .as_ref()
        .map_err(|_| "policy file invalid".to_string())?;
    if policy.decision(spec.service, spec.operation) != Effect::Allow {
        return Err("not allowed by your read-only policy".to_string());
    }
    if spec.effect == OperationEffect::QueryStart
        && policy.decision(spec.service, "StopQuery") != Effect::Allow
    {
        return Err(format!(
            "query cleanup capability missing: {}:StopQuery must be allowed before {}:{}",
            spec.service, spec.service, spec.operation
        ));
    }
    Ok(())
}

/// CLI table calls require an explicitly mapped resource read and a narrowing
/// user policy. Query control and credential acquisition are app-owned paths.
pub fn gate_cli(
    policy: &Result<Policy, String>,
    service: &str,
    operation: &str,
) -> Result<(), String> {
    let spec = guard::operation(service, operation)
        .filter(|spec| spec.service == service)
        .ok_or_else(|| "not in the approved CLI operation registry".to_string())?;
    if spec.effect != OperationEffect::ResourceRead || spec.cli_command.is_none() {
        return Err("operation is not approved for the CLI table".to_string());
    }
    gate(policy, spec.service, spec.operation)
}

/// Lowercase only the part before the first ':' so `CloudFormation:List*`
/// matches a lowercased request action.
fn lower_service_segment(pattern: &str) -> String {
    match pattern.split_once(':') {
        Some((svc, rest)) => format!("{}:{}", svc.to_ascii_lowercase(), rest),
        None => pattern.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn glob_matches() {
        assert!(glob_match(
            "cloudformation:List*",
            "cloudformation:ListStacks"
        ));
        assert!(glob_match("logs:*", "logs:StartQuery"));
        assert!(glob_match("*", "anything:AtAll"));
        assert!(glob_match("logs:Get?ueryResults", "logs:GetQueryResults"));
        assert!(!glob_match("cloudformation:List*", "logs:ListStacks"));
        assert!(!glob_match("logs:Start", "logs:StartQuery"));
        assert!(glob_match("logs:StartQuery*", "logs:StartQuery"));
        assert!(!glob_match("logs:StartQuery?", "logs:StartQuery"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn decision_rules() {
        let p = Policy::parse(
            "statements:\n  - effect: Allow\n    action: [cloudformation:Describe*, logs:*]\n  - effect: Deny\n    action: [logs:StartQuery]\n",
        )
        .unwrap();
        assert_eq!(
            p.decision("cloudformation", "DescribeStacks"),
            Effect::Allow
        );
        assert_eq!(p.decision("logs", "FilterLogEvents"), Effect::Allow);
        assert_eq!(p.decision("logs", "StartQuery"), Effect::Deny); // explicit deny wins
        assert_eq!(
            p.decision("codepipeline", "ListPipelineExecutions"),
            Effect::Deny
        ); // default deny
        assert_eq!(
            p.decision("CloudFormation", "DescribeStacks"),
            Effect::Allow
        ); // service case-insensitive
    }

    #[test]
    fn empty_policy_denies_all() {
        let p = Policy::parse("").unwrap();
        assert_eq!(p.decision("logs", "FilterLogEvents"), Effect::Deny);
    }

    #[test]
    fn invalid_yaml_and_bad_action_error() {
        assert!(Policy::parse("statements: [ : : :").is_err());
        assert!(Policy::parse("statements:\n  - effect: Allow\n    action: [nocolon]\n").is_err());
        assert!(Policy::parse("statements:\n  - effect: Maybe\n    action: []\n").is_err());
    }

    #[test]
    fn allow_summary_deduplicates_and_sorts() {
        let p = Policy::parse(
            "statements:\n  - effect: Allow\n    action: [logs:*, cloudformation:Describe*]\n  - effect: Allow\n    action: [logs:*]\n  - effect: Deny\n    action: [s3:*]\n",
        )
        .unwrap();
        assert_eq!(
            p.allow_summary(),
            vec!["cloudformation:Describe*", "logs:*"]
        );
    }

    #[test]
    fn deny_before_allow_still_denies() {
        let p = Policy::parse(
            "statements:\n  - effect: Deny\n    action: [logs:StartQuery]\n  - effect: Allow\n    action: [logs:*]\n",
        )
        .unwrap();
        assert_eq!(p.decision("logs", "StartQuery"), Effect::Deny);
        assert_eq!(p.decision("logs", "FilterLogEvents"), Effect::Allow);
    }

    #[test]
    fn wildcard_allow_matches_everything() {
        let p = Policy::parse("statements:\n  - effect: Allow\n    action: [\"*\"]\n").unwrap();
        assert_eq!(p.decision("cloudformation", "ListStacks"), Effect::Allow);
        assert_eq!(p.decision("anyservice", "AnyOp"), Effect::Allow);
    }

    #[test]
    fn default_allows_every_app_op() {
        let p = Ok(Policy::parse(&default_yaml()).unwrap());
        for spec in guard::APP_OPS {
            assert!(
                gate(&p, spec.service, spec.operation).is_ok(),
                "default must allow {}:{}",
                spec.service,
                spec.operation
            );
        }
    }

    #[test]
    fn generated_default_preserves_existing_text_and_action_order() {
        let expected = concat!(
            "# Cloud Burrito read-only policy.\n",
            "# IAM-style: explicit Deny > Allow > default-deny. Globs * and ? work.\n",
            "# This file can only NARROW what the app calls; it can never enable a write.\n",
            "# Delete lines to scope down; everything not allowed shows as a locked tile.\n",
            "statements:\n  - effect: Allow\n    action:\n",
            "      - sso:GetRoleCredentials\n",
            "      - sts:GetCallerIdentity\n",
            "      - cloudformation:ListStacks\n",
            "      - cloudformation:DescribeStackResources\n",
            "      - cloudformation:DescribeStackEvents\n",
            "      - logs:DescribeLogGroups\n",
            "      - logs:StartQuery\n",
            "      - logs:GetQueryResults\n",
            "      - logs:StopQuery\n",
            "      - logs:FilterLogEvents\n",
            "      - codepipeline:ListPipelines\n",
            "      - codepipeline:ListPipelineExecutions\n",
            "      - codepipeline:ListActionExecutions\n",
            "      - codebuild:BatchGetBuilds\n",
            "      - codeartifact:ListPackages\n",
            "      - codeartifact:ListPackageVersions\n",
            "      - codeartifact:DescribePackageVersion\n",
            "      - lambda:ListFunctions\n",
            "      - logs:GetLogEvents\n",
            "      - logs:DescribeLogStreams\n",
            "      - resourcegroupstaggingapi:GetResources\n",
        );
        assert_eq!(default_yaml(), expected);
    }

    #[test]
    fn upgrades_only_unchanged_legacy_default_policy() {
        let mut legacy = "statements:\n  - effect: Allow\n    action:\n".to_string();
        for (svc, op) in LEGACY_DEFAULT_OPS {
            legacy.push_str(&format!("      - {svc}:{op}\n"));
        }

        let upgraded = upgrade_legacy_default_text(&legacy).unwrap();
        assert!(upgraded.contains("sso:GetRoleCredentials"));
        assert!(upgraded.contains("sts:GetCallerIdentity"));
        assert!(upgraded.contains("codepipeline:ListPipelines"));
        assert!(upgraded.contains("codeartifact:ListPackages"));
        assert!(upgraded.contains("codeartifact:ListPackageVersions"));
        assert!(upgraded.contains("codeartifact:DescribePackageVersion"));
        assert!(upgraded.contains("lambda:ListFunctions"));
        assert!(upgraded.contains("logs:DescribeLogStreams"));
        assert!(!upgraded.contains("codepipeline:GetPipelineExecution"));

        let narrowed = legacy.replace("      - logs:FilterLogEvents\n", "");
        assert!(upgrade_legacy_default_text(&narrowed).is_none());
    }

    #[test]
    fn upgrades_pre_codeartifact_default_policy() {
        let mut previous = "statements:\n  - effect: Allow\n    action:\n".to_string();
        for (svc, op) in PRE_CODEARTIFACT_DEFAULT_OPS {
            previous.push_str(&format!("      - {svc}:{op}\n"));
        }

        let upgraded = upgrade_legacy_default_text(&previous).unwrap();
        assert!(upgraded.contains("codeartifact:ListPackages"));
        assert!(upgraded.contains("codeartifact:ListPackageVersions"));
        assert!(upgraded.contains("codeartifact:DescribePackageVersion"));
        assert!(upgraded.contains("lambda:ListFunctions"));
        assert!(upgraded.contains("logs:DescribeLogStreams"));
    }

    #[test]
    fn upgrades_pre_lambda_log_browser_default_policy() {
        let mut previous = "statements:\n  - effect: Allow\n    action:\n".to_string();
        for (svc, op) in PRE_LAMBDA_LOG_BROWSER_DEFAULT_OPS {
            previous.push_str(&format!("      - {svc}:{op}\n"));
        }

        let upgraded = upgrade_legacy_default_text(&previous).unwrap();
        assert!(upgraded.contains("lambda:ListFunctions"));
        assert!(upgraded.contains("logs:DescribeLogStreams"));
    }

    #[test]
    fn wildcard_policy_cannot_add_unreviewed_operations() {
        let permissive =
            Ok(Policy::parse("statements:\n  - effect: Allow\n    action: [\"*\"]\n").unwrap());
        for (service, operation) in [
            ("ecr", "BatchDeleteImage"),
            ("dynamodb", "BatchWriteItem"),
            ("rds-data", "ExecuteStatement"),
            ("ec2", "TerminateInstances"),
            ("s3", "PutObject"),
            ("ec2", "DescribeInstances"),
            ("logs", "GetUnknownResource"),
            ("logs", "StartQueryExtra"),
            ("logs", "ListStacks"),
            ("sts", "AssumeRole"),
            ("sts", "GetSessionToken"),
            ("sts", "GetFederationToken"),
        ] {
            assert!(
                gate(&permissive, service, operation).is_err(),
                "SDK must deny unregistered {service}:{operation}"
            );
            assert!(
                gate_cli(&permissive, service, operation).is_err(),
                "CLI must deny unregistered {service}:{operation}"
            );
        }
    }

    #[test]
    fn cli_accepts_registered_reads_but_not_query_control_or_credentials() {
        let default = Ok(Policy::parse(&default_yaml()).unwrap());
        let permissive =
            Ok(Policy::parse("statements:\n  - effect: Allow\n    action: [\"*\"]\n").unwrap());
        for spec in guard::APP_OPS {
            for policy in [&default, &permissive] {
                assert_eq!(
                    gate_cli(policy, spec.service, spec.operation).is_ok(),
                    spec.effect == OperationEffect::ResourceRead,
                    "CLI approval for {}:{} must follow its reviewed effect",
                    spec.service,
                    spec.operation
                );
            }
        }
    }

    #[test]
    fn sdk_service_aliases_remain_case_insensitive_but_cli_spelling_is_exact() {
        let allow = Ok(Policy::parse(&default_yaml()).unwrap());
        assert!(gate(&allow, "CloudFormation", "ListStacks").is_ok());
        assert!(gate(&allow, "LOGS", "StartQuery").is_ok());
        assert!(gate(&allow, "cloudformation", "liststacks").is_err());
        assert!(gate_cli(&allow, "CloudFormation", "ListStacks").is_err());
        assert!(gate_cli(&allow, "cloudformation", "liststacks").is_err());
        assert!(gate_cli(&allow, "cloudformation", "list-stacks").is_err());
        assert!(gate_cli(&allow, "cloudformation", "ListStacks").is_ok());
    }

    #[test]
    fn upgrades_pre_byo_widgets_default_policy() {
        let mut previous = "statements:\n  - effect: Allow\n    action:\n".to_string();
        for (svc, op) in PRE_BYO_WIDGETS_DEFAULT_OPS {
            previous.push_str(&format!("      - {svc}:{op}\n"));
        }

        let upgraded = upgrade_legacy_default_text(&previous).unwrap();
        assert!(upgraded.contains("logs:StopQuery"));
    }

    #[test]
    fn registered_operations_still_require_valid_policy_and_explicit_allow() {
        for yaml in [
            "statements:\n  - effect: Allow\n    action: [\"*\"]\n  - effect: Deny\n    action: [logs:FilterLogEvents]\n",
            "statements:\n  - effect: Deny\n    action: [logs:FilterLogEvents]\n  - effect: Allow\n    action: [\"*\"]\n",
            "statements: []\n",
        ] {
            let policy = Ok(Policy::parse(yaml).unwrap());
            assert!(gate(&policy, "logs", "FilterLogEvents").is_err());
            assert!(gate_cli(&policy, "logs", "FilterLogEvents").is_err());
        }
        for yaml in [
            "statements: [ : : :",
            "statements:\n  - effect: Maybe\n    action: [\"*\"]\n",
            "statements:\n  - effect: Allow\n    action: [nocolon]\n",
        ] {
            let policy = Policy::parse(yaml).map_err(|error| error.message);
            assert!(policy.is_err());
            for operation in ["FilterLogEvents", "StartQuery"] {
                assert!(gate(&policy, "logs", operation)
                    .unwrap_err()
                    .starts_with("policy file invalid"));
            }
            assert!(gate_cli(&policy, "logs", "FilterLogEvents")
                .unwrap_err()
                .starts_with("policy file invalid"));
        }
    }

    #[test]
    fn start_query_requires_allowed_cleanup_without_widening_policy() {
        for yaml in [
            "statements:\n  - effect: Allow\n    action: [logs:StartQuery]\n",
            "statements:\n  - effect: Allow\n    action: [\"*\"]\n  - effect: Deny\n    action: [logs:StopQuery]\n",
            "statements:\n  - effect: Deny\n    action: [logs:StopQuery]\n  - effect: Allow\n    action: [logs:*]\n",
        ] {
            let policy = Ok(Policy::parse(yaml).unwrap());
            assert_eq!(
                gate(&policy, "logs", "StartQuery").unwrap_err(),
                "query cleanup capability missing: logs:StopQuery must be allowed before logs:StartQuery"
            );
            assert_eq!(
                policy.as_ref().unwrap().decision("logs", "StopQuery"),
                Effect::Deny
            );
            assert!(gate(&policy, "logs", "StopQuery").is_err());
        }

        let both = Ok(Policy::parse(
            "statements:\n  - effect: Allow\n    action: [logs:StartQuery, logs:StopQuery]\n",
        )
        .unwrap());
        assert!(gate(&both, "logs", "StartQuery").is_ok());
        assert!(gate(&both, "logs", "StopQuery").is_ok());

        let stop_only = Ok(Policy::parse(
            "statements:\n  - effect: Allow\n    action: [logs:StopQuery]\n",
        )
        .unwrap());
        assert!(gate(&stop_only, "logs", "StopQuery").is_ok());
        assert_eq!(
            gate(&stop_only, "logs", "StartQuery").unwrap_err(),
            "not allowed by your read-only policy"
        );

        let denied_start = Ok(Policy::parse(
            "statements:\n  - effect: Allow\n    action: [logs:*]\n  - effect: Deny\n    action: [logs:StartQuery]\n",
        )
        .unwrap());
        assert_eq!(
            gate(&denied_start, "logs", "StartQuery").unwrap_err(),
            "not allowed by your read-only policy"
        );
        assert!(gate(&denied_start, "logs", "StopQuery").is_ok());
    }

    #[test]
    fn credential_preflight_requires_sso_before_service_calls() {
        let allow = Ok(Policy::parse(&default_yaml()).unwrap());
        assert_eq!(
            credential_preflight(&allow, "sso", "GetRoleCredentials"),
            CredentialPreflight::NotRequired
        );

        match credential_preflight(&allow, "logs", "FilterLogEvents") {
            CredentialPreflight::Allowed {
                service,
                operation,
                reason,
            } => {
                assert_eq!(service, "sso");
                assert_eq!(operation, "GetRoleCredentials");
                assert_eq!(reason, "credential preflight for logs:FilterLogEvents");
            }
            other => panic!("expected allowed credential preflight, got {other:?}"),
        }

        let no_sso = Ok(Policy::parse(
            "statements:\n  - effect: Allow\n    action: [logs:FilterLogEvents]\n",
        )
        .unwrap());
        match credential_preflight(&no_sso, "logs", "FilterLogEvents") {
            CredentialPreflight::Denied {
                service,
                operation,
                reason,
            } => {
                assert_eq!(service, "sso");
                assert_eq!(operation, "GetRoleCredentials");
                assert!(reason.contains("required before logs:FilterLogEvents"));
            }
            other => panic!("expected denied credential preflight, got {other:?}"),
        }
    }

    #[test]
    fn raw_text_writes_default_when_missing() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let text = raw_text(&paths).unwrap();
        assert!(text.contains("cloudformation:ListStacks"));
        assert!(text.contains("sso:GetRoleCredentials"));
        assert!(text.contains("sts:GetCallerIdentity"));
        assert!(text.contains("codepipeline:ListPipelines"));
        assert!(text.contains("lambda:ListFunctions"));
        assert!(text.contains("logs:DescribeLogStreams"));
        assert!(policy_path(&paths).exists());
        // round-trip a narrowed policy
        write_text(
            &paths,
            "statements:\n  - effect: Allow\n    action: [logs:*]\n",
        )
        .unwrap();
        assert_eq!(
            load(&paths)
                .unwrap()
                .decision("cloudformation", "ListStacks"),
            Effect::Deny
        );
        assert_eq!(
            load(&paths).unwrap().decision("logs", "StartQuery"),
            Effect::Allow
        );
    }
}
