//! The local Tauri command surface with additive request/context metadata.

use serde_json::{json, Value};
use tauri::State;

use crate::aws::{self, AwsContext};
use crate::request::RequestEnvelope;
use crate::state::AppState;
use crate::{dashboard, settings, widgets};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn config_path_from_settings(s: &Value) -> String {
    settings::get_str(s, "aws_config_path")
}

fn observe_settings(state: &AppState, settings: &Value) -> u64 {
    state.observe_credential_settings(
        &config_path_from_settings(settings),
        &settings::get_str(settings, "sso_session_name"),
    )
}

const UNSUPPORTED_REGION: &str =
    "Choose a supported AWS region before connecting or running this request";
const SESSION_CONFLICT: &str =
    "The requested SSO session conflicts with the saved SSO session constraint";

/// A saved session is an additional constraint. It never supplies credentials
/// or changes the session selected by the profile's validated configuration.
fn effective_session_constraint(
    settings: &Value,
    hint: Option<&str>,
) -> Result<Option<String>, &'static str> {
    let configured = settings::get_str(settings, "sso_session_name");
    let hint = hint.map(str::trim).filter(|name| !name.is_empty());
    if !configured.is_empty() {
        if hint.is_some_and(|name| name != configured) {
            return Err(SESSION_CONFLICT);
        }
        Ok(Some(configured))
    } else {
        Ok(hint.map(str::to_string))
    }
}

fn audit_aws_call(
    runtime: &crate::runtime::Runtime,
    policy: &Result<aws::policy::Policy, String>,
    service: &str,
    operation: &str,
    account_id: &str,
    region: &str,
) -> Result<(), (String, String)> {
    match aws::policy::credential_preflight(policy, service, operation) {
        aws::policy::CredentialPreflight::NotRequired => {}
        aws::policy::CredentialPreflight::Allowed {
            service,
            operation,
            reason,
        } => {
            runtime.audit(json!({
                "kind": "aws",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
                "reason": reason,
            }));
        }
        aws::policy::CredentialPreflight::Denied {
            service,
            operation,
            reason,
        } => {
            runtime.audit(json!({
                "kind": "aws-blocked",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
                "reason": reason,
            }));
            return Err((format!("{service}:{operation}"), reason));
        }
    }

    match aws::policy::gate(policy, service, operation) {
        Ok(()) => {
            runtime.audit(json!({
                "kind": "aws",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
            }));
            Ok(())
        }
        Err(reason) => {
            runtime.audit(json!({
                "kind": "aws-blocked",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
                "reason": reason,
            }));
            Err((format!("{service}:{operation}"), reason))
        }
    }
}

#[tauri::command]
pub async fn ping() -> Result<Value, String> {
    Ok(json!({"pong": true, "version": VERSION}))
}

#[tauri::command]
pub async fn aws_set_account(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    final_diagnostics(&state, aws_set_account_impl(&state, params).await).await
}

async fn aws_set_account_impl(state: &AppState, params: Value) -> Result<Value, String> {
    let mut request = match request_envelope(state, "aws_set_account", &params) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    let result = aws_set_account_request(state, params, &mut request).await;
    finish_request(request, result)
}

async fn aws_set_account_request(
    state: &AppState,
    params: Value,
    request: &mut RequestEnvelope,
) -> Result<Value, String> {
    let user_settings = match load_settings(state) {
        Ok(settings) => settings,
        Err(error) => return Ok(error),
    };
    let cfg_path = config_path_from_settings(&user_settings);
    let revision = observe_settings(state, &user_settings);
    let profile = params
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let account_id = params
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let region = params
        .get("region")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| settings::get_str(&user_settings, "default_region"));
    let supplied_session = params.get("sso_session_name").and_then(Value::as_str);
    let Some(attempt) = state.begin_attempt(
        json!({
            "profile": profile, "account_id": account_id, "region": region,
            "sso_session": supplied_session, "ts": state.runtime.clock.now_epoch(),
        }),
        revision,
    ) else {
        return Ok(superseded());
    };
    state
        .runtime
        .audit(json!({"kind": "lifecycle", "event": "set_account_attempt", "attempt": attempt}));
    if profile.is_empty() || account_id.is_empty() {
        return Ok(finish_connection_failure(
            state,
            attempt,
            "InvalidContext",
            "Profile and account are required",
            false,
        ));
    }
    if !settings::allowed_region(&region) {
        return Ok(finish_connection_failure(
            state,
            attempt,
            "UnsupportedRegion",
            UNSUPPORTED_REGION,
            false,
        ));
    }
    let session = match effective_session_constraint(&user_settings, supplied_session) {
        Ok(session) => session,
        Err(message) => {
            return Ok(finish_connection_failure(
                state,
                attempt,
                "SsoSessionConflict",
                message,
                false,
            ))
        }
    };
    let ctx = AwsContext::new(
        profile,
        account_id,
        region,
        session,
        cfg_path,
        state.runtime.clone(),
    )
    .with_settings_revision(revision);
    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if let Err((action, reason)) =
        verification_gate(&request.runtime(&state.runtime), &policy, &ctx)
    {
        return Ok(finish_connection_failure(
            state,
            attempt,
            "PolicyDenied",
            &format!("Request blocked: {action}: {reason}"),
            false,
        ));
    }
    request.start();
    let session = match ctx.verified_session().await {
        Ok(session) => session,
        Err(error) => {
            return Ok(finish_connection_failure(
                state,
                attempt,
                error.error_type,
                &error.message,
                needs_login(&error),
            ))
        }
    };
    if let Err(error) = refresh_configuration_revision(state) {
        return Ok(error);
    }
    if let Err(error) = ctx.ensure_current(&session) {
        return Ok(finish_connection_failure(
            state,
            attempt,
            error.error_type,
            &error.message,
            needs_login(&error),
        ));
    }
    let mut connection = state.connection.lock();
    if connection.attempt != attempt || connection.settings_revision != revision {
        return Ok(superseded());
    }
    connection.status = "verified";
    connection.active = Some(ctx.clone());
    request.bind(&ctx, &session);
    connection.set_account_at = Some(state.runtime.clock.now_epoch());
    if let Some(last) = connection.last_attempt.as_mut() {
        last["sso_session"] = json!(session.snapshot.session_name);
        last["caller_arn"] = json!(session.identity.arn);
    }
    state.runtime.audit(
        json!({"kind": "lifecycle", "event": "set_account_ok", "attempt": attempt,
        "account_id": session.identity.account_id, "region": ctx.region}),
    );
    Ok(
        json!({"ok": true, "account_id": session.identity.account_id, "region": ctx.region,
        "caller_arn": session.identity.arn, "attempt_id": attempt, "connection_state": "verified"}),
    )
}

