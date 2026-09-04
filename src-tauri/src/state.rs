//! Atomic connection state. Only the latest attempt may publish an outcome.

use std::collections::HashMap;

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
}

pub(crate) struct ConnectionState {
    pub attempt: u64,
    pub settings_revision: u64,
    config_path: Option<String>,
    settings_failed: bool,
    pub status: &'static str,
    pub active: Option<AwsContext>,
    pub overrides: HashMap<PinnedKey, CachedContext>,
    pub last_attempt: Option<Value>,
    pub set_account_at: Option<f64>,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            attempt: 0,
            settings_revision: 0,
            config_path: None,
            settings_failed: false,
            status: "disconnected",
            active: None,
            overrides: HashMap::new(),
            last_attempt: None,
            set_account_at: None,
        }
    }
}

#[derive(Default)]
pub struct AppState {
    pub runtime: Runtime,
    pub(crate) connection: Mutex<ConnectionState>,
}

impl AppState {
    #[cfg(test)]
    pub fn with_runtime(runtime: Runtime) -> Self {
        Self {
            runtime,
            connection: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub fn current_ctx(&self) -> Option<AwsContext> {
        self.connection.lock().active.clone()
    }

    /// Detect persisted path changes before a new request or a pending result.
    /// No locks survive an await. Provider fields are checked separately against
    /// the context's exact parsed configuration snapshot.
    pub(crate) fn observe_config_path(&self, path: &str) -> u64 {
        let mut state = self.connection.lock();
        if state.settings_failed || state.config_path.as_deref() != Some(path) {
            state.settings_failed = false;
            let was_configured = state.config_path.is_some();
            state.config_path = Some(path.to_string());
            state.settings_revision += 1;
            if was_configured {
                state.attempt += 1;
                state.active = None;
                state.overrides.clear();
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
        let mut state = self.connection.lock();
        if !state.settings_failed {
            state.settings_failed = true;
            state.settings_revision += 1;
            state.attempt += 1;
        }
        state.active = None;
        state.overrides.clear();
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
        let mut state = self.connection.lock();
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
