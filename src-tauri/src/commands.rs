//! The local Tauri command surface with additive request/context metadata.

use serde_json::{json, Value};
use tauri::State;

use crate::aws::{self, AwsContext};
use crate::request::RequestEnvelope;
use crate::state::AppState;
use crate::{dashboard, settings, widgets};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn config_path_from_settings(s: &Value) -> String {
    let p = settings::get_str(s, "aws_config_path");
    if p.is_empty() {
        "~/.aws/config".to_string()
    } else {
        p
    }
}

fn audit_aws_call(
    state: &AppState,
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
            state.runtime.audit(json!({
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
            state.runtime.audit(json!({
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
            state.runtime.audit(json!({
                "kind": "aws",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
            }));
            Ok(())
        }
        Err(reason) => {
            state.runtime.audit(json!({
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
    aws_set_account_impl(&state, params).await
}

async fn aws_set_account_impl(state: &AppState, params: Value) -> Result<Value, String> {
    let mut request = match request_envelope(&params) {
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
    let user_settings = settings::load(&state.runtime.paths);
    let cfg_path = config_path_from_settings(&user_settings);
    let revision = state.observe_config_path(&cfg_path);
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
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| settings::get_str(&user_settings, "default_region"));
    let region = if region.is_empty() {
        "eu-west-1".into()
    } else {
        region
    };
    let session = params
        .get("sso_session_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let Some(attempt) = state.begin_attempt(
        json!({
            "profile": profile, "account_id": account_id, "region": region,
            "sso_session": session, "ts": state.runtime.clock.now_epoch(),
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
    if let Err((action, reason)) = verification_gate(state, &policy, &ctx) {
        return Ok(finish_connection_failure(
            state,
            attempt,
            "PolicyDenied",
            &format!("Request blocked: {action}: {reason}"),
            false,
        ));
    }
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
    refresh_configuration_revision(state);
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

fn request_envelope(params: &Value) -> Result<RequestEnvelope, Value> {
    RequestEnvelope::from_params(params).map_err(|message| {
        RequestEnvelope::default().attach(request_error("InvalidRequest", message))
    })
}

fn finish_request(
    request: RequestEnvelope,
    result: Result<Value, String>,
) -> Result<Value, String> {
    Ok(request.attach(result.unwrap_or_else(|message| request_error("RequestFailed", &message))))
}

fn superseded() -> Value {
    request_error(
        "Superseded",
        "The account selection or configuration changed; retry in the current context",
    )
}

fn refresh_configuration_revision(state: &AppState) -> u64 {
    state.observe_config_path(&config_path_from_settings(&settings::load(
        &state.runtime.paths,
    )))
}

fn finish_connection_failure(
    state: &AppState,
    attempt: u64,
    error_type: &str,
    message: &str,
    needs_sso_login: bool,
) -> Value {
    refresh_configuration_revision(state);
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
    state: &AppState,
    policy: &Result<aws::policy::Policy, String>,
    ctx: &AwsContext,
) -> Result<(), (String, String)> {
    audit_aws_call(
        state,
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

struct ResolvedContext {
    context: AwsContext,
    /// Pinned requests are independent of topbar attempts; settings still apply.
    inherited_attempt: Option<u64>,
}

fn resolve_widget_ctx(state: &AppState, params: &Value) -> Result<ResolvedContext, Value> {
    let user_settings = settings::load(&state.runtime.paths);
    let path = config_path_from_settings(&user_settings);
    let revision = state.observe_config_path(&path);
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
            let connection = state.connection.lock();
            let cached = connection
                .overrides
                .values()
                .find(|cached| {
                    let ctx = &cached.context;
                    ctx.profile == profile
                        && ctx.account_id == account
                        && ctx.region == region
                        && ctx.settings_revision == revision
                        && ctx.aws_config_path == path
                })
                .map(|cached| cached.context.clone());
            let context = cached.unwrap_or_else(|| {
                AwsContext::new(
                    profile.into(),
                    account.into(),
                    region.into(),
                    None,
                    path,
                    state.runtime.clone(),
                )
                .with_settings_revision(revision)
            });
            return Ok(ResolvedContext {
                context,
                inherited_attempt: None,
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
    Ok(ResolvedContext {
        context,
        inherited_attempt: Some(connection.attempt),
    })
}

fn validate_request_context(
    state: &AppState,
    resolved: &ResolvedContext,
    session: &std::sync::Arc<aws::context::VerifiedSession>,
) -> Result<(), Value> {
    refresh_configuration_revision(state);
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

async fn verify_request_context(
    state: &AppState,
    resolved: &ResolvedContext,
    policy: &Result<aws::policy::Policy, String>,
) -> Result<std::sync::Arc<aws::context::VerifiedSession>, Value> {
    let ctx = &resolved.context;
    if let Err((action, reason)) = verification_gate(state, policy, ctx) {
        let mut denial = widgets::permission_denied_render("sts", "GetCallerIdentity", &reason);
        denial["action"] = json!(action);
        denial["ok"] = json!(false);
        denial["error"] = json!(reason);
        denial["error_type"] = json!("PolicyDenied");
        return Err(denial);
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
            },
        );
    }
    Ok(session)
}

#[tauri::command]
pub async fn widget_fetch(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    widget_fetch_impl(&state, params).await
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
    let mut request = match request_envelope(&params) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    let result = widget_fetch_request(state, params, &mut request).await;
    finish_request(request, result)
}

async fn widget_fetch_request(
    state: &AppState,
    params: Value,
    request: &mut RequestEnvelope,
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
        return Ok(
            json!({"render": "raw_json", "data": {"error": format!("Unknown widget: {name}")}}),
        );
    }

    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    let inputs = params.get("inputs").cloned().unwrap_or_else(|| json!({}));
    if name == "aws-cli" {
        let command = inputs.get("command").and_then(Value::as_str).unwrap_or("");
        let parsed = match widgets::parse_cli_command(command) {
            Ok(parsed) => parsed,
            Err(error) => return Ok(request_error("UnsupportedCommand", &error)),
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
            state,
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
    let session = match verify_request_context(state, &resolved, &policy).await {
        Ok(session) => session,
        Err(error) => return Ok(error),
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
    let ctx = &resolved.context;
    let wctx = widgets::WidgetCtx {
        runtime: state.runtime.clone(),
        sdk: session.sdk.clone(),
        account_id: session.identity.account_id.clone(),
        region: ctx.region.clone(),
        widget_name: name,
        inputs,
        policy,
        cli,
    };
    let result = if let Some(cli) = &wctx.cli {
        let work = widgets::fetch(&wctx.widget_name, &wctx);
        tokio::pin!(work);
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = tick.tick() => {
                    if let Err(error) = validate_request_context(state, &resolved, &session) {
                        cli.cancellation.cancel();
                        // The process owner confirms termination/reaping before
                        // this command reports that the context was superseded.
                        let cleanup = work.await;
                        if retain_cli_cleanup_failure(&wctx, &cleanup) {
                            return Ok(cleanup);
                        }
                        return Ok(error);
                    }
                }
                result = &mut work => break result,
            }
        }
    } else {
        widgets::fetch(&wctx.widget_name, &wctx).await
    };
    if retain_cli_cleanup_failure(&wctx, &result) {
        return Ok(result);
    }
    if let Err(error) = validate_request_context(state, &resolved, &session) {
        return Ok(error);
    }
    Ok(result)
}

#[tauri::command]
pub async fn widget_get_source(params: Value) -> Result<Value, String> {
    let name = params.get("widget").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Ok(json!({"ok": false, "error": "missing required field 'widget'"}));
    }
    Ok(widgets::get_source(name))
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<Value, String> {
    Ok(settings::load(&state.runtime.paths))
}

#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    let result = settings::save(&state.runtime.paths, &params);
    refresh_configuration_revision(&state);
    Ok(result)
}

#[tauri::command]
pub async fn dashboard_get(state: State<'_, AppState>) -> Result<Value, String> {
    Ok(dashboard::load(&state.runtime.paths))
}

#[tauri::command]
pub async fn dashboard_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    let tiles = params.get("tiles").cloned().unwrap_or_else(|| json!([]));
    Ok(dashboard::save(&state.runtime.paths, &tiles))
}

#[tauri::command]
pub async fn audit_tail(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    Ok(json!({"entries": crate::audit::tail(&state.runtime.paths, limit)}))
}

#[tauri::command]
pub async fn aws_list_profiles(state: State<'_, AppState>) -> Result<Value, String> {
    let cfg_path = config_path_from_settings(&settings::load(&state.runtime.paths));
    let mut info = state.runtime.aws.inspect_config(&cfg_path);
    info["allowed_regions"] = json!(settings::ALLOWED_REGIONS);
    Ok(info)
}

#[tauri::command]
pub async fn aws_list_pipelines(
    state: State<'_, AppState>,
    params: Value,
) -> Result<Value, String> {
    aws_list_pipelines_impl(&state, params).await
}

async fn aws_list_pipelines_impl(state: &AppState, params: Value) -> Result<Value, String> {
    let mut request = match request_envelope(&params) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    let result = aws_list_pipelines_request(state, params, &mut request).await;
    finish_request(request, result)
}

async fn aws_list_pipelines_request(
    state: &AppState,
    params: Value,
    request: &mut RequestEnvelope,
) -> Result<Value, String> {
    let resolved = match resolve_widget_ctx(state, &params) {
        Ok(context) => context,
        Err(error) => return Ok(error),
    };
    let ctx = &resolved.context;
    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    if let Err((action, reason)) = audit_aws_call(
        state,
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
    let session = match verify_request_context(state, &resolved, &policy).await {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    request.bind(&resolved.context, &session);
    let client = aws_sdk_codepipeline::Client::new(&session.sdk);

    let mut pipelines: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut req = client.list_pipelines();
        if let Some(t) = &token {
            req = req.next_token(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                if let Err(error) = validate_request_context(state, &resolved, &session) {
                    return Ok(error);
                }
                return Ok(
                    json!({"ok": false, "error": widgets::err_msg(e), "error_type": "ClientError"}),
                );
            }
        };
        for p in resp.pipelines() {
            if let Some(name) = p.name() {
                let updated = widgets::dt_iso(p.updated().or_else(|| p.created()));
                pipelines.push(json!({"name": name, "updated": updated}));
                if pipelines.len() >= 200 {
                    break;
                }
            }
        }
        if pipelines.len() >= 200 {
            break;
        }
        token = resp
            .next_token()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if token.is_none() {
            break;
        }
    }
    pipelines.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    if let Err(error) = validate_request_context(state, &resolved, &session) {
        return Ok(error);
    }
    Ok(json!({"ok": true, "pipelines": pipelines}))
}

#[tauri::command]
pub async fn aws_auth_status(state: State<'_, AppState>) -> Result<Value, String> {
    aws_auth_status_impl(&state).await
}

async fn aws_auth_status_impl(state: &AppState) -> Result<Value, String> {
    let mut request = RequestEnvelope::default();
    let result = aws_auth_status_request(state, &mut request).await;
    finish_request(request, result)
}

async fn aws_auth_status_request(
    state: &AppState,
    request: &mut RequestEnvelope,
) -> Result<Value, String> {
    refresh_configuration_revision(state);
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
        "expires_at": Value::Null, "set_account_at": set_at, "read_only_guard_active": true,
        "error": lp("error"), "connection_state": status, "attempt_id": attempt,
    });
    let resolved = match resolve_widget_ctx(state, &json!({})) {
        Ok(context) => context,
        Err(_) => return Ok(out),
    };
    let policy = aws::policy::load(&state.runtime.paths).map_err(|e| e.message);
    let session = match verify_request_context(state, &resolved, &policy).await {
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
    Ok(out)
}

/// Build the status payload the Settings panel renders.
fn policy_status(state: &AppState, raw: String) -> Value {
    match aws::policy::Policy::parse(&raw) {
        Ok(p) => json!({
            "raw": raw,
            "valid": true,
            "error": Value::Null,
            "actions": p.allow_summary(),
            "path": aws::policy::policy_path(&state.runtime.paths).to_string_lossy(),
        }),
        Err(e) => json!({
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
    match aws::policy::raw_text(&state.runtime.paths) {
        Ok(raw) => Ok(policy_status(&state, raw)),
        Err(e) => Ok(json!({
            "raw": "",
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path(&state.runtime.paths).to_string_lossy(),
        })),
    }
}

#[tauri::command]
pub async fn policy_set(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
    let text = params
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match aws::policy::write_text(&state.runtime.paths, &text) {
        Ok(_) => Ok(policy_status(&state, text)),
        // Return the candidate text + error WITHOUT writing, so the editor keeps it.
        Err(e) => Ok(json!({
            "raw": text,
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path(&state.runtime.paths).to_string_lossy(),
        })),
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