fn needs_login(error: &aws::context::ContextError) -> bool {
    matches!(
        error.error_type,
        "CredentialsError" | "CredentialsExpired" | "IdentityVerificationFailed"
    ) && aws::sso_login_required(&error.message)
}

fn request_error(error_type: &str, message: &str) -> Value {
    json!({"ok": false, "error_type": error_type, "error": message,
        "render": "raw_json", "data": {"error": message}})
}

fn request_envelope(
    state: &AppState,
    command: &'static str,
    params: &Value,
) -> Result<RequestEnvelope, Value> {
    let request = match RequestEnvelope::from_params(params) {
        Ok(request) => request.audited(command, &state.runtime),
        Err(message) => {
            return Err(RequestEnvelope::default()
                .audited(command, &state.runtime)
                .attach(request_error("InvalidRequest", message)))
        }
    };
    if let Err(message) = crate::validation::validate(command, params) {
        return Err(request.attach(request_error("InvalidRequest", message)));
    }
    Ok(request)
}

fn finish_request(
    request: RequestEnvelope,
    result: Result<Value, String>,
) -> Result<Value, String> {
    Ok(request.attach(
        result.unwrap_or_else(|_| request_error("RequestFailed", "The desktop request failed")),
    ))
}

fn superseded() -> Value {
    request_error(
        "Superseded",
        "The account selection or configuration changed; retry in the current context",
    )
}

fn load_settings(state: &AppState) -> Result<Value, Value> {
    match settings::load(&state.runtime.storage) {
        Ok(settings) => {
            observe_settings(state, &settings);
            Ok(settings)
        }
        Err(error) => {
            state.invalidate_settings();
            Err(settings::with_metadata(
                error.response("settings"),
                &json!({}),
            ))
        }
    }
}

fn refresh_configuration_revision(state: &AppState) -> Result<u64, Value> {
    let settings = load_settings(state)?;
    Ok(observe_settings(state, &settings))
}

fn finish_connection_failure(
    state: &AppState,
    attempt: u64,
    error_type: &str,
    message: &str,
    needs_sso_login: bool,
) -> Value {
    if let Err(error) = refresh_configuration_revision(state) {
        return error;
    }
    let mut connection = state.connection.lock();
    if connection.attempt != attempt {
        return superseded();
    }
    connection.status = "failed";
    connection.active = None;
    connection.set_account_at = None;
    if let Some(last) = connection.last_attempt.as_mut() {
        last["error"] = json!(message);
        last["error_type"] = json!(error_type);
        last["needs_sso_login"] = json!(needs_sso_login);
    }
    state.runtime.audit(json!({"kind": "lifecycle", "event": "set_account_failed", "attempt": attempt, "error_type": error_type}));
    json!({"ok": false, "error": message, "error_type": error_type, "needs_sso_login": needs_sso_login,
        "attempt_id": attempt, "connection_state": "failed"})
}

fn verification_gate(
    runtime: &crate::runtime::Runtime,
    policy: &Result<aws::policy::Policy, String>,
    ctx: &AwsContext,
) -> Result<(), (String, String)> {
    audit_aws_call(
        runtime,
        policy,
        "sts",
        "GetCallerIdentity",
        &ctx.account_id,
        &ctx.region,
    )
}

