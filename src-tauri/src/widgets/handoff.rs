//! Closed, evidence-labelled navigation targets. These are hints for an explicit
//! user action, never authority to change the verified account or region.

use serde::Serialize;
use serde_json::{json, Value};

use super::WidgetCtx;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Source {
    PipelineBuildExecution,
    PipelineStackConfiguration,
    CodebuildLogs,
    CfnLogGroup,
    CfnOwnership,
    LambdaLoggingConfig,
    LambdaDefaultConvention,
    RelationshipUnknown,
    ContextMismatch,
    UnsupportedAction,
    IdentifierUnavailable,
    OwnershipAmbiguous,
    OwnershipFailed,
    OwnershipNotFound,
    OwnershipUnsupported,
    OwnershipNotAttempted,
    OwnershipDenied,
}

impl Source {
    fn reason(self) -> &'static str {
        match self {
            Self::PipelineBuildExecution => "AWS returned this build execution identifier for this pipeline action.",
            Self::PipelineStackConfiguration => "Configured stack target for this pipeline action; ownership and existence are not established.",
            Self::CodebuildLogs => "AWS returned this log group and stream for the requested build.",
            Self::CfnLogGroup => "CloudFormation returned this physical identifier for a log group resource.",
            Self::CfnOwnership => "CloudFormation matched the exact resource identifier to this stack in the current context.",
            Self::LambdaLoggingConfig => "Lambda configuration names this log group; its existence is not yet verified.",
            Self::LambdaDefaultConvention => "Default naming convention only; this log group is not verified. Check it explicitly.",
            Self::RelationshipUnknown => "No stack relationship was returned. Search in this context and choose a candidate explicitly.",
            Self::ContextMismatch => "The returned identifier does not establish a target in this request's verified context.",
            Self::UnsupportedAction => "This action does not provide a reviewed resource handoff.",
            Self::IdentifierUnavailable => "No usable resource identifier was returned.",
            Self::OwnershipAmbiguous => "The returned resource evidence does not identify one consistent owning stack.",
            Self::OwnershipFailed => "Stack ownership could not be established from the returned resource evidence.",
            Self::OwnershipNotFound => "CloudFormation returned no exact owning-stack match for this resource.",
            Self::OwnershipUnsupported => "This resource type does not provide a reviewed ownership lookup in this context.",
            Self::OwnershipNotAttempted => "Stack ownership was not requested for this resource.",
            Self::OwnershipDenied => "Stack ownership lookup was not permitted; the resource is retained without an ownership claim.",
        }
    }
}

pub(super) fn unavailable(source: Source) -> Value {
    json!({"status":"unavailable", "source":source, "widget":null, "inputs":{}, "reason":source.reason()})
}

pub(super) fn manual_stack() -> Value {
    let source = Source::RelationshipUnknown;
    json!({"status":"manual", "source":source, "widget":"resource-lookup", "inputs":{}, "reason":source.reason()})
}

pub(super) fn build(id: &str) -> Value {
    let source = Source::PipelineBuildExecution;
    json!({"status":"available", "source":source, "widget":"codebuild-log", "inputs":{"build_id":id}, "reason":source.reason()})
}

pub(super) fn stack(stack: &str, source: Source) -> Value {
    json!({"status":"available", "source":source, "widget":"cfn-stack-detail", "inputs":{"stack_name":stack}, "reason":source.reason()})
}

pub(super) fn logs(group: &str, stream: Option<&str>, source: Source) -> Value {
    if !valid_log_group(group) || stream.is_some_and(|stream| !valid_log_stream(stream)) {
        return unavailable(Source::IdentifierUnavailable);
    }
    let mut inputs =
        json!({"mode":if stream.is_some() { "events" } else { "streams" },"log_group":group});
    if let Some(stream) = stream {
        inputs["log_stream"] = json!(stream);
    }
    json!({"status":if matches!(source,Source::LambdaDefaultConvention) { "manual" } else { "available" },"source":source,"widget":"log-tail","inputs":inputs,"reason":source.reason()})
}

pub(super) fn same_context_arn(
    value: &str,
    service: &str,
    resource_prefix: &str,
    ctx: &WidgetCtx,
) -> bool {
    let parts: Vec<_> = value.splitn(6, ':').collect();
    value.len() <= 1024
        && !value.chars().any(char::is_control)
        && parts.len() == 6
        && parts[0] == "arn"
        && parts[1] == "aws"
        && parts[2] == service
        && (parts[3] == ctx.region || (service == "iam" && parts[3].is_empty()))
        && parts[4] == ctx.account_id
        && parts[5].starts_with(resource_prefix)
        && parts[5].len() > resource_prefix.len()
}

pub(super) fn valid_stack(value: &str, ctx: &WidgetCtx) -> bool {
    if value.starts_with("arn:") {
        return same_context_arn(value, "cloudformation", "stack/", ctx)
            && value.splitn(6, ':').nth(5).is_some_and(|resource| {
                let parts: Vec<_> = resource.split('/').collect();
                parts.len() == 3
                    && valid_stack_name(parts[1])
                    && !parts[2].is_empty()
                    && parts[2]
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
    }
    valid_stack_name(value)
}

fn valid_stack_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub(super) fn valid_build(value: &str, ctx: &WidgetCtx) -> bool {
    let id = if value.starts_with("arn:") {
        if !same_context_arn(value, "codebuild", "build/", ctx) {
            return false;
        }
        value
            .splitn(6, ':')
            .nth(5)
            .and_then(|resource| resource.strip_prefix("build/"))
            .unwrap_or("")
    } else {
        value
    };
    let Some((project, execution)) = id.split_once(':') else {
        return false;
    };
    !project.is_empty()
        && project.len() <= 255
        && !execution.is_empty()
        && execution.len() <= 128
        && project
            .bytes()
            .chain(execution.bytes())
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_log_group(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b'#')
        })
}

fn valid_log_stream(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, ':' | '*'))
}
