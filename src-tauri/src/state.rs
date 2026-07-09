//! Process-lifetime application state, managed by Tauri and shared across
//! commands. Replaces the Python sidecar's module-level `_state` dicts.

use std::collections::HashMap;

use parking_lot::{Mutex, RwLock};
use serde_json::Value;

use crate::aws::AwsContext;

#[derive(Default)]
pub struct AppState {
    /// The active topbar context. `None` until a successful set-account.
    pub active: RwLock<Option<AwsContext>>,
    /// Pinned per-tile contexts, memoised by (profile, account_id, region) so
    /// repeat fetches don't re-resolve SSO every time.
    pub overrides: Mutex<HashMap<(String, String, String), AwsContext>>,
    /// The most recent set-account attempt, so the Identity panel can show what
    /// was tried even when it failed and no context was set.
    pub last_attempt: Mutex<Option<Value>>,
    /// Epoch seconds of the last *successful* set-account.
    pub set_account_at: Mutex<Option<f64>>,
}

impl AppState {
    /// Clone out the active context (cheap — `AwsContext` is `Arc`-backed) so
    /// callers can `await` on it without holding the lock.
    pub fn current_ctx(&self) -> Option<AwsContext> {
        self.active.read().clone()
    }
}