fn pinned_context_fields(scope: &Value) -> Option<(&str, &str, &str)> {
    let field = |key| {
        scope
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    Some((field("profile")?, field("account_id")?, field("region")?))
}

#[derive(Clone)]
struct ResolvedContext {
    context: AwsContext,
    /// Pinned requests are independent of topbar attempts; settings still apply.
    inherited_attempt: Option<u64>,
    _reservation: Option<std::sync::Arc<crate::state::ContextLease>>,
}

fn resolve_widget_ctx(state: &AppState, params: &Value) -> Result<ResolvedContext, Value> {
    let user_settings = load_settings(state)?;
    let path = config_path_from_settings(&user_settings);
    let revision = observe_settings(state, &user_settings);
    let explicit = params
        .get("context")
        .or_else(|| params.get("account_override"));
    if let Some(scope) = explicit {
        let mode = scope.get("mode").and_then(Value::as_str).unwrap_or("");
        if mode == "pinned" || mode.is_empty() {
            let (profile, account, region) = pinned_context_fields(scope).ok_or_else(|| {
                request_error(
                    "InvalidContext",
                    "Pinned context requires its own profile, account and region",
                )
            })?;
            if !settings::allowed_region(region) {
                return Err(request_error("UnsupportedRegion", UNSUPPORTED_REGION));
            }
            let session = effective_session_constraint(&user_settings, None)
                .map_err(|message| request_error("SsoSessionConflict", message))?;
            let mut connection = state.connection.lock();
            let pending_key =
                json!([profile, account, region, path, session, revision]).to_string();
            let cached = connection
                .overrides
                .values_mut()
                .find(|cached| {
                    let ctx = &cached.context;
                    ctx.profile == profile
                        && ctx.account_id == account
                        && ctx.region == region
                        && ctx.settings_revision == revision
                        && ctx.aws_config_path == path
                        && ctx.sso_session_name == session
                })
                .map(|cached| {
                    cached.last_used = std::time::Instant::now();
                    cached.context.clone()
                });
            let (context, reservation) = if let Some(context) = cached {
                (context, None)
            } else if let Some((context, lease)) = connection
                .pending_contexts
                .get(&pending_key)
                .and_then(|pending| {
                    pending
                        .lease
                        .upgrade()
                        .map(|lease| (pending.context.clone(), lease))
                })
            {
                (context, Some(lease))
            } else {
                if connection.pending_contexts.len() >= 128 {
                    return Err(request_error(
                        "QueueFull",
                        "Too many account contexts are waiting for verification",
                    ));
                }
                let context = AwsContext::new(
                    profile.into(),
                    account.into(),
                    region.into(),
                    session,
                    path,
                    state.runtime.clone(),
                )
                .with_settings_revision(revision);
                let lease = std::sync::Arc::new(crate::state::ContextLease {
                    connection: std::sync::Arc::downgrade(&state.connection),
                    key: pending_key.clone(),
                    context_id: context.id(),
                });
                connection.pending_contexts.insert(
                    pending_key,
                    crate::state::PendingContext {
                        context: context.clone(),
                        lease: std::sync::Arc::downgrade(&lease),
                    },
                );
                (context, Some(lease))
            };
            return Ok(ResolvedContext {
                context,
                inherited_attempt: None,
                _reservation: reservation,
            });
        }
        if mode != "inherit" {
            return Err(request_error(
                "InvalidContext",
                "Unknown account context mode",
            ));
        }
    }
    let connection = state.connection.lock();
    let context = connection
        .active
        .clone()
        .ok_or_else(|| request_error("NoVerifiedContext", "No verified AWS account selected"))?;
    if !settings::allowed_region(&context.region) {
        return Err(request_error("UnsupportedRegion", UNSUPPORTED_REGION));
    }
    Ok(ResolvedContext {
        context,
        inherited_attempt: Some(connection.attempt),
        _reservation: None,
    })
}

fn validate_request_context(
    state: &AppState,
    resolved: &ResolvedContext,
    session: &std::sync::Arc<aws::context::VerifiedSession>,
) -> Result<(), Value> {
    refresh_configuration_revision(state)?;
    if let Err(error) = resolved.context.ensure_current(session) {
        // An older result must not tear down a newer verified session on the
        // same context. The refresh that invalidated a context owns its failure.
        if error.error_type == "StaleContext" {
            return Err(superseded());
        }
        state.invalidate_context(resolved.context.id(), error.error_type, &error.message);
        return Err(request_error(error.error_type, &error.message));
    }
    let connection = state.connection.lock();
    if connection.settings_revision != resolved.context.settings_revision {
        return Err(superseded());
    }
    if let Some(attempt) = resolved.inherited_attempt {
        if connection.attempt != attempt
            || connection.active.as_ref().map(AwsContext::id) != Some(resolved.context.id())
        {
            return Err(superseded());
        }
    }
    Ok(())
}

#[cfg(test)]
async fn verify_request_context(
    state: &AppState,
    resolved: &ResolvedContext,
    policy: &Result<aws::policy::Policy, String>,
) -> Result<std::sync::Arc<aws::context::VerifiedSession>, Value> {
    verify_request_context_owned(state, resolved, policy, None).await
}

async fn verify_request_context_owned(
    state: &AppState,
    resolved: &ResolvedContext,
    policy: &Result<aws::policy::Policy, String>,
    mut request: Option<&mut RequestEnvelope>,
) -> Result<std::sync::Arc<aws::context::VerifiedSession>, Value> {
    let runtime = request
        .as_ref()
        .map(|r| r.runtime(&state.runtime))
        .unwrap_or_else(|| state.runtime.clone());
    let ctx = &resolved.context;
    if let Err((action, reason)) = verification_gate(&runtime, policy, ctx) {
        let mut denial = widgets::permission_denied_render("sts", "GetCallerIdentity", &reason);
        denial["action"] = json!(action);
        denial["ok"] = json!(false);
        denial["error"] = json!(reason);
        denial["error_type"] = json!("PolicyDenied");
        return Err(denial);
    }
    if let Some(request) = request.as_mut() {
        request.start();
    }
    let session = match ctx.verified_session().await {
        Ok(session) => session,
        Err(error) => {
            state.invalidate_context(ctx.id(), error.error_type, &error.message);
            return Err(request_error(error.error_type, &error.message));
        }
    };
    validate_request_context(state, resolved, &session)?;
    if resolved.inherited_attempt.is_none() {
        let mut connection = state.connection.lock();
        if connection.settings_revision != ctx.settings_revision {
            return Err(superseded());
        }
        connection.overrides.retain(|_, cached| {
            let old = &cached.context;
            !(old.profile == ctx.profile
                && old.account_id == ctx.account_id
                && old.region == ctx.region)
        });
        connection
            .pending_contexts
            .retain(|_, pending| pending.context.id() != resolved.context.id());
        if connection.overrides.len() >= 64 {
            if let Some(oldest) = connection
                .overrides
                .iter()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(key, _)| key.clone())
            {
                connection.overrides.remove(&oldest);
            }
        }
        connection.overrides.insert(
            crate::state::PinnedKey {
                snapshot: session.snapshot.clone(),
                account_id: session.identity.account_id.clone(),
                arn: session.identity.arn.clone(),
                user_id: session.identity.user_id.clone(),
                provider_revision: session.provider_revision,
            },
            crate::state::CachedContext {
                context: ctx.clone(),
                last_used: std::time::Instant::now(),
            },
        );
    }
    Ok(session)
}

#[tauri::command]
pub async fn widget_fetch(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    final_diagnostics(&state, widget_fetch_impl(&state, params).await).await
}

fn retain_cli_cleanup_failure(ctx: &widgets::WidgetCtx, result: &Value) -> bool {
    if result["error_type"] != "CliCleanupFailed" {
        return false;
    }
    ctx.log(
        "CLI process exit could not be confirmed",
        json!({
            "error_type": "CliCleanupFailed", "account_id": ctx.account_id, "region": ctx.region,
        }),
    );
    true
}

async fn widget_fetch_impl(state: &AppState, params: Value) -> Result<Value, String> {
    let mut request = match request_envelope(state, "widget_fetch", &params) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    let registration = match state.work.register(request.work_id()) {
        Ok(registration) => registration,
        Err(error) => return finish_request(request, Ok(error)),
    };
    let started = tokio::time::Instant::now();
    let result = widget_fetch_request(
        state,
        params,
        &mut request,
        registration.cancellation.clone(),
        started,
    )
    .await;
    finish_request(request, result)
}

