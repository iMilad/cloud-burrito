//! Cloud Burrito — pure-Rust Tauri application.
//!
//! All AWS work happens in-process via the AWS SDK for Rust (`aws-sdk-*` +
//! `aws-config`). There is no sidecar process and no JSON-RPC bridge: the
//! frontend's `invoke(...)` calls land directly on the `#[tauri::command]`
//! handlers in `commands`, which talk to AWS and the local persistence files.
//!
//! Read-only by construction: only registered read-style operations are ever
//! compiled in, so no mutating call can be issued regardless of credential scope.

mod audit;
mod aws;
mod commands;
mod dashboard;
mod paths;
mod settings;
mod state;
mod widgets;

use state::AppState;

/// Serializes tests that mutate the process-global HOME env var.
#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // tauri-plugin-log: defaults route to stdout AND the platform log dir
        // (~/Library/Logs/<bundle-id>/ on macOS).
        .plugin(tauri_plugin_log::Builder::new().build())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::aws_set_account,
            commands::aws_list_profiles,
            commands::aws_list_pipelines,
            commands::aws_auth_status,
            commands::widget_fetch,
            commands::widget_get_source,
            commands::settings_get,
            commands::settings_set,
            commands::dashboard_get,
            commands::dashboard_set,
            commands::audit_tail,
            commands::policy_get,
            commands::policy_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
