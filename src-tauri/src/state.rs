//! Atomic connection state. Only the latest attempt may publish an outcome.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::aws::config_file::SsoProfileSnapshot;
use crate::aws::AwsContext;
use crate::runtime::Runtime;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct PinnedKey {
    pub snapshot: SsoProfileSnapshot,
    pub account_id: String,
    pub arn: String,
    pub user_id: String,
    pub provider_revision: u64,
}

pub(crate) struct CachedContext {
    pub context: AwsContext,
    pub last_used: std::time::Instant,
}

pub(crate) struct PendingContext {
    pub context: AwsContext,
    pub lease: Weak<ContextLease>,
}

pub(crate) struct ContextLease {
    pub connection: Weak<Mutex<ConnectionState>>,
    pub key: String,
    pub context_id: u64,
}

impl Drop for ContextLease {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.upgrade() {
            let mut connection = connection.lock();
            if connection
                .pending_contexts
                .get(&self.key)
                .is_some_and(|pending| pending.context.id() == self.context_id)
            {
                connection.pending_contexts.remove(&self.key);
            }
        }
    }
}

pub(crate) struct ConnectionState {
    pub attempt: u64,
    pub settings_revision: u64,
    config_path: Option<String>,
    sso_constraint: String,
    settings_failed: bool,
    pub status: &'static str,
    pub active: Option<AwsContext>,
    pub overrides: HashMap<PinnedKey, CachedContext>,
    pub pending_contexts: HashMap<String, PendingContext>,
    pub last_attempt: Option<Value>,
    pub set_account_at: Option<f64>,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            attempt: 0,
            settings_revision: 0,
            config_path: None,
            sso_constraint: String::new(),
            settings_failed: false,
            status: "disconnected",
            active: None,
            overrides: HashMap::new(),
            pending_contexts: HashMap::new(),
            last_attempt: None,
            set_account_at: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    pub runtime: Runtime,
    pub(crate) connection: Arc<Mutex<ConnectionState>>,
    pub(crate) work: Arc<crate::work_registry::WorkRegistry>,
    /// Serializes local settings saves, retention activation and preservation.
    pub(crate) audit_settings: Arc<Mutex<()>>,
    observed_policy: Arc<Mutex<(Option<String>, u64)>>,
    pub(crate) results: Arc<crate::result_cache::ResultCache>,
}

impl AppState {
    #[cfg(test)]
    pub fn with_runtime(runtime: Runtime) -> Self {
        Self {
            runtime,
            connection: Arc::default(),
            work: Arc::default(),
            audit_settings: Arc::default(),
            observed_policy: Arc::default(),
            results: Arc::default(),
        }
    }

    pub(crate) fn observe_policy(
        &self,
        policy: &Result<crate::aws::policy::Policy, String>,
    ) -> u64 {
        let key = match policy {
            Ok(policy) => serde_json::to_string(
                &policy
                    .statements
                    .iter()
                    .map(|statement| {
                        (
                            match statement.effect {
                                crate::aws::policy::Effect::Allow => "allow",
                                crate::aws::policy::Effect::Deny => "deny",
                            },
                            &statement.actions,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("policy can be serialized"),
            Err(_) => "invalid".into(),
        };
        let mut observed = self.observed_policy.lock();
        if observed.0.as_ref() != Some(&key) {
            let changed = observed.0.is_some();
            observed.0 = Some(key);
            observed.1 += 1;
            if changed {
                self.work.cancel_jobs();
                self.results.clear();
            }
        }
        observed.1
    }

    #[cfg(test)]
    pub fn current_ctx(&self) -> Option<AwsContext> {
        self.connection.lock().active.clone()
    }

    /// Detect credential-setting changes before a request or pending result.
    /// No locks survive an await. Provider fields are checked separately against
    /// the context's exact parsed configuration snapshot.
    pub(crate) fn observe_credential_settings(&self, path: &str, sso_constraint: &str) -> u64 {
        let mut state = self.connection.lock();
        if state.settings_failed
            || state.config_path.as_deref() != Some(path)
            || state.sso_constraint != sso_constraint
        {
            state.settings_failed = false;
            let was_configured = state.config_path.is_some();
            state.config_path = Some(path.to_string());
            state.sso_constraint = sso_constraint.to_string();
            state.settings_revision += 1;
            if was_configured {
                self.work.cancel_all();
                self.results.clear();
                state.attempt += 1;
                state.active = None;
                state.overrides.clear();
                state.pending_contexts.clear();
                state.set_account_at = None;
                state.status = "disconnected";
                if let Some(last) = state.last_attempt.as_mut() {
                    last["error"] = json!("AWS configuration changed; select the account again");
                    last["error_type"] = json!("ConfigurationChanged");
                    last["needs_sso_login"] = json!(false);
                }
            }
        }
        state.settings_revision
    }

    /// A broken app-settings file cannot leave a formerly verified context usable.
    pub(crate) fn invalidate_settings(&self) {
        self.work.cancel_all();
        self.results.clear();
        let mut state = self.connection.lock();
        if !state.settings_failed {
            state.settings_failed = true;
            state.settings_revision += 1;
            state.attempt += 1;
        }
        state.active = None;
        state.overrides.clear();
        state.pending_contexts.clear();
        state.set_account_at = None;
        state.status = "failed";
    }

    pub(crate) fn begin_attempt(&self, details: Value, settings_revision: u64) -> Option<u64> {
        let mut state = self.connection.lock();
        if state.settings_revision != settings_revision {
            return None;
        }
        state.attempt += 1;
        state.status = "verifying";
        state.active = None;
        state.set_account_at = None;
        state.last_attempt = Some(details);
        Some(state.attempt)
    }

    pub(crate) fn invalidate_context(&self, id: u64, error_type: &str, message: &str) {
        self.results.clear();
        let mut state = self.connection.lock();
        state
            .pending_contexts
            .retain(|_, pending| pending.context.id() != id);
        state
            .overrides
            .retain(|_, cached| cached.context.id() != id);
        if state
            .active
            .as_ref()
            .is_some_and(|context| context.id() == id)
        {
            state.active = None;
            state.set_account_at = None;
            state.status = "failed";
            if let Some(last) = state.last_attempt.as_mut() {
                last["error"] = json!(message);
                last["error_type"] = json!(error_type);
                last["needs_sso_login"] = json!(
                    matches!(
                        error_type,
                        "CredentialsError" | "CredentialsExpired" | "IdentityVerificationFailed"
                    ) && crate::aws::sso_login_required(message)
                );
            }
        }
    }
}
