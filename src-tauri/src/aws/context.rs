//! Explicit SSO contexts with immutable, STS-verified credential sessions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use aws_config::SdkConfig;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::Credentials;
use futures::FutureExt;
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
enum VerificationResult {
    Complete(Result<Arc<VerifiedSession>, ContextError>),
    Panicked,
}

type VerificationReceiver = tokio::sync::watch::Receiver<Option<VerificationResult>>;

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
    verification_flight: Arc<Mutex<Option<VerificationReceiver>>>,
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
            verification_flight: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_settings_revision(mut self, revision: u64) -> Self {
        self.settings_revision = revision;
        self.id = NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed);
        self.state = Arc::new(Mutex::new(SessionState::default()));
        self.verification = Arc::new(tokio::sync::Mutex::new(()));
        self.verification_flight = Arc::new(Mutex::new(None));
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
        self.runtime.aws.snapshot_sso(self).map_err(|_| {
            self.invalidate(
                "UnsupportedProfile",
                "Selected profile is unsupported or its SSO configuration is invalid",
            )
        })
    }

    /// Callers authorize SSO and STS before entering this method. One owned,
    /// bounded worker verifies a context; cancelling a subscriber cannot abort
    /// and restart another subscriber's credential/identity work.
    pub async fn verified_session(&self) -> Result<Arc<VerifiedSession>, ContextError> {
        let mut receiver = {
            let mut flight = self.verification_flight.lock();
            if let Some(current) = flight
                .as_ref()
                .filter(|current| current.borrow().is_none() && current.has_changed().is_ok())
            {
                current.clone()
            } else {
                let (sender, receiver) = tokio::sync::watch::channel(None);
                *flight = Some(receiver.clone());
                let context = self.clone();
                tokio::spawn(async move {
                    let result = std::panic::AssertUnwindSafe(context.verified_session_once())
                        .catch_unwind()
                        .await;
                    let result = match result {
                        Ok(result) => VerificationResult::Complete(result),
                        Err(_) => VerificationResult::Panicked,
                    };
                    let _ = sender.send_replace(Some(result));
                });
                receiver
            }
        };
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return match result {
                    VerificationResult::Complete(result) => {
                        let session = result?;
                        self.ensure_current(&session)?;
                        Ok(session)
                    }
                    VerificationResult::Panicked => {
                        #[cfg(test)]
                        panic!("unexpected live AWS/provider or personal configuration access in shared identity verification");
                        #[cfg(not(test))]
                        Err(ContextError::new(
                            "VerificationInterrupted",
                            "Identity verification was interrupted; reconnect the selected account",
                        ))
                    }
                };
            }
            receiver.changed().await.map_err(|_| {
                ContextError::new(
                    "VerificationInterrupted",
                    "Identity verification was interrupted; reconnect the selected account",
                )
            })?;
        }
    }

    async fn verification_policy(
        &self,
        service: &str,
        operation: &str,
    ) -> Result<(), ContextError> {
        let paths = self.runtime.paths.clone();
        let policy = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::task::spawn_blocking(move || crate::runtime::read_current_policy(&paths, false)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| Err("policy unavailable".into()));
        crate::aws::policy::gate(&policy, service, operation).map_err(|_| {
            ContextError::new(
                "PolicyDenied",
                "Identity verification is blocked by the current local policy",
            )
        })
    }

    fn verification_active(&self) -> Result<(), ContextError> {
        if self.state.lock().invalidated {
            return Err(ContextError::new(
                "StaleContext",
                "this AWS context changed before identity dispatch",
            ));
        }
        Ok(())
    }

    async fn verified_session_once(&self) -> Result<Arc<VerifiedSession>, ContextError> {
        let verification_scope = crate::scheduler::WorkScope::new(
            format!("verification-{}", self.id),
            self.account_id.clone(),
            self.region.clone(),
            crate::process::ProcessCancellation::new(),
            crate::scheduler::WorkBudget::for_widget("identity"),
        );
        let _verification =
            tokio::time::timeout_at(verification_scope.deadline, self.verification.lock())
                .await
                .map_err(|_| {
                    ContextError::new(
                        "VerificationDeadline",
                        "Identity verification reached its time limit; try connecting again",
                    )
                })?;
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
            .scheduler
            .run(
                &verification_scope,
                "sso",
                crate::scheduler::ResourceKind::Ordinary,
                async {
                    self.verification_policy("sso", "GetRoleCredentials")
                        .await?;
                    self.verification_active()?;
                    self.runtime.aws.resolve_sso(&snapshot).await.map_err(|_| {
                        ContextError::new(
                            "CredentialsError",
                            "SSO credentials could not be loaded or refreshed; sign in again",
                        )
                    })
                },
            )
            .await
            .map_err(|_| {
                ContextError::new(
                    "VerificationDeadline",
                    "Credential verification reached its local work limit; try connecting again",
                )
            })??;
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
        let caller=self.runtime.scheduler.run(&verification_scope,"sts",crate::scheduler::ResourceKind::Ordinary,async {
            self.verification_policy("sts","GetCallerIdentity").await?;
            self.verification_active()?;
            self.runtime.aws.caller_identity(&sdk).await.map_err(|_|
                ContextError::new("IdentityVerificationFailed","AWS identity verification failed; check the selected SSO session and try again"))
        }).await.map_err(|_|ContextError::new("VerificationDeadline","Identity verification reached its local work limit; try connecting again"))??;
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

#[cfg(test)]
mod shared_verification_tests {
    use super::*;
    use crate::{runtime::AwsBackend, test_support::TestDir};
    use futures::future::BoxFuture;
    use serde_json::{json, Value};
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, UNIX_EPOCH};

    #[derive(Default)]
    struct SharedBackend {
        resolves: AtomicUsize,
        callers: AtomicUsize,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    impl AwsBackend for SharedBackend {
        fn inspect_config(&self, _: &str, _: Option<&str>) -> Value {
            json!({"profiles":[]})
        }
        fn snapshot_sso(&self, ctx: &AwsContext) -> Result<SsoProfileSnapshot, String> {
            Ok(SsoProfileSnapshot {
                config_path: ctx.aws_config_path.clone().into(),
                profile: ctx.profile.clone(),
                account_id: ctx.account_id.clone(),
                role_name: "SyntheticReadOnly".into(),
                region: ctx.region.clone(),
                sso_region: ctx.region.clone(),
                start_url: "https://synthetic.example.invalid/start".into(),
                session_name: None,
                registration_scopes: None,
                settings_revision: ctx.settings_revision,
            })
        }
        fn resolve_sso<'a>(
            &'a self,
            _: &'a SsoProfileSnapshot,
        ) -> BoxFuture<'a, Result<Credentials, String>> {
            Box::pin(async move {
                self.resolves.fetch_add(1, Ordering::SeqCst);
                self.entered.notify_one();
                self.release.notified().await;
                Ok(Credentials::new(
                    "CB_SYNTHETIC_KEY",
                    "CB_SYNTHETIC_SECRET",
                    Some("CB_SYNTHETIC_TOKEN".into()),
                    Some(UNIX_EPOCH + Duration::from_secs(1_800_000_000)),
                    "shared-verification-test",
                ))
            })
        }
        fn caller_identity<'a>(
            &'a self,
            _: &'a SdkConfig,
        ) -> BoxFuture<'a, Result<CallerIdentity, String>> {
            Box::pin(async move {
                self.callers.fetch_add(1, Ordering::SeqCst);
                Ok(CallerIdentity::builder()
                    .account("acct-shared-fixture")
                    .arn("synthetic:principal:shared")
                    .user_id("synthetic-shared-user")
                    .build())
            })
        }
    }
    fn fixture() -> (TestDir, Arc<SharedBackend>, AwsContext) {
        let dir = TestDir::new();
        crate::aws::policy::load(&dir.paths()).unwrap();
        let backend = Arc::new(SharedBackend::default());
        let mut runtime = Runtime::for_test(dir.paths());
        runtime.aws = backend.clone();
        let context = AwsContext::new(
            "synthetic-profile".into(),
            "acct-shared-fixture".into(),
            "us-east-1".into(),
            None,
            "synthetic-config.ini".into(),
            runtime,
        );
        (dir, backend, context)
    }
    #[tokio::test]
    async fn cancelled_subscriber_does_not_restart_shared_cold_verification() {
        let (_dir, backend, context) = fixture();
        let first = tokio::spawn({
            let context = context.clone();
            async move { context.verified_session().await }
        });
        backend.entered.notified().await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        // A new subscriber arriving after cancellation still joins the owned
        // bounded worker, including its captured in-flight credential work.
        let second = tokio::spawn({
            let context = context.clone();
            async move { context.verified_session().await }
        });
        tokio::task::yield_now().await;
        backend.release.notify_one();
        let session = second.await.unwrap().unwrap();
        assert_eq!(session.identity.account_id, "acct-shared-fixture");
        assert_eq!(backend.resolves.load(Ordering::SeqCst), 1);
        assert_eq!(backend.callers.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(
            &session,
            &context.verified_session().await.unwrap()
        ));
        assert_eq!(backend.resolves.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn context_invalidation_during_owned_verification_cannot_publish_a_session() {
        let (_dir, backend, context) = fixture();
        let request = tokio::spawn({
            let context = context.clone();
            async move { context.verified_session().await }
        });
        backend.entered.notified().await;
        context.invalidate("SyntheticChange", "synthetic context changed");
        backend.release.notify_one();
        let failure = request.await.unwrap().unwrap_err();
        assert_eq!(failure.error_type, "StaleContext");
        assert_eq!(backend.callers.load(Ordering::SeqCst), 0);
        assert!(context.state.lock().session.is_none());
        assert_eq!(backend.resolves.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn invalidation_while_queued_prevents_credential_provider_dispatch() {
        let (_dir, backend, context) = fixture();
        let scope = crate::scheduler::WorkScope::new(
            "synthetic-capacity-owner".into(),
            context.account_id.clone(),
            context.region.clone(),
            crate::process::ProcessCancellation::new(),
            crate::scheduler::WorkBudget::for_widget("identity"),
        );
        let mut permits = Vec::new();
        for _ in 0..4 {
            permits.push(
                context
                    .runtime
                    .scheduler
                    .acquire(&scope, "sso", crate::scheduler::ResourceKind::Ordinary)
                    .await
                    .unwrap(),
            );
        }
        let request = tokio::spawn({
            let context = context.clone();
            async move { context.verified_session().await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while context.runtime.scheduler.snapshot().queued == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        context.invalidate("SyntheticChange", "synthetic context changed");
        drop(permits);
        let failure = request.await.unwrap().unwrap_err();
        assert_eq!(failure.error_type, "StaleContext");
        assert_eq!(backend.resolves.load(Ordering::SeqCst), 0);
        assert_eq!(backend.callers.load(Ordering::SeqCst), 0);
        assert!(context.state.lock().session.is_none());
    }

    #[tokio::test]
    async fn policy_revocation_after_credentials_blocks_identity_dispatch() {
        let (dir, backend, context) = fixture();
        let request = tokio::spawn({
            let context = context.clone();
            async move { context.verified_session().await }
        });
        backend.entered.notified().await;
        crate::aws::policy::write_text(
            &dir.paths(),
            "statements:\n - effect: Deny\n   action: ['sts:GetCallerIdentity']\n",
        )
        .unwrap();
        backend.release.notify_one();
        let failure = request.await.unwrap().unwrap_err();
        assert_eq!(failure.error_type, "PolicyDenied");
        assert_eq!(backend.callers.load(Ordering::SeqCst), 0);
        assert!(context.state.lock().session.is_none());
    }
}