async fn widget_fetch_request(
    state: &AppState,
    params: Value,
    request: &mut RequestEnvelope,
    cancellation: crate::process::ProcessCancellation,
    started: tokio::time::Instant,
) -> Result<Value, String> {
    let name = params
        .get("widget")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return Ok(
            json!({"render": "raw_json", "data": {"error": "missing required field 'widget'"}}),
        );
    }
    if !widgets::is_known(&name) {
        return Ok(json!({"render": "raw_json", "data": {"error": "Unknown widget"}}));
    }

    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    let policy_revision = state.observe_policy(&policy);
    let inputs = params.get("inputs").cloned().unwrap_or_else(|| json!({}));
    if name == "aws-cli" {
        let command = inputs.get("command").and_then(Value::as_str).unwrap_or("");
        let parsed = match widgets::parse_cli_command(command) {
            Ok(parsed) => parsed,
            Err(_) => {
                return Ok(request_error(
                    "UnsupportedCommand",
                    "Only reviewed AWS read commands and arguments are supported",
                ))
            }
        };
        if let Err(reason) = aws::policy::gate_cli(&policy, &parsed.service, &parsed.operation) {
            return Ok(widgets::permission_denied_render(
                &parsed.service,
                &parsed.operation,
                &reason,
            ));
        }
    }
    let resolved = match resolve_widget_ctx(state, &params) {
        Ok(context) => context,
        Err(error) => return Ok(error),
    };
    for &(service, operation) in widgets::entry_operations(&name, &inputs) {
        if let Err((action, reason)) = audit_aws_call(
            &request.runtime(&state.runtime),
            &policy,
            service,
            operation,
            &resolved.context.account_id,
            &resolved.context.region,
        ) {
            let mut denied = widgets::permission_denied_render(service, operation, &reason);
            denied["action"] = json!(action);
            return Ok(denied);
        }
    }
    let verification = verify_request_context_owned(state, &resolved, &policy, Some(request));
    let session = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Ok(crate::work_registry::cancelled_result()),
        _ = tokio::time::sleep_until(started + crate::scheduler::WorkBudget::for_widget(&name).deadline) => return Ok(request_error("WorkDeadline", "The request deadline was reached before verification completed")),
        verified = verification => match verified { Ok(session) => session, Err(error) => return Ok(error) },
    };
    request.bind(&resolved.context, &session);
    let cli = if name == "aws-cli" {
        let credentials = match session.cli_credentials().await {
            Ok(credentials) => credentials,
            Err(error) => return Ok(request_error(error.error_type, &error.message)),
        };
        // The frozen provider is asynchronous. A superseded handoff must not
        // start a child even when it resolved successfully.
        if let Err(error) = validate_request_context(state, &resolved, &session) {
            return Ok(error);
        }
        Some(widgets::CliAccess {
            credentials,
            cancellation: crate::process::ProcessCancellation::new(),
        })
    } else {
        None
    };
    let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if state.observe_policy(&current_policy) != policy_revision {
        return Ok(request_error(
            "PolicyChanged",
            "Policy changed; refresh with the current permissions",
        ));
    }
    let authority_key = json!([
        resolved.context.id(),
        session.provider_revision,
        session.snapshot.settings_revision,
        policy_revision,
        session.identity.account_id,
        session.identity.arn,
        session.identity.user_id,
        session.snapshot.profile,
        session.snapshot.config_path,
        session.snapshot.region
    ])
    .to_string();
    let normalized_inputs = if name == "aws-cli" {
        json!(
            widgets::parse_cli_command(inputs["command"].as_str().unwrap_or_default())
                .map(|command| command.argv)
                .unwrap_or_default()
        )
    } else {
        inputs.clone()
    };
    let key = json!([authority_key, name, normalized_inputs]).to_string();
    let cacheable = crate::result_cache::cacheable(&name);
    if cacheable && params.get("reuse_result").and_then(Value::as_bool) == Some(true) {
        // Identity/configuration and all current operation gates were checked
        // above. A hit does not refresh credentials or extend data age.
        if let Some(result) = state.results.get(&key) {
            if let Err(error) = validate_request_context(state, &resolved, &session) {
                return Ok(error);
            }
            if cancellation.is_cancelled() {
                return Ok(crate::work_registry::cancelled_result());
            }
            request
                .runtime(&state.runtime)
                .audit(json!({"kind":"widget", "widget":name, "event":"cache_hit"}));
            return Ok(result);
        }
    }
    let cache_key = key.clone();
    let cache = state.results.clone();
    let state_owned = state.clone();
    let runtime = request.runtime(&state.runtime);
    let result = state
        .work
        .coalesce(key, cancellation, move |job_cancellation| async move {
            let mut scope = crate::scheduler::WorkScope::new(
                authority_key,
                session.identity.account_id.clone(),
                resolved.context.region.clone(),
                job_cancellation.clone(),
                crate::scheduler::WorkBudget::for_widget(&name),
            )
            .with_recovery_authority_key(
                json!([
                    session.identity.account_id,
                    session.identity.arn,
                    session.identity.user_id,
                    session.snapshot.profile,
                    session.snapshot.config_path,
                    session.snapshot.region
                ])
                .to_string(),
            );
            scope.deadline = started + scope.budget.deadline;
            if inputs["mode"] == "enrich" {
                scope = scope.with_priority(crate::scheduler::Priority::Background);
            }
            let cli = cli.map(|mut cli| {
                cli.cancellation = job_cancellation;
                cli
            });
            let wctx = widgets::WidgetCtx {
                runtime: runtime.with_work(scope),
                sdk: session.sdk.clone(),
                account_id: session.identity.account_id.clone(),
                region: resolved.context.region.clone(),
                widget_name: name,
                inputs,
                policy,
                cli,
            };
            let result = run_widget_job(
                state_owned,
                resolved,
                session,
                wctx.clone(),
                policy_revision,
            )
            .await;
            if cacheable {
                cache.insert(cache_key, &result, wctx.runtime.clock.now_epoch());
            }
            result
        })
        .await;
    Ok(result)
}

async fn run_widget_job(
    state: AppState,
    resolved: ResolvedContext,
    session: std::sync::Arc<aws::context::VerifiedSession>,
    wctx: widgets::WidgetCtx,
    policy_revision: u64,
) -> Value {
    let work = widgets::fetch(&wctx.widget_name, &wctx);
    tokio::pin!(work);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let scope = wctx.runtime.work.as_ref().expect("production widget scope");
    let result = loop {
        tokio::select! {
            biased;
            _ = tick.tick() => {
                let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
                if validate_request_context(&state, &resolved, &session).is_err()
                    || state.observe_policy(&current_policy) != policy_revision
                    || tokio::time::Instant::now() >= scope.deadline {
                    scope.cancellation.cancel();
                }
            }
            result = &mut work => break result,
        }
    };
    if retain_cli_cleanup_failure(&wctx, &result) {
        return result;
    }
    if let Err(mut error) = validate_request_context(&state, &resolved, &session) {
        retain_query_outcome(&mut error, &result);
        return error;
    }
    if scope.cancellation.is_cancelled() && result.get("cleanup").is_none() {
        return crate::work_registry::cancelled_result();
    }
    let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if state.observe_policy(&current_policy) != policy_revision {
        let mut error = request_error(
            "PolicyChanged",
            "Policy changed while the request was running",
        );
        retain_query_outcome(&mut error, &result);
        return error;
    }
    result
}

fn retain_query_outcome(error: &mut Value, result: &Value) {
    for key in [
        "cleanup",
        "query_state",
        "recovery_required",
        "can_acknowledge_unknown",
        "recovery_action",
        "recovery_pending",
    ] {
        if let Some(value) = result.get(key) {
            error[key] = value.clone();
        }
    }
}

#[tauri::command]
pub async fn request_cancel(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    if let Some(error) = local_validation(&state, "request_cancel", &params) {
        return final_diagnostics(&state, Ok(error)).await;
    }
    state
        .work
        .cancel(params["request_id"].as_str().expect("validated request ID"));
    Ok(json!({"ok":true,"cancelled_locally":true,"cleanup_confirmed":false}))
}

#[tauri::command]
pub async fn widget_get_source(params: Value) -> Result<Value, String> {
    if let Err(message) = crate::validation::validate("widget_get_source", &params) {
        return Ok(request_error("InvalidRequest", message));
    }
    let name = params.get("widget").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Ok(json!({"ok": false, "error": "missing required field 'widget'"}));
    }
    Ok(widgets::get_source(name))
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<Value, String> {
    let worker_state = state.inner().clone();
    let result = tokio::task::spawn_blocking(move || settings_get_impl(&worker_state))
        .await
        .unwrap_or_else(|_| request_error("StorageReadFailed", "Saved settings could not be read"));
    final_diagnostics(&state, Ok(result)).await
}

