//! Read-only AWS context: a handle to one (profile, account, region) session.
//!
//! `aws-config` resolves the SSO profile from `~/.aws/config` the standard way
//! — cached SSO token → short-lived role credentials that the SDK auto-refreshes.
//! The read-only guarantee is structural: only read operations are ever issued
//! (originally ported from the Python sidecar's `awsctx.py`).

use std::sync::Arc;

use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_credential_types::provider::ProvideCredentials;
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct AwsContext {
    pub profile: String,
    pub account_id: String,
    pub region: String,
    pub sso_session_name: Option<String>,
    pub aws_config_path: String,
    // Built once on first use, then shared. `Arc` so cloning the context (for
    // the per-tile override cache) shares the same resolved config.
    config: Arc<OnceCell<SdkConfig>>,
}

impl AwsContext {
    pub fn new(
        profile: String,
        account_id: String,
        region: String,
        sso_session_name: Option<String>,
        aws_config_path: String,
    ) -> Self {
        Self {
            profile,
            account_id,
            region,
            sso_session_name,
            aws_config_path,
            config: Arc::new(OnceCell::new()),
        }
    }

    /// Build (once) and return the SdkConfig. `aws-config` reads the profile
    /// from `~/.aws/config` (honoring the `AWS_CONFIG_FILE` env var that the
    /// set-account command sets for custom paths) and wires the SSO provider.
    pub async fn sdk_config(&self) -> &SdkConfig {
        self.config
            .get_or_init(|| async {
                // Apply a custom config-file location (read by aws-config) so
                // per-tile override contexts honor it too, not just the active
                // topbar context.
                if !self.aws_config_path.is_empty() && self.aws_config_path != "~/.aws/config" {
                    std::env::set_var(
                        "AWS_CONFIG_FILE",
                        super::config_file::expand(&self.aws_config_path),
                    );
                }
                aws_config::defaults(BehaviorVersion::latest())
                    .profile_name(&self.profile)
                    .region(Region::new(self.region.clone()))
                    .load()
                    .await
            })
            .await
    }

    /// Force credential resolution so an expired SSO token surfaces immediately.
    /// No sts:GetCallerIdentity round-trip — resolving SSO creds is proof enough
    /// (analogous to the Python sidecar's `get_frozen_credentials()` probe).
    pub async fn resolve_credentials(&self) -> Result<(), String> {
        let cfg = self.sdk_config().await;
        let provider = cfg
            .credentials_provider()
            .ok_or_else(|| "no credentials provider for this profile".to_string())?;
        provider
            .provide_credentials()
            .await
            .map(|_| ())
            .map_err(|e| format!("{e}"))
    }
}

/// Heuristic: does this error string indicate the user must re-run `aws sso login`?
/// (Analogous to `sso_login_required` in the Python sidecar.)
pub fn sso_login_required(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("sso")
        || e.contains("token")
        || e.contains("expired")
        || e.contains("unauthorized")
        || e.contains("forbidden")
        || e.contains("refresh the sso")
}
