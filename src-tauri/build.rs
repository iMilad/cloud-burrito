fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "ping",
            "aws_set_account",
            "aws_list_profiles",
            "aws_list_pipelines",
            "aws_auth_status",
            "widget_fetch",
            "widget_get_source",
            "settings_get",
            "settings_set",
            "dashboard_get",
            "dashboard_set",
            "audit_tail",
            "policy_get",
            "policy_set",
        ]),
    ))
    .expect("failed to generate Tauri application manifest");
}