fn settings_get_impl(state: &AppState) -> Value {
    let _guard = state.audit_settings.lock();
    let mut result = match load_settings(state) {
        Ok(value) => value,
        Err(error) => {
            let _ = state
                .runtime
                .set_audit_retention(crate::audit::RetentionMode::Preserve);
            return state.runtime.with_diagnostics(error);
        }
    };
    let configured = retention_mode(&result);
    let preserve_required = state.runtime.set_audit_retention(configured).is_err();
    if preserve_required {
        // Reading saved bounded mode must never activate pruning for oversized
        // legacy history. Loading does no rotation, rename or write itself.
        let _ = state
            .runtime
            .set_audit_retention(crate::audit::RetentionMode::Preserve);
    }
    result["_audit_retention"] = json!({
        "configured_mode": configured.as_str(),
        "effective_mode": state.runtime.audit_retention_mode().as_str(),
        "preserve_required": preserve_required,
    });
    state.runtime.with_diagnostics(result)
}

fn retention_mode(settings: &Value) -> crate::audit::RetentionMode {
    if settings["audit_retention"] == "bounded" {
        crate::audit::RetentionMode::Bounded
    } else {
        crate::audit::RetentionMode::Preserve
    }
}

#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    let worker_state = state.inner().clone();
    let result = tokio::task::spawn_blocking(move || settings_set_impl(&worker_state, params))
        .await
        .unwrap_or_else(|_| request_error("StorageWriteFailed", "Settings could not be saved"));
    final_diagnostics(&state, Ok(result)).await
}

fn local_validation(state: &AppState, command: &'static str, params: &Value) -> Option<Value> {
    crate::validation::validate(command, params)
        .err()
        .map(|message| {
            RequestEnvelope::default()
                .audited(command, &state.runtime)
                .attach(request_error("InvalidRequest", message))
        })
}

fn settings_set_impl(state: &AppState, mut params: Value) -> Value {
    if let Some(error) = local_validation(state, "settings_set", &params) {
        return state
            .runtime
            .with_diagnostics(settings::with_metadata(error, &params));
    }
    let _guard = state.audit_settings.lock();
    // Older five-field callers keep their current retention selection. Only an
    // explicit sixth field can change it; a missing saved value means preserve.
    if params.get("audit_retention").is_none() {
        let previous = match load_settings(state) {
            Ok(previous) => previous,
            Err(error) => return state.runtime.with_diagnostics(error),
        };
        params["audit_retention"] = json!(retention_mode(&previous).as_str());
    }
    let selected = retention_mode(&params);
    let result = match state
        .runtime
        .commit_audit_retention(selected, || settings::save(&state.runtime.storage, &params))
    {
        Ok(result) => match refresh_configuration_revision(state) {
            Ok(_) => result,
            Err(error) => error,
        },
        Err(crate::audit::RetentionError::Save(error)) => error.response("settings"),
        Err(crate::audit::RetentionError::HistoryUnavailable) => request_error(
            "AuditHistoryFailed",
            "Activity history could not be checked; settings were not saved",
        ),
        Err(crate::audit::RetentionError::PreserveRequired) => {
            let mut error = request_error(
                "AuditPreserveRequired",
                "Preserve existing activity history before enabling bounded retention",
            );
            error["preserve_required"] = json!(true);
            error
        }
    };
    let mut result = settings::with_metadata(result, &params);
    result["_audit_retention"] =
        json!({"effective_mode":state.runtime.audit_retention_mode().as_str()});
    state.runtime.with_diagnostics(result)
}

#[tauri::command]
pub async fn dashboard_get(state: State<'_, AppState>) -> Result<Value, String> {
    final_diagnostics(&state, Ok(dashboard_get_impl(&state))).await
}

fn dashboard_get_impl(state: &AppState) -> Value {
    state.runtime.with_diagnostics(
        dashboard::load(&state.runtime.storage).unwrap_or_else(|error| error.response("dashboard")),
    )
}

#[tauri::command]
pub async fn dashboard_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    final_diagnostics(&state, Ok(dashboard_set_impl(&state, params))).await
}

fn dashboard_set_impl(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "dashboard_set", &params) {
        return error;
    }
    state.runtime.with_diagnostics(
        dashboard::save(&state.runtime.storage, &params["tiles"])
            .unwrap_or_else(|error| error.response("dashboard")),
    )
}

/// Flush after the final request event, then refresh the sticky diagnostics.
async fn final_diagnostics(
    state: &AppState,
    result: Result<Value, String>,
) -> Result<Value, String> {
    state.runtime.flush_audit().await;
    result.map(|value| state.runtime.with_diagnostics(value))
}

#[tauri::command]
pub async fn audit_tail(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    Ok(audit_tail_async(&state, params).await)
}

async fn audit_tail_async(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "audit_tail", &params) {
        return final_diagnostics(state, Ok(error)).await.unwrap();
    }
    state.runtime.flush_audit().await;
    let worker_state = state.clone();
    let result = tokio::task::spawn_blocking(move || audit_tail_impl(&worker_state, params))
        .await
        .unwrap_or_else(|_| audit_read_failed());
    state.runtime.with_diagnostics(result)
}

fn audit_read_failed() -> Value {
    request_error(
        "AuditReadFailed",
        "Local activity history could not be read",
    )
}

/// Synchronous seam for disposable fixtures; native IPC dispatches this off
/// async workers, and never holds connection/request locks during disk reads.
fn audit_tail_impl(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "audit_tail", &params) {
        return error;
    }
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(300) as usize;
    let cursor = params.get("cursor").and_then(Value::as_str);
    let result = match crate::audit::read_page(&state.runtime.paths, cursor, limit) {
        Ok(page) => {
            let mut value = serde_json::to_value(page).expect("activity page is serializable");
            value["ok"] = json!(true);
            value
        }
        Err(()) => audit_read_failed(),
    };
    state.runtime.with_diagnostics(result)
}

#[tauri::command]
pub async fn audit_history(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    Ok(audit_history_async(&state, params).await)
}

async fn audit_history_async(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "audit_history", &params) {
        return final_diagnostics(state, Ok(error)).await.unwrap();
    }
    if !state.runtime.flush_audit().await && params["action"] == "preserve" {
        return state.runtime.with_diagnostics(request_error(
            "AuditFlushFailed",
            "Activity writes could not be confirmed; history was not moved",
        ));
    }
    let worker_state = state.clone();
    let result = tokio::task::spawn_blocking(move || audit_history_impl(&worker_state, params))
        .await
        .unwrap_or_else(|_| {
            request_error(
                "AuditHistoryFailed",
                "Local activity history could not be updated",
            )
        });
    final_diagnostics(state, Ok(result)).await.unwrap()
}

