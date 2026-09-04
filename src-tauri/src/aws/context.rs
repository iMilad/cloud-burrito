//! Explicit SSO contexts with immutable, STS-verified credential sessions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use aws_config::SdkConfig;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::Credentials;
use parking_lot::Mutex;

use super::config_file::SsoProfileSnapshot;
use crate::runtime::{fixed_sdk_config, unexpired, CallerIdentity, Runtime};

static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PROVIDER_REVISION: AtomicU64 = AtomicU64::new(1);
const REFRESH_MARGIN_SECONDS: f64 = 60.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextError {
    pub error_type: &'static str,
    pub message: String,
}

impl ContextError {
    pub fn new(error_type: &'static str, message: impl Into<String>) -> Self {
        Self {
            error_type,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContextError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VerifiedIdentity {
    pub account_id: String,
    pub arn: String,
    pub user_id: String,
}

impl VerifiedIdentity {
    fn from_output(output: CallerIdentity) -> Result<Self, ContextError> {
        let field = |value: Option<&str>| {
            value
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    ContextError::new(
                        "IdentityVerificationFailed",
                        "STS returned an incomplete caller identity",
                    )
                })
        };
        Ok(Self {
            account_id: field(output.account())?,
            arn: field(output.arn())?,
            user_id: field(output.user_id())?,
        })
    }
}

/// Config and identity verified with exactly one captured credential set. The
/// provider cannot refresh itself or discover credentials from another source.
pub struct VerifiedSession {
    pub sdk: SdkConfig,
    pub identity: VerifiedIdentity,
    pub snapshot: SsoProfileSnapshot,
    pub provider_revision: u64,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for VerifiedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedSession")
            .field("provider_revision", &self.provider_revision)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl VerifiedSession {
    /// Export only the fixed credential set already used for STS verification.
    /// This provider cannot refresh, discover another source or run a process.
    pub(crate) async fn cli_credentials(&self) -> Result<Credentials, ContextError> {
        let unavailable = || {
            ContextError::new(
                "CliCredentialsUnavailable",
                "Verified temporary credentials are unavailable; reconnect the account",
            )
        };
        let credentials = self
            .sdk
            .credentials_provider()
            .ok_or_else(unavailable)?
            .provide_credentials()
            .await
            .map_err(|_| unavailable())?;
        if credentials.access_key_id().trim().is_empty()
            || credentials.secret_access_key().trim().is_empty()
            || credentials
                .session_token()
                .is_none_or(|token| token.trim().is_empty())
            || credentials.expiry() != Some(self.expires_at)
        {
            return Err(unavailable());
        }
        Ok(credentials)
    }
}

#[derive(Default)]
struct SessionState {
    snapshot: Option<SsoProfileSnapshot>,
    principal: Option<VerifiedIdentity>,
    session: Option<Arc<VerifiedSession>>,
    invalidated: bool,
}

#[derive(Clone)]
pub struct AwsContext {
    pub profile: String,
    pub account_id: String,
    pub region: String,
    pub sso_session_name: Option<String>,
    pub aws_config_path: String,
    pub settings_revision: u64,
    runtime: Runtime,
    id: u64,
    state: Arc<Mutex<SessionState>>,
    verification: Arc<tokio::sync::Mutex<()>>,
}

impl AwsContext {
    pub fn new(
        profile: String,
        account_id: String,
        region: String,
        sso_session_name: Option<String>,
        aws_config_path: String,
        runtime: Runtime,
    ) -> Self {
        Self {
            profile,
            account_id,
            region,
            sso_session_name,
            aws_config_path,
            settings_revision: 0,
            runtime,
            id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(Mutex::new(SessionState::default())),
            verification: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn with_settings_revision(mut self, revision: u64) -> Self {
        self.settings_revision = revision;
        self.id = NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed);
        self.state = Arc::new(Mutex::new(SessionState::default()));
        self.verification = Arc::new(tokio::sync::Mutex::new(()));
        self
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    fn invalidate(&self, error_type: &'static str, message: impl Into<String>) -> ContextError {
        let mut state = self.state.lock();
        state.invalidated = true;
        state.session = None;
        ContextError::new(error_type, message)
    }

    fn fresh_snapshot(&self) -> Result<SsoProfileSnapshot, ContextError> {
        self.runtime
            .aws
            .snapshot_sso(self)
            .map_err(|error| self.invalidate("UnsupportedProfile", error))
    }

    /// Callers authorize SSO and STS before entering this method. Refresh is
    /// serialized per context; resource clients never refresh credentials.
    pub async fn verified_session(&self) -> Result<Arc<VerifiedSession>, ContextError> {
        let _verification = self.verification.lock().await;
        let snapshot = self.fresh_snapshot()?;
        {
            let mut state = self.state.lock();
            if state.invalidated {
                return Err(ContextError::new(
                    "StaleContext",
                    "this AWS context changed; select the account again",
                ));
            }
            if state.snapshot.as_ref().is_some_and(|old| old != &snapshot) {
                state.invalidated = true;
                state.session = None;
                return Err(ContextError::new(
                    "ConfigurationChanged",
                    "selected SSO configuration changed; select the account again",
                ));
            }
            if let Some(session) = &state.session {
                if unexpired(
                    session.expires_at,
                    self.runtime.clock.now_epoch(),
                    REFRESH_MARGIN_SECONDS,
                ) {
                    return Ok(session.clone());
                }
            }
            state.snapshot = Some(snapshot.clone());
            state.session = None;
        }

        let credentials = self
            .runtime
            .aws
            .resolve_sso(&snapshot)
            .await
            .map_err(|error| ContextError::new("CredentialsError", error))?;
        let expires_at = credentials.expiry().ok_or_else(|| {
            ContextError::new("CredentialsError", "SSO credentials must have an expiry")
        })?;
        if !unexpired(
            expires_at,
            self.runtime.clock.now_epoch(),
            REFRESH_MARGIN_SECONDS,
        ) {
            return Err(ContextError::new(
                "CredentialsExpired",
                "SSO credentials expired or are too close to expiry; sign in again",
            ));
        }
        let sdk = fixed_sdk_config(credentials, &snapshot.region, self.runtime.clock.clone());
        let caller = self
            .runtime
            .aws
            .caller_identity(&sdk)
            .await
            .map_err(|error| ContextError::new("IdentityVerificationFailed", error))?;
        let identity = VerifiedIdentity::from_output(caller)?;
        if identity.account_id != snapshot.account_id || identity.account_id != self.account_id {
            return Err(self.invalidate(
                "IdentityMismatch",
                "verified AWS account does not match the selected account",
            ));
        }
        if self.fresh_snapshot()? != snapshot {
            return Err(self.invalidate(
                "ConfigurationChanged",
                "selected SSO configuration changed during verification",
            ));
        }
        if !unexpired(expires_at, self.runtime.clock.now_epoch(), 0.0) {
            return Err(ContextError::new(
                "CredentialsExpired",
                "SSO credentials expired during identity verification",
            ));
        }
        let mut state = self.state.lock();
        if state.invalidated {
            return Err(ContextError::new(
                "StaleContext",
                "this AWS context was invalidated during verification",
            ));
        }
        if state.principal.as_ref().is_some_and(|old| old != &identity) {
            state.invalidated = true;
            return Err(ContextError::new(
                "IdentityChanged",
                "verified AWS principal changed; select the account again",
            ));
        }
        let session = Arc::new(VerifiedSession {
            sdk,
            identity: identity.clone(),
            snapshot,
            provider_revision: NEXT_PROVIDER_REVISION.fetch_add(1, Ordering::Relaxed),
            expires_at,
        });
        state.principal = Some(identity);
        state.session = Some(session.clone());
        Ok(session)
    }

    /// Fence an awaited result against file/config changes, expiry and a newer
    /// verified credential snapshot. Root also checks its selection generation.
    pub fn ensure_current(&self, session: &Arc<VerifiedSession>) -> Result<(), ContextError> {
        if self.fresh_snapshot()? != session.snapshot {
            return Err(self.invalidate(
                "ConfigurationChanged",
                "selected SSO configuration changed while work was running",
            ));
        }
        let state = self.state.lock();
        if state.invalidated
            || state
                .session
                .as_ref()
                .is_none_or(|current| current.provider_revision != session.provider_revision)
        {
            return Err(ContextError::new(
                "StaleContext",
                "a newer AWS credential session replaced this result",
            ));
        }
        if !unexpired(session.expires_at, self.runtime.clock.now_epoch(), 0.0) {
            return Err(ContextError::new(
                "CredentialsExpired",
                "AWS credentials expired while work was running",
            ));
        }
        Ok(())
    }
}

/// Heuristic used only to suggest the existing external SSO login workflow.
pub fn sso_login_required(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("sso")
        || e.contains("token")
        || e.contains("expired")
        || e.contains("unauthorized")
        || e.contains("forbidden")
        || e.contains("refresh the sso")
}
