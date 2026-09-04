//! Cloud Burrito — pure-Rust Tauri application.
//!
//! All AWS work happens in-process via the AWS SDK for Rust (`aws-sdk-*` +
//! `aws-config`). There is no sidecar process and no JSON-RPC bridge: the
//! frontend's `invoke(...)` calls land directly on the `#[tauri::command]`
//! handlers in `commands`, which talk to AWS and the local persistence files.
//!
//! A closed operation registry permits reviewed resource reads and separately
//! controlled Logs Insights start/stop capabilities. Query execution can incur scan costs.

mod audit;
mod audit_reader;
mod audit_writer;
mod aws;
mod commands;
mod dashboard;
mod paths;
mod process;
mod request;
mod result_cache;
mod runtime;
mod scheduler;
mod settings;
mod state;
mod storage;
mod validation;
mod widgets;
mod work_registry;

use state::AppState;

#[cfg(test)]
mod benchmarks;
#[cfg(test)]
mod benchmarks_cli;
#[cfg(test)]
mod test_aws;
#[cfg(test)]
mod test_support;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = match paths::AppPaths::native() {
        Ok(paths) => paths,
        Err(message) => {
            // A fixed message avoids leaking local paths. Refuse before Tauri,
            // audit storage, or any other startup work can use a relative home.
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        // Only deliberate application diagnostics reach stdout/platform logs.
        // SDK/dependency transport logs are not a reviewed redacted surface.
        .plugin(
            tauri_plugin_log::Builder::new()
                .filter(|metadata| metadata.target().starts_with("cloud_burrito"))
                .build(),
        )
        .manage(AppState::with_runtime(runtime::Runtime::native(paths)))
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::aws_set_account,
            commands::aws_list_profiles,
            commands::aws_list_pipelines,
            commands::aws_auth_status,
            commands::cli_availability,
            commands::widget_fetch,
            commands::request_cancel,
            commands::widget_get_source,
            commands::settings_get,
            commands::settings_set,
            commands::dashboard_get,
            commands::dashboard_set,
            commands::audit_tail,
            commands::audit_history,
            commands::policy_get,
            commands::policy_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