fn audit_history_impl(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "audit_history", &params) {
        return error;
    }
    let _guard = state.audit_settings.lock();
    let result = if params["action"] == "preserve" {
        crate::audit::preserve_history(&state.runtime.paths)
    } else {
        crate::audit::history_status(&state.runtime.paths)
    };
    let mut result = result.unwrap_or_else(|_| {
        request_error(
            "AuditHistoryFailed",
            "Local activity history could not be updated",
        )
    });
    result["mode"] = json!(state.runtime.audit_retention_mode().as_str());
    state.runtime.with_diagnostics(result)
}

#[tauri::command]
pub async fn aws_list_profiles(state: State<'_, AppState>) -> Result<Value, String> {
    final_diagnostics(&state, Ok(aws_list_profiles_impl(&state))).await
}

fn aws_list_profiles_impl(state: &AppState) -> Value {
    let user_settings = match load_settings(state) {
        Ok(settings) => settings,
        Err(error) => return state.runtime.with_diagnostics(error),
    };
    let cfg_path = config_path_from_settings(&user_settings);
    let constraint = settings::get_str(&user_settings, "sso_session_name");
    let mut info = state.runtime.aws.inspect_config(
        &cfg_path,
        (!constraint.is_empty()).then_some(constraint.as_str()),
    );
    info["allowed_regions"] = json!(settings::ALLOWED_REGIONS);
    state.runtime.with_diagnostics(info)
}

/// Local launch discovery only: no configuration, credential or process work.
#[tauri::command]
pub async fn cli_availability(state: State<'_, AppState>) -> Result<Value, String> {
    final_diagnostics(&state, Ok(cli_availability_impl(&state))).await
}

fn cli_availability_impl(state: &AppState) -> Value {
    state
        .runtime
        .with_diagnostics(state.runtime.process.availability().response())
}

#[tauri::command]
pub async fn aws_list_pipelines(
    state: State<'_, AppState>,
    params: Value,
) -> Result<Value, String> {
    final_diagnostics(&state, aws_list_pipelines_impl(&state, params).await).await
}

async fn aws_list_pipelines_impl(state: &AppState, params: Value) -> Result<Value, String> {
    let mut request = match request_envelope(state, "aws_list_pipelines", &params) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    let registration = match state.work.register(request.work_id()) {
        Ok(registration) => registration,
        Err(error) => return finish_request(request, Ok(error)),
    };
    let result = aws_list_pipelines_request(
        state,
        params,
        &mut request,
        registration.cancellation.clone(),
        tokio::time::Instant::now(),
    )
    .await;
    finish_request(request, result)
}

async fn aws_list_pipelines_request(
    state: &AppState,
    params: Value,
    request: &mut RequestEnvelope,
    cancellation: crate::process::ProcessCancellation,
    started: tokio::time::Instant,
) -> Result<Value, String> {
    let resolved = match resolve_widget_ctx(state, &params) {
        Ok(context) => context,
        Err(error) => return Ok(error),
    };
    let ctx = &resolved.context;
    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    let policy_revision = state.observe_policy(&policy);
    if let Err((action, reason)) = audit_aws_call(
        &request.runtime(&state.runtime),
        &policy,
        "codepipeline",
        "ListPipelines",
        &ctx.account_id,
        &ctx.region,
    ) {
        return Ok(request_error(
            "PolicyDenied",
            &format!("Request blocked: {action}: {reason}"),
        ));
    }
    let verification = verify_request_context_owned(state, &resolved, &policy, Some(request));
    let session = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Ok(crate::work_registry::cancelled_result()),
        _ = tokio::time::sleep_until(started + std::time::Duration::from_secs(30)) => return Ok(request_error("WorkDeadline", "Pipeline discovery reached its deadline")),
        verified = verification => match verified { Ok(session) => session, Err(error) => return Ok(error) },
    };
    request.bind(&resolved.context, &session);
    let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if state.observe_policy(&current_policy) != policy_revision {
        return Ok(request_error(
            "PolicyChanged",
            "Policy changed during verification",
        ));
    }
    let mut scope = crate::scheduler::WorkScope::new(
        json!([
            resolved.context.id(),
            session.provider_revision,
            policy_revision
        ])
        .to_string(),
        session.identity.account_id.clone(),
        resolved.context.region.clone(),
        cancellation.clone(),
        crate::scheduler::WorkBudget::for_widget("pipeline-selector"),
    );
    scope.deadline = started + scope.budget.deadline;
    let wctx = widgets::WidgetCtx {
        runtime: request.runtime(&state.runtime).with_work(scope),
        sdk: session.sdk.clone(),
        account_id: session.identity.account_id.clone(),
        region: resolved.context.region.clone(),
        widget_name: "pipeline-selector".into(),
        inputs: json!({}),
        policy: policy.clone(),
        cli: None,
    };
    let work = list_pipelines_scoped(&wctx);
    tokio::pin!(work);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            biased;
            _ = tick.tick() => {
                let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e|e.message);
                if validate_request_context(state, &resolved, &session).is_err() || state.observe_policy(&current_policy) != policy_revision {
                    cancellation.cancel();
                }
            }
            result = &mut work => break result,
        }
    };
    let current_policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if state.observe_policy(&current_policy) != policy_revision {
        return Ok(request_error(
            "PolicyChanged",
            "Policy changed during discovery",
        ));
    }
    if cancellation.is_cancelled() {
        return Ok(crate::work_registry::cancelled_result());
    }
    if let Err(error) = validate_request_context(state, &resolved, &session) {
        return Ok(error);
    }
    Ok(result)
}

/// The command supplies a verified, frozen SDK configuration and fences the
/// result afterwards. This helper retains pages without expanding the existing
/// 200-pipeline result budget.
#[cfg(test)]
async fn list_pipelines_with_coverage(sdk: &aws_config::SdkConfig) -> Value {
    let dir = crate::test_support::TestDir::new();
    aws::policy::load(&dir.paths()).expect("initialize disposable selector policy");
    let runtime = crate::runtime::Runtime::for_test(dir.paths());
    let scope = crate::scheduler::WorkScope::new(
        "synthetic-selector".into(),
        "acct-fixture".into(),
        "us-east-1".into(),
        crate::process::ProcessCancellation::new(),
        crate::scheduler::WorkBudget::for_widget("pipeline-selector"),
    );
    let wctx = widgets::WidgetCtx {
        runtime: runtime.with_work(scope),
        sdk: sdk.clone(),
        account_id: "acct-fixture".into(),
        region: "us-east-1".into(),
        widget_name: "pipeline-selector".into(),
        inputs: json!({}),
        policy: aws::policy::Policy::parse("statements:\n  - effect: Allow\n    action: ['*']\n")
            .map_err(|e| e.message),
        cli: None,
    };
    list_pipelines_scoped(&wctx).await
}

