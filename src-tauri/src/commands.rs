//! The Tauri command surface. Command names and request/response JSON shapes
//! are identical to the original Python-sidecar RPC design, so the frontend is unchanged.

use serde_json::{json, Value};
use tauri::State;

use crate::aws::config_file;
use crate::aws::{self, AwsContext};
use crate::state::AppState;
use crate::{audit, dashboard, settings, widgets};

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
            audit::append(json!({
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
            audit::append(json!({
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
            audit::append(json!({
                "kind": "aws",
                "service": service,
                "operation": operation,
                "account_id": account_id,
                "region": region,
            }));
            Ok(())
        }
        Err(reason) => {
            audit::append(json!({
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
    audit::append(json!({
        "kind": "lifecycle",
        "event": "set_account_attempt",
        "profile": params.get("profile"),
        "account_id": params.get("account_id"),
        "region": params.get("region"),
    }));

    let profile = params
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let account_id = params
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if profile.is_empty() {
        return Ok(json!({"ok": false, "error": "missing required field 'profile'"}));
    }
    if account_id.is_empty() {
        return Ok(json!({"ok": false, "error": "missing required field 'account_id'"}));
    }

    let user_settings = settings::load();
    let cfg_path = config_path_from_settings(&user_settings);
    // A custom config path is applied via AWS_CONFIG_FILE (read by aws-config);
    // clearing it when back on the default lets the user *unset* a custom path.
    if cfg_path != "~/.aws/config" {
        std::env::set_var("AWS_CONFIG_FILE", config_file::expand(&cfg_path));
    } else {
        std::env::remove_var("AWS_CONFIG_FILE");
    }

    let region = {
        let r = params.get("region").and_then(|v| v.as_str()).unwrap_or("");
        let r = if r.is_empty() {
            settings::get_str(&user_settings, "default_region")
        } else {
            r.to_string()
        };
        if r.is_empty() {
            "eu-west-1".to_string()
        } else {
            r
        }
    };
    let sso_session = {
        let p = params
            .get("sso_session_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let s = if p.is_empty() {
            settings::get_str(&user_settings, "sso_session_name")
        } else {
            p.to_string()
        };
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    let ctx = AwsContext::new(
        profile.clone(),
        account_id.clone(),
        region.clone(),
        sso_session.clone(),
        cfg_path,
    );
    *state.last_attempt.lock() = Some(json!({
        "profile": profile,
        "account_id": account_id,
        "region": region,
        "sso_session": sso_session,
        "ts": audit::now_epoch(),
    }));

    let policy = aws::policy::load().map_err(|e| e.message);
    if let Err((action, reason)) =
        audit_aws_call(&policy, "sso", "GetRoleCredentials", &account_id, &region)
    {
        let error = format!("missing permission: {action} — {reason}");
        {
            let mut last = state.last_attempt.lock();
            if let Some(obj) = last.as_mut().and_then(Value::as_object_mut) {
                obj.insert("error".into(), json!(error.clone()));
                obj.insert("error_type".into(), json!("PolicyDenied"));
                obj.insert("needs_sso_login".into(), json!(false));
            }
        }
        *state.active.write() = None;
        audit::append(json!({
            "kind": "lifecycle",
            "event": "set_account_failed",
            "error_type": "PolicyDenied",
            "needs_sso_login": false,
        }));
        return Ok(json!({
            "ok": false,
            "needs_sso_login": false,
            "error": error,
            "error_type": "PolicyDenied",
        }));
    }

    match ctx.resolve_credentials().await {
        Ok(()) => {
            *state.active.write() = Some(ctx);
            *state.set_account_at.lock() = Some(audit::now_epoch());
            audit::append(json!({
                "kind": "lifecycle", "event": "set_account_ok",
                "account_id": account_id, "region": region,
            }));
            Ok(json!({"ok": true, "account_id": account_id, "region": region}))
        }
        Err(e) => {
            *state.active.write() = None;
            let needs = aws::sso_login_required(&e);
            {
                let mut last = state.last_attempt.lock();
                if let Some(obj) = last.as_mut().and_then(Value::as_object_mut) {
                    obj.insert("error".into(), json!(e.clone()));
                    obj.insert("error_type".into(), json!("CredentialsError"));
                    obj.insert("needs_sso_login".into(), json!(needs));
                }
            }
            audit::append(json!({
                "kind": "lifecycle", "event": "set_account_failed",
                "error_type": "CredentialsError", "needs_sso_login": needs,
            }));
            Ok(json!({
                "ok": false, "needs_sso_login": needs,
                "error": e, "error_type": "CredentialsError",
            }))
        }
    }
}

fn pinned_context_fields(scope: &Value) -> Option<(&str, &str, &str)> {
    let profile = scope
        .get("profile")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let account = scope
        .get("account_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let region = scope
        .get("region")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match (profile, account, region) {
        (Some(p), Some(a), Some(r)) => Some((p, a, r)),
        _ => None,
    }
}

fn cached_pinned_context(
    state: &AppState,
    profile: &str,
    account: &str,
    region: &str,
) -> AwsContext {
    let key = (profile.to_string(), account.to_string(), region.to_string());
    let mut cache = state.overrides.lock();
    if let Some(c) = cache.get(&key) {
        return c.clone();
    }
    let user_settings = settings::load();
    let sso = {
        let s = settings::get_str(&user_settings, "sso_session_name");
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    let cfg_path = config_path_from_settings(&user_settings);
    let ctx = AwsContext::new(
        profile.to_string(),
        account.to_string(),
        region.to_string(),
        sso,
        cfg_path,
    );
    cache.insert(key, ctx.clone());
    ctx
}

/// Resolve the context for a widget-scoped command: a valid pinned per-tile
/// context, else the active topbar default. Legacy `account_override` payloads
/// are still accepted for existing saved dashboards.
fn resolve_widget_ctx(state: &AppState, params: &Value) -> Option<AwsContext> {
    let explicit = params
        .get("context")
        .or_else(|| params.get("account_override"));

    if let Some(scope) = explicit {
        let mode = scope.get("mode").and_then(|v| v.as_str()).unwrap_or("");
        if mode == "pinned" {
            return pinned_context_fields(scope)
                .map(|(p, a, r)| cached_pinned_context(state, p, a, r));
        }
        if mode.is_empty() {
            if let Some((p, a, r)) = pinned_context_fields(scope) {
                return Some(cached_pinned_context(state, p, a, r));
            }
        }
    }

    state.current_ctx()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROD_ACCOUNT: &str = "acct-prod-fixture";
    const DEV_ACCOUNT: &str = "acct-dev-fixture";

    fn ctx(profile: &str, account: &str, region: &str) -> AwsContext {
        AwsContext::new(
            profile.to_string(),
            account.to_string(),
            region.to_string(),
            None,
            "~/.aws/config".to_string(),
        )
    }

    #[tokio::test]
    async fn ping_reports_the_cargo_package_version() {
        let response = ping().await.unwrap();

        assert_eq!(response["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn pinned_context_resolves_without_active_topbar_context() {
        let state = AppState::default();
        let params = json!({
            "context": {
                "mode": "pinned",
                "profile": "prod",
                "account_id": PROD_ACCOUNT,
                "region": "eu-west-1",
            }
        });

        let resolved = resolve_widget_ctx(&state, &params).unwrap();

        assert_eq!(resolved.profile, "prod");
        assert_eq!(resolved.account_id, PROD_ACCOUNT);
        assert_eq!(resolved.region, "eu-west-1");
    }

    #[test]
    fn inherited_context_uses_active_topbar_context() {
        let state = AppState::default();
        *state.active.write() = Some(ctx("dev", DEV_ACCOUNT, "us-east-1"));
        let params = json!({"context": {"mode": "inherit"}});

        let resolved = resolve_widget_ctx(&state, &params).unwrap();

        assert_eq!(resolved.profile, "dev");
        assert_eq!(resolved.account_id, DEV_ACCOUNT);
        assert_eq!(resolved.region, "us-east-1");
    }

    #[test]
    fn incomplete_pinned_context_does_not_fall_back_to_active_context() {
        let state = AppState::default();
        *state.active.write() = Some(ctx("dev", DEV_ACCOUNT, "us-east-1"));
        let params = json!({
            "context": {
                "mode": "pinned",
                "profile": "prod",
                "region": "eu-west-1",
            }
        });

        assert!(resolve_widget_ctx(&state, &params).is_none());
    }
}

#[tauri::command]
pub async fn widget_fetch(state: State<'_, AppState>, params: Value) -> Result<Value, String> {
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

    let aws_ctx = match resolve_widget_ctx(&state, &params) {
        Some(c) => c,
        None => {
            return Ok(json!({"render": "raw_json", "data": {"error": "No AWS account selected"}}));
        }
    };

    let inputs = params.get("inputs").cloned().unwrap_or_else(|| json!({}));
    let sdk = aws_ctx.sdk_config().await.clone();
    let wctx = widgets::WidgetCtx {
        sdk,
        profile: aws_ctx.profile.clone(),
        account_id: aws_ctx.account_id.clone(),
        region: aws_ctx.region.clone(),
        widget_name: name.clone(),
        inputs,
        policy: aws::policy::load().map_err(|e| e.message),
    };
    Ok(widgets::fetch(&name, &wctx).await)
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
pub async fn settings_get() -> Result<Value, String> {
    Ok(settings::load())
}

#[tauri::command]
pub async fn settings_set(params: Value) -> Result<Value, String> {
    Ok(settings::save(&params))
}

#[tauri::command]
pub async fn dashboard_get() -> Result<Value, String> {
    Ok(dashboard::load())
}

#[tauri::command]
pub async fn dashboard_set(params: Value) -> Result<Value, String> {
    let tiles = params.get("tiles").cloned().unwrap_or_else(|| json!([]));
    Ok(dashboard::save(&tiles))
}

#[tauri::command]
pub async fn audit_tail(params: Value) -> Result<Value, String> {
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    Ok(json!({"entries": audit::tail(limit)}))
}

#[tauri::command]
pub async fn aws_list_profiles() -> Result<Value, String> {
    let cfg_path = config_path_from_settings(&settings::load());
    let mut info = config_file::inspect(&cfg_path);
    info["allowed_regions"] = json!(settings::ALLOWED_REGIONS);
    Ok(info)
}

#[tauri::command]
pub async fn aws_list_pipelines(
    state: State<'_, AppState>,
    params: Value,
) -> Result<Value, String> {
    let ctx = match resolve_widget_ctx(&state, &params) {
        Some(c) => c,
        None => {
            return Ok(
                json!({"ok": false, "error": "no context — pick a default account or pin this widget"}),
            );
        }
    };
    let policy = aws::policy::load().map_err(|e| e.message);
    if let Err((action, reason)) = audit_aws_call(
        &policy,
        "codepipeline",
        "ListPipelines",
        &ctx.account_id,
        &ctx.region,
    ) {
        return Ok(json!({
            "ok": false,
            "error": format!("missing permission: {action} — {reason}"),
            "error_type": "PolicyDenied",
        }));
    }
    let sdk = ctx.sdk_config().await.clone();
    let client = aws_sdk_codepipeline::Client::new(&sdk);

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
    Ok(json!({"ok": true, "pipelines": pipelines}))
}

#[tauri::command]
pub async fn aws_auth_status(state: State<'_, AppState>) -> Result<Value, String> {
    let last = state.last_attempt.lock().clone();
    let set_at = *state.set_account_at.lock();
    let lp = |k: &str| {
        last.as_ref()
            .and_then(|l| l.get(k).cloned())
            .unwrap_or(Value::Null)
    };

    let mut out = json!({
        "has_context": false,
        "profile": lp("profile"),
        "account_id": lp("account_id"),
        "region": lp("region"),
        "sso_session": lp("sso_session"),
        "caller_arn": Value::Null,
        "logged_in": false,
        "needs_sso_login": false,
        "expires_at": Value::Null,
        "set_account_at": set_at,
        "read_only_guard_active": true,
        "error": Value::Null,
    });

    let ctx = match state.current_ctx() {
        None => {
            if let Some(err) = last
                .as_ref()
                .and_then(|l| l.get("error"))
                .and_then(|v| v.as_str())
            {
                out["error"] = json!(err);
            } else {
                out["error"] = if last.is_none() {
                    json!("no active context — pick a default account in the topbar")
                } else {
                    json!("last set-account attempt did not succeed — open the Identity panel for details")
                };
            }
            out["needs_sso_login"] = last
                .as_ref()
                .and_then(|l| l.get("needs_sso_login"))
                .and_then(Value::as_bool)
                .map(Value::from)
                .unwrap_or(Value::Bool(false));
            if let Some(s) = lp("sso_session").as_str() {
                out["expires_at"] = config_file::read_sso_token(s)
                    .get("expires_at")
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            return Ok(out);
        }
        Some(c) => c,
    };

    out["has_context"] = json!(true);
    out["profile"] = json!(ctx.profile);
    out["account_id"] = json!(ctx.account_id);
    out["region"] = json!(ctx.region);
    out["sso_session"] = ctx
        .sso_session_name
        .clone()
        .map(Value::from)
        .unwrap_or(Value::Null);

    let policy = aws::policy::load().map_err(|e| e.message);
    if let Err((action, reason)) = audit_aws_call(
        &policy,
        "sts",
        "GetCallerIdentity",
        &ctx.account_id,
        &ctx.region,
    ) {
        out["error"] = json!(format!("missing permission: {action} — {reason}"));
        if let Some(s) = &ctx.sso_session_name {
            out["expires_at"] = config_file::read_sso_token(s)
                .get("expires_at")
                .cloned()
                .unwrap_or(Value::Null);
        }
        return Ok(out);
    }
    let sdk = ctx.sdk_config().await.clone();
    let sts = aws_sdk_sts::Client::new(&sdk);
    match sts.get_caller_identity().send().await {
        Ok(id) => {
            out["caller_arn"] = json!(id.arn().unwrap_or(""));
            out["logged_in"] = json!(true);
        }
        Err(e) => {
            let m = widgets::err_msg(e);
            out["needs_sso_login"] = json!(aws::sso_login_required(&m));
            out["error"] = json!(m);
        }
    }
    if let Some(s) = &ctx.sso_session_name {
        out["expires_at"] = config_file::read_sso_token(s)
            .get("expires_at")
            .cloned()
            .unwrap_or(Value::Null);
    }
    Ok(out)
}

/// Build the status payload the Settings panel renders.
fn policy_status(raw: String) -> Value {
    match aws::policy::Policy::parse(&raw) {
        Ok(p) => json!({
            "raw": raw,
            "valid": true,
            "error": Value::Null,
            "actions": p.allow_summary(),
            "path": aws::policy::policy_path().to_string_lossy(),
        }),
        Err(e) => json!({
            "raw": raw,
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path().to_string_lossy(),
        }),
    }
}

#[tauri::command]
pub async fn policy_get() -> Result<Value, String> {
    match aws::policy::raw_text() {
        Ok(raw) => Ok(policy_status(raw)),
        Err(e) => Ok(json!({
            "raw": "",
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path().to_string_lossy(),
        })),
    }
}

#[tauri::command]
pub async fn policy_set(params: Value) -> Result<Value, String> {
    let text = params
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match aws::policy::write_text(&text) {
        Ok(_) => Ok(policy_status(text)),
        // Return the candidate text + error WITHOUT writing, so the editor keeps it.
        Err(e) => Ok(json!({
            "raw": text,
            "valid": false,
            "error": e.message,
            "actions": [],
            "path": aws::policy::policy_path().to_string_lossy(),
        })),
    }
}