async fn list_pipelines_scoped(wctx: &widgets::WidgetCtx) -> Value {
    const RESULT_LIMIT: usize = 200;
    let client = aws_sdk_codepipeline::Client::new(&wctx.sdk);

    let mut pipelines: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    let mut pages = 0usize;
    let mut limited = false;
    let mut error = None;
    let mut budget = widgets::budget::PageBudget::default();
    loop {
        let mut req = client.list_pipelines();
        if let Some(t) = &token {
            req = req.next_token(t);
        }
        let resp = match wctx.send("codepipeline", "ListPipelines", req.send()).await {
            Ok(r) => r,
            Err(e) => {
                error = Some(widgets::err_msg(e));
                break;
            }
        };
        pages += 1;
        token = resp
            .next_token()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let continue_scan = budget.advance(token.as_deref());
        for (index, p) in resp.pipelines().iter().enumerate() {
            if let Some(name) = p.name() {
                let updated = widgets::dt_iso(p.updated().or_else(|| p.created()));
                let row = json!({"name": name, "updated": updated});
                if !budget.retain(&row) {
                    break;
                }
                pipelines.push(row);
                if pipelines.len() >= RESULT_LIMIT {
                    limited = index + 1 < resp.pipelines().len() || token.is_some();
                    break;
                }
            }
        }
        if pipelines.len() >= RESULT_LIMIT {
            break;
        }
        if !continue_scan || budget.stopped() {
            break;
        }
    }
    pipelines.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    let mut coverage = widgets::coverage::Coverage::complete(pipelines.len());
    coverage.count("pages", pages);
    coverage.limit("results", Some(RESULT_LIMIT));
    if limited {
        coverage.has_more(Some(true));
        coverage.limited(
            "result_limit",
            "Additional pipelines were not loaded after the 200-pipeline limit.",
        );
    }
    budget.apply(&mut coverage, !pipelines.is_empty());
    let mut result = json!({"ok":true, "pipelines":pipelines});
    if let Some(error) = error {
        coverage.failure(
            "request_failed",
            "A pipeline-list page could not be loaded; earlier pipelines are retained.",
            pages > 0,
        );
        result["error"] = json!(error);
        if pages == 0 {
            result["error_type"] = json!("ClientError");
        }
    }
    coverage.attach(result)
}

#[tauri::command]
pub async fn aws_auth_status(state: State<'_, AppState>) -> Result<Value, String> {
    final_diagnostics(&state, aws_auth_status_impl(&state).await).await
}

async fn aws_auth_status_impl(state: &AppState) -> Result<Value, String> {
    let mut request = RequestEnvelope::default().audited("aws_auth_status", &state.runtime);
    let result = aws_auth_status_request(state, &mut request).await;
    finish_request(request, result)
}

async fn aws_auth_status_request(
    state: &AppState,
    request: &mut RequestEnvelope,
) -> Result<Value, String> {
    if let Err(error) = refresh_configuration_revision(state) {
        return Ok(error);
    }
    let (last, set_at, status, attempt) = {
        let connection = state.connection.lock();
        (
            connection.last_attempt.clone(),
            connection.set_account_at,
            connection.status,
            connection.attempt,
        )
    };
    let lp = |key: &str| {
        last.as_ref()
            .and_then(|last| last.get(key))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let mut out = json!({
        "has_context": false, "profile": lp("profile"), "account_id": lp("account_id"),
        "region": lp("region"), "sso_session": lp("sso_session"), "caller_arn": Value::Null,
        "logged_in": false, "needs_sso_login": lp("needs_sso_login").as_bool().unwrap_or(false),
        "expires_at": Value::Null, "sso_token_expires_at": Value::Null,
        "set_account_at": set_at, "read_only_guard_active": true,
        "error": lp("error"), "connection_state": status, "attempt_id": attempt,
    });
    let resolved = match resolve_widget_ctx(state, &json!({})) {
        Ok(context) => context,
        Err(_) => return Ok(out),
    };
    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    let session = match verify_request_context_owned(state, &resolved, &policy, Some(request)).await
    {
        Ok(session) => session,
        Err(error) => {
            out["error"] = error
                .get("error")
                .or_else(|| error.get("reason"))
                .cloned()
                .unwrap_or(Value::Null);
            out["error_type"] = error.get("error_type").cloned().unwrap_or(Value::Null);
            out["needs_sso_login"] =
                json!(out["error"].as_str().is_some_and(aws::sso_login_required));
            let connection = state.connection.lock();
            out["connection_state"] = json!(if connection.active.is_none() {
                connection.status
            } else {
                "blocked"
            });
            out["set_account_at"] = json!(connection.set_account_at);
            // Never attach an older auth outcome to a newer connection attempt.
            if connection.attempt != attempt {
                out["error_type"] = json!("Superseded");
            }
            return Ok(out);
        }
    };
    // Read the selected SSO cache on every accepted poll, including when the
    // verified AWS role credentials are reused. A terminal login can replace
    // the cached token independently of those credentials. This read never
    // invokes an SSO provider; inability to inspect it means unknown expiry.
    let backend = state.runtime.aws.clone();
    let snapshot = session.snapshot.clone();
    let sso_token_expiry = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking(move || backend.sso_token_expiry(&snapshot)),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    // Metadata read completion cannot attach an older SSO cache to a newer
    // selected account, provider snapshot or settings generation.
    if let Err(error) = validate_request_context(state, &resolved, &session) {
        out["error_type"] = error["error_type"].clone();
        out["error"] = error["error"].clone();
        let connection = state.connection.lock();
        out["connection_state"] = json!(connection.status);
        out["set_account_at"] = json!(connection.set_account_at);
        if connection.attempt != attempt {
            out["error_type"] = json!("Superseded");
        }
        return Ok(out);
    }
    if state.connection.lock().attempt != attempt {
        out["error_type"] = json!("Superseded");
        return Ok(out);
    }
    request.bind(&resolved.context, &session);
    out["has_context"] = json!(true);
    out["logged_in"] = json!(true);
    out["connection_state"] = json!("verified");
    out["profile"] = json!(resolved.context.profile);
    out["account_id"] = json!(session.identity.account_id);
    out["region"] = json!(resolved.context.region);
    out["sso_session"] = json!(session.snapshot.session_name);
    out["caller_arn"] = json!(session.identity.arn);
    out["needs_sso_login"] = json!(false);
    out["error"] = Value::Null;
    out["expires_at"] = json!(aws_smithy_types::DateTime::from(session.expires_at)
        .fmt(aws_smithy_types::date_time::Format::DateTime)
        .unwrap_or_default());
    out["sso_token_expires_at"] =
        json!(
            sso_token_expiry.and_then(|expiry| aws_smithy_types::DateTime::from(expiry)
                .fmt(aws_smithy_types::date_time::Format::DateTime)
                .ok())
        );
    Ok(out)
}

/// Build the status payload the Settings panel renders.
fn policy_status(state: &AppState, raw: String) -> Value {
    match aws::policy::Policy::parse(&raw) {
        Ok(p) => json!({
            "ok": true,
            "raw": raw,
            "valid": true,
            "error": Value::Null,
            "actions": p.allow_summary(),
            "path": aws::policy::policy_path(&state.runtime.paths).to_string_lossy(),
        }),
        Err(e) => json!({
            "ok": true,
            "raw": raw,
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path(&state.runtime.paths).to_string_lossy(),
        }),
    }
}

#[tauri::command]
pub async fn policy_get(state: State<'_, AppState>) -> Result<Value, String> {
    final_diagnostics(&state, Ok(policy_get_impl(&state))).await
}

fn policy_get_impl(state: &AppState) -> Value {
    match aws::policy::raw_text(&state.runtime.paths) {
        Ok(raw) => policy_status(state, raw),
        Err(e) => {
            json!({"ok":false, "error_type":e.error_type(), "raw":"", "valid":false, "error":e.message, "actions":[],
            "path":aws::policy::policy_path(&state.runtime.paths).to_string_lossy()})
        }
    }
}

#[tauri::command]
pub async fn policy_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    final_diagnostics(&state, Ok(policy_set_impl(&state, params))).await
}

fn policy_set_impl(state: &AppState, params: Value) -> Value {
    if let Some(error) = local_validation(state, "policy_set", &params) {
        return error;
    }
    let text = params["text"].as_str().unwrap_or_default().to_string();
    let result = match aws::policy::write_text(&state.runtime.paths, &text) {
        Ok(_) => {
            state.observe_policy(&aws::policy::load(&state.runtime.paths).map_err(|e| e.message));
            policy_status(state, text)
        }
        // Only the intentional local policy editor receives its candidate text.
        Err(e) => {
            json!({"ok":false, "error_type":e.error_type(), "raw":text, "valid":false, "error":e.message, "actions":[],
            "path":aws::policy::policy_path(&state.runtime.paths).to_string_lossy()})
        }
    };
    state.runtime.with_diagnostics(result)
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

#[cfg(test)]
mod pipeline_coverage_tests {
    use super::list_pipelines_with_coverage;
    use crate::test_aws::{ExpectedRequest, ScriptedHttp};
    use serde_json::{json, Value};

    const TARGET: &str = "CodePipeline_20150709.ListPipelines";

    #[tokio::test]
    async fn pipeline_listing_keeps_sorted_earlier_pages_after_later_failure() {
        let private = "CB_SYNTHETIC_PRIVATE_PIPELINE_DETAIL";
        let http = ScriptedHttp::new(vec![
            ExpectedRequest::json(
                TARGET,
                json!({"nextToken":null,"maxResults":null}),
                json!({"pipelines":[{"name":"synthetic-z"},{"name":"synthetic-a"}],"nextToken":"synthetic-next"}),
            ),
            ExpectedRequest::json(
                TARGET,
                json!({"nextToken":"synthetic-next","maxResults":null}),
                json!({"__type":"AccessDeniedException","message":private}),
            )
            .status(403),
        ]);
        let result = list_pipelines_with_coverage(&http.sdk_config()).await;
        http.assert_finished();
        assert_eq!(http.calls(), 2);
        assert_eq!(result["pipelines"][0]["name"], "synthetic-a");
        assert_eq!(result["pipelines"][1]["name"], "synthetic-z");
        assert_eq!(result["ok"], false);
        assert_eq!(result["partial"], true);
        assert_eq!(result["error_type"], "PartialFailure");
        assert_eq!(result["coverage"]["counts"]["returned"], 2);
        assert_eq!(result["coverage"]["counts"]["pages"], 1);
        assert_eq!(result["coverage"]["has_more"], Value::Null);
        assert_eq!(crate::request::outcome(&result), "failed");
        assert!(!result.to_string().contains(private));
    }

    #[tokio::test]
    async fn pipeline_listing_reports_mid_page_and_token_caps_without_an_extra_request() {
        for (count, next, limited) in [
            (200, None, false),
            (201, None, true),
            (200, Some("synthetic-next"), true),
        ] {
            let pipelines = (0..count)
                .map(|index| json!({"name":format!("synthetic-pipeline-{index:03}")}))
                .collect::<Vec<_>>();
            let http = ScriptedHttp::new(vec![ExpectedRequest::json(
                TARGET,
                json!({"nextToken":null,"maxResults":null}),
                json!({"pipelines":pipelines,"nextToken":next}),
            )]);
            let result = list_pipelines_with_coverage(&http.sdk_config()).await;
            http.assert_finished();
            assert_eq!(http.calls(), 1);
            assert_eq!(result["pipelines"].as_array().unwrap().len(), 200);
            assert_eq!(result["ok"], true);
            assert_eq!(result["coverage"]["counts"]["pages"], 1);
            assert_eq!(result["coverage"]["limits"]["results"], 200);
            assert_eq!(result["coverage"]["has_more"], limited);
            assert_eq!(
                result["coverage"]["completeness"],
                if limited { "limited" } else { "complete" }
            );
            assert_eq!(
                result.get("truncated"),
                limited.then_some(&Value::Bool(true))
            );
            assert_eq!(crate::request::outcome(&result), "succeeded");
        }
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    use crate::process::{CliAvailability, CliRequest, ProcessOutput, ProcessRunner};
    use crate::runtime::Runtime;
    use crate::test_support::TestDir;
    use futures::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct AvailabilityFixture {
        result: CliAvailability,
        calls: AtomicUsize,
    }

    impl ProcessRunner for AvailabilityFixture {
        fn availability(&self) -> CliAvailability {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result
        }

        fn run(&self, _request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
            panic!("the availability command must not execute a process")
        }
    }

    #[test]
    fn cli_availability_command_is_injected_and_independent_of_settings_and_aws() {
        for (availability, status, available) in [
            (CliAvailability::Available, "available", json!(true)),
            (CliAvailability::Missing, "missing", json!(false)),
            (CliAvailability::Unknown, "unknown", Value::Null),
        ] {
            let directory = TestDir::new();
            let mut runtime = Runtime::for_test(directory.paths());
            let process = Arc::new(AvailabilityFixture {
                result: availability,
                calls: AtomicUsize::new(0),
            });
            runtime.process = process.clone();
            // The remaining native AWS boundary panics if it is accidentally
            // reached. No settings, credentials or audit file is needed.
            let state = AppState::with_runtime(runtime);
            let response = cli_availability_impl(&state);
            assert_eq!(response["ok"], true);
            assert_eq!(response["status"], status);
            assert_eq!(response["available"], available);
            assert_eq!(response["version_verified"], false);
            assert!(response.get("path").is_none());
            assert_eq!(process.calls.load(Ordering::SeqCst), 1);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }
}
