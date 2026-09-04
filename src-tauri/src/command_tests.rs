//! Synthetic command integration fixtures. No real AWS configuration, provider,
//! token cache, subprocess, HTTP connector or process environment is used.

use super::*;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use aws_config::SdkConfig;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::Credentials;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::aws::config_file::SsoProfileSnapshot;
use crate::process::{CliRequest, ProcessOutput, ProcessRunner};
use crate::runtime::{AwsBackend, CallerIdentity, Clock, Runtime};
use crate::test_support::TestDir;

const ACCOUNT_A: &str = "acct-a-fixture";
const ACCOUNT_B: &str = "acct-b-fixture";

type Responses<T> = Mutex<HashMap<String, VecDeque<oneshot::Receiver<Result<T, String>>>>>;

#[derive(Default)]
struct ScriptedAws {
    snapshots: Mutex<HashMap<String, Result<SsoProfileSnapshot, String>>>,
    credentials: Responses<Credentials>,
    identities: Responses<CallerIdentity>,
    snapshot_calls: AtomicUsize,
    credential_calls: AtomicUsize,
    identity_calls: AtomicUsize,
    resolved_snapshots: Mutex<Vec<SsoProfileSnapshot>>,
    identity_keys: Mutex<Vec<String>>,
}

impl ScriptedAws {
    fn profile(&self, profile: &str, account: &str) {
        self.snapshots.lock().insert(
            profile.into(),
            Ok(SsoProfileSnapshot {
                config_path: PathBuf::from("synthetic-config.ini"),
                profile: profile.into(),
                account_id: account.into(),
                role_name: "SyntheticReadOnly".into(),
                region: "us-east-1".into(),
                sso_region: "eu-west-1".into(),
                start_url: "https://example.invalid/start".into(),
                session_name: Some("synthetic-session".into()),
                registration_scopes: None,
                settings_revision: 0,
            }),
        );
    }

    fn queue_credentials(&self, profile: &str) -> oneshot::Sender<Result<Credentials, String>> {
        let (tx, rx) = oneshot::channel();
        self.credentials
            .lock()
            .entry(profile.into())
            .or_default()
            .push_back(rx);
        tx
    }

    fn queue_identity(&self, key: &str) -> oneshot::Sender<Result<CallerIdentity, String>> {
        let (tx, rx) = oneshot::channel();
        self.identities
            .lock()
            .entry(key.into())
            .or_default()
            .push_back(rx);
        tx
    }

    fn ready_identity(&self, key: &str, account: &str, principal: &str) {
        self.queue_identity(key)
            .send(Ok(identity(account, principal)))
            .unwrap();
    }

    fn ready(&self, profile: &str, key: &str, account: &str, principal: &str, expiry: u64) {
        self.ready_identity(key, account, principal);
        self.queue_credentials(profile)
            .send(Ok(credentials(key, expiry)))
            .unwrap();
    }

    fn assert_no_resolution(&self) {
        assert_eq!(self.credential_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.identity_calls.load(Ordering::SeqCst), 0);
        assert!(self.identity_keys.lock().is_empty());
    }
}

impl AwsBackend for ScriptedAws {
    fn inspect_config(&self, _: &str) -> Value {
        panic!("unexpected native-style profile inspection");
    }

    fn snapshot_sso(&self, ctx: &AwsContext) -> Result<SsoProfileSnapshot, String> {
        self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
        let mut snapshot = self
            .snapshots
            .lock()
            .get(&ctx.profile)
            .cloned()
            .ok_or_else(|| "synthetic profile is unsupported".to_string())??;
        if snapshot.account_id != ctx.account_id {
            return Err("selected profile account does not match the requested account".into());
        }
        if ctx
            .sso_session_name
            .as_ref()
            .is_some_and(|session| Some(session) != snapshot.session_name.as_ref())
        {
            return Err("configured SSO session conflicts with the selected profile".into());
        }
        snapshot.config_path = PathBuf::from(&ctx.aws_config_path);
        snapshot.settings_revision = ctx.settings_revision;
        snapshot.region = ctx.region.clone();
        Ok(snapshot)
    }

    fn resolve_sso<'a>(
        &'a self,
        snapshot: &'a SsoProfileSnapshot,
    ) -> BoxFuture<'a, Result<Credentials, String>> {
        Box::pin(async move {
            self.credential_calls.fetch_add(1, Ordering::SeqCst);
            self.resolved_snapshots.lock().push(snapshot.clone());
            let receiver = self
                .credentials
                .lock()
                .get_mut(&snapshot.profile)
                .and_then(VecDeque::pop_front)
                .expect("unexpected credential resolution: no scripted response");
            receiver.await.expect("scripted credential sender dropped")
        })
    }

    fn caller_identity<'a>(
        &'a self,
        config: &'a SdkConfig,
    ) -> BoxFuture<'a, Result<CallerIdentity, String>> {
        Box::pin(async move {
            self.identity_calls.fetch_add(1, Ordering::SeqCst);
            // Exercise the real fixed provider to prove STS verification uses
            // the captured credentials later exposed to resource clients.
            let credentials = config
                .credentials_provider()
                .expect("fixed provider missing")
                .provide_credentials()
                .await
                .expect("fixed test credentials rejected");
            let key = credentials.access_key_id().to_string();
            self.identity_keys.lock().push(key.clone());
            let receiver = self
                .identities
                .lock()
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .expect("unexpected identity request: no scripted response for credential key");
            receiver.await.expect("scripted identity sender dropped")
        })
    }
}

#[derive(Default)]
struct RejectProcess(AtomicUsize);

impl ProcessRunner for RejectProcess {
    fn run(&self, _: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("unexpected process execution in command test");
    }
}

struct MutableClock(AtomicU64);

impl Clock for MutableClock {
    fn now_epoch(&self) -> f64 {
        self.0.load(Ordering::SeqCst) as f64
    }
}

struct Fixture {
    _dir: TestDir,
    state: AppState,
    aws: Arc<ScriptedAws>,
    process: Arc<RejectProcess>,
    clock: Arc<MutableClock>,
}

impl Fixture {
    fn new() -> Self {
        let dir = TestDir::new();
        let aws = Arc::new(ScriptedAws::default());
        aws.profile("demo-a", ACCOUNT_A);
        aws.profile("demo-b", ACCOUNT_B);
        let process = Arc::new(RejectProcess::default());
        let clock = Arc::new(MutableClock(AtomicU64::new(1234)));
        let mut runtime = Runtime::for_test(dir.paths());
        runtime.aws = aws.clone();
        runtime.process = process.clone();
        runtime.clock = clock.clone();
        Self {
            _dir: dir,
            state: AppState::with_runtime(runtime),
            aws,
            process,
            clock,
        }
    }

    fn policy(&self) -> Result<aws::policy::Policy, String> {
        aws::policy::load(&self.state.runtime.paths).map_err(|error| error.message)
    }

    fn no_process(&self) {
        assert_eq!(self.process.0.load(Ordering::SeqCst), 0);
    }

    async fn connect_a(&self, key: &str, expiry: u64) {
        self.aws
            .ready("demo-a", key, ACCOUNT_A, "principal-a", expiry);
        let result = aws_set_account_impl(&self.state, demo("demo-a", ACCOUNT_A))
            .await
            .unwrap();
        assert_eq!(result["ok"], true, "{result}");
    }
}

fn credentials(key: &str, expiry: u64) -> Credentials {
    Credentials::new(
        key,
        "CB_SYNTHETIC_SECRET",
        Some("CB_SYNTHETIC_TOKEN".into()),
        Some(UNIX_EPOCH + Duration::from_secs(expiry)),
        "command-test-only",
    )
}

fn identity(account: &str, principal: &str) -> CallerIdentity {
    CallerIdentity::builder()
        .account(account)
        .arn(format!(
            "arn:aws:sts::{account}:assumed-role/SyntheticReadOnly/{principal}"
        ))
        .user_id(format!("SYNTHETIC_ROLE:{principal}"))
        .build()
}

fn demo(profile: &str, account: &str) -> Value {
    json!({"profile": profile, "account_id": account, "region": "us-east-1"})
}

fn pinned(profile: &str, account: &str) -> Value {
    json!({"context": {"mode": "pinned", "profile": profile, "account_id": account, "region": "us-east-1"}})
}

#[tokio::test]
async fn ping_and_unverified_auth_status_report_no_aws_work() {
    let fixture = Fixture::new();
    assert_eq!(ping().await.unwrap()["version"], env!("CARGO_PKG_VERSION"));
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], false);
    assert_eq!(auth["has_context"], false);
    assert_eq!(auth["connection_state"], "disconnected");
    assert!(resolve_widget_ctx(&fixture.state, &json!({})).is_err());
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn matched_identity_uses_the_exact_resource_sdk_credentials_and_auth_cache() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
    let resolved =
        resolve_widget_ctx(&fixture.state, &json!({"context": {"mode": "inherit"}})).unwrap();
    let session = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    let resource_credentials = session
        .sdk
        .credentials_provider()
        .unwrap()
        .provide_credentials()
        .await
        .unwrap();
    assert_eq!(resource_credentials.access_key_id(), "CB_SYNTHETIC_A1");
    assert_eq!(
        fixture.aws.identity_keys.lock().as_slice(),
        ["CB_SYNTHETIC_A1"]
    );
    assert_eq!(session.identity.account_id, ACCOUNT_A);
    assert_eq!(resolved.context.profile, "demo-a");
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], true);
    assert_eq!(auth["caller_arn"], session.identity.arn);
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.state.connection.lock().set_account_at, Some(1234.0));
    fixture.no_process();
}

#[tokio::test]
async fn mismatched_sts_account_never_becomes_active() {
    let fixture = Fixture::new();
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_WRONG_ACCOUNT",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    let result = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A))
        .await
        .unwrap();
    assert_eq!(result["error_type"], "IdentityMismatch");
    assert!(fixture.state.current_ctx().is_none());
    assert_eq!(fixture.state.connection.lock().status, "failed");
    assert_eq!(
        aws_auth_status_impl(&fixture.state).await.unwrap()["logged_in"],
        false
    );
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    fixture.no_process();
}

#[tokio::test]
async fn newer_account_survives_older_credential_success_or_failure() {
    for old_succeeds in [true, false] {
        let fixture = Fixture::new();
        let finish_a = fixture.aws.queue_credentials("demo-a");
        let finish_b = fixture.aws.queue_credentials("demo-b");
        fixture
            .aws
            .ready_identity("CB_SYNTHETIC_A1", ACCOUNT_A, "principal-a");
        fixture
            .aws
            .ready_identity("CB_SYNTHETIC_B1", ACCOUNT_B, "principal-b");
        let a = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A));
        let b = aws_set_account_impl(&fixture.state, demo("demo-b", ACCOUNT_B));
        tokio::pin!(a, b);
        assert!(futures::poll!(&mut a).is_pending());
        assert!(futures::poll!(&mut b).is_pending());
        finish_b
            .send(Ok(credentials("CB_SYNTHETIC_B1", 3000)))
            .unwrap();
        assert_eq!(b.await.unwrap()["ok"], true);
        finish_a
            .send(if old_succeeds {
                Ok(credentials("CB_SYNTHETIC_A1", 3000))
            } else {
                Err("synthetic SSO failure".into())
            })
            .unwrap();
        assert_eq!(a.await.unwrap()["error_type"], "Superseded");
        assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_B);
        let connection = fixture.state.connection.lock();
        assert_eq!(connection.status, "verified");
        assert_eq!(
            connection.last_attempt.as_ref().unwrap()["profile"],
            "demo-b"
        );
        drop(connection);
        fixture.no_process();
    }
}

#[tokio::test]
async fn newer_failure_is_not_replaced_by_older_success() {
    let fixture = Fixture::new();
    let finish_a = fixture.aws.queue_credentials("demo-a");
    let finish_b = fixture.aws.queue_credentials("demo-b");
    fixture
        .aws
        .ready_identity("CB_SYNTHETIC_A1", ACCOUNT_A, "principal-a");
    let a = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A));
    let b = aws_set_account_impl(&fixture.state, demo("demo-b", ACCOUNT_B));
    tokio::pin!(a, b);
    assert!(futures::poll!(&mut a).is_pending());
    assert!(futures::poll!(&mut b).is_pending());
    finish_b
        .send(Err("synthetic SSO token expired".into()))
        .unwrap();
    let failed = b.await.unwrap();
    assert_eq!(failed["error_type"], "CredentialsError");
    assert_eq!(failed["needs_sso_login"], true);
    finish_a
        .send(Ok(credentials("CB_SYNTHETIC_A1", 3000)))
        .unwrap();
    assert_eq!(a.await.unwrap()["error_type"], "Superseded");
    assert!(fixture.state.current_ctx().is_none());
    let connection = fixture.state.connection.lock();
    assert_eq!(connection.status, "failed");
    assert_eq!(
        connection.last_attempt.as_ref().unwrap()["profile"],
        "demo-b"
    );
    fixture.no_process();
}

#[tokio::test]
async fn a_delayed_sts_verification_cannot_replace_a_newer_verified_account() {
    let fixture = Fixture::new();
    fixture
        .aws
        .queue_credentials("demo-a")
        .send(Ok(credentials("CB_SYNTHETIC_A1", 3000)))
        .unwrap();
    let finish_identity_a = fixture.aws.queue_identity("CB_SYNTHETIC_A1");
    let a = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A));
    tokio::pin!(a);
    assert!(futures::poll!(&mut a).is_pending());
    fixture
        .aws
        .ready("demo-b", "CB_SYNTHETIC_B1", ACCOUNT_B, "principal-b", 3000);
    assert_eq!(
        aws_set_account_impl(&fixture.state, demo("demo-b", ACCOUNT_B))
            .await
            .unwrap()["ok"],
        true
    );
    finish_identity_a
        .send(Ok(identity(ACCOUNT_A, "principal-a")))
        .unwrap();
    assert_eq!(a.await.unwrap()["error_type"], "Superseded");
    assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_B);
    fixture.no_process();
}

#[tokio::test]
async fn beginning_verification_removes_the_previous_usable_context() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
    let finish_b = fixture.aws.queue_credentials("demo-b");
    fixture
        .aws
        .ready_identity("CB_SYNTHETIC_B1", ACCOUNT_B, "principal-b");
    let pending = aws_set_account_impl(&fixture.state, demo("demo-b", ACCOUNT_B));
    tokio::pin!(pending);
    assert!(futures::poll!(&mut pending).is_pending());
    assert!(fixture.state.current_ctx().is_none());
    assert_eq!(fixture.state.connection.lock().status, "verifying");
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], false);
    assert_eq!(auth["has_context"], false);
    assert_eq!(auth["account_id"], ACCOUNT_B);
    finish_b
        .send(Ok(credentials("CB_SYNTHETIC_B1", 3000)))
        .unwrap();
    assert_eq!(pending.await.unwrap()["ok"], true);
    fixture.no_process();
}

#[tokio::test]
async fn pinned_context_is_verified_independently_and_incomplete_scopes_never_inherit() {
    let fixture = Fixture::new();
    let params = pinned("demo-b", ACCOUNT_B);
    let resolved = resolve_widget_ctx(&fixture.state, &params).unwrap();
    assert!(resolved.inherited_attempt.is_none());
    assert!(fixture.state.current_ctx().is_none());
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_B_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    let pinned_session = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    assert_eq!(pinned_session.identity.account_id, ACCOUNT_B);
    assert_eq!(fixture.state.connection.lock().overrides.len(), 1);
    fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
    let cached = resolve_widget_ctx(&fixture.state, &params).unwrap();
    assert_eq!(cached.context.id(), resolved.context.id());
    assert_eq!(
        verify_request_context(&fixture.state, &cached, &fixture.policy())
            .await
            .unwrap()
            .provider_revision,
        pinned_session.provider_revision
    );
    for invalid in [
        json!({"context": {"mode": "pinned", "profile": "demo-b", "region": "us-east-1"}}),
        json!({"account_override": {"profile": "demo-b", "region": "us-east-1"}}),
        json!({"context": {"mode": "unknown"}}),
    ] {
        assert!(resolve_widget_ctx(&fixture.state, &invalid).is_err());
    }
    let legacy = json!({"account_override": {"profile": "demo-b", "account_id": ACCOUNT_B, "region": "us-east-1"}});
    assert_eq!(
        resolve_widget_ctx(&fixture.state, &legacy)
            .unwrap()
            .context
            .id(),
        resolved.context.id()
    );
    assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_A);
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 2);
    fixture.no_process();
}

#[tokio::test]
async fn same_profile_with_a_different_claimed_account_cannot_reuse_verified_cache() {
    let fixture = Fixture::new();
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_A_PIN",
        ACCOUNT_A,
        "principal-a",
        3000,
    );
    let original = resolve_widget_ctx(&fixture.state, &pinned("demo-a", ACCOUNT_A)).unwrap();
    verify_request_context(&fixture.state, &original, &fixture.policy())
        .await
        .unwrap();
    let spoofed = resolve_widget_ctx(&fixture.state, &pinned("demo-a", ACCOUNT_B)).unwrap();
    assert_ne!(spoofed.context.id(), original.context.id());
    let error = verify_request_context(&fixture.state, &spoofed, &fixture.policy())
        .await
        .unwrap_err();
    assert_eq!(error["error_type"], "UnsupportedProfile");
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    fixture.no_process();
}

#[tokio::test]
async fn settings_path_switch_invalidates_pinned_cache_and_pending_selection() {
    let fixture = Fixture::new();
    let resolved = resolve_widget_ctx(&fixture.state, &pinned("demo-b", ACCOUNT_B)).unwrap();
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_B_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    let finish_a = fixture.aws.queue_credentials("demo-a");
    fixture
        .aws
        .ready_identity("CB_SYNTHETIC_A1", ACCOUNT_A, "principal-a");
    let pending = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A));
    tokio::pin!(pending);
    assert!(futures::poll!(&mut pending).is_pending());
    let previous_revision = fixture.state.connection.lock().settings_revision;
    settings::save(
        &fixture.state.runtime.paths,
        &json!({"aws_config_path": "synthetic-config-b.ini"}),
    );
    let revision = refresh_configuration_revision(&fixture.state);
    assert!(revision > previous_revision);
    let current_attempt = fixture.state.connection.lock().attempt;
    assert!(fixture
        .state
        .begin_attempt(json!({"profile": "stale-fixture"}), previous_revision)
        .is_none());
    assert_eq!(fixture.state.connection.lock().attempt, current_attempt);
    assert!(fixture.state.connection.lock().overrides.is_empty());
    assert!(fixture.state.current_ctx().is_none());
    finish_a
        .send(Ok(credentials("CB_SYNTHETIC_A1", 3000)))
        .unwrap();
    assert_eq!(pending.await.unwrap()["error_type"], "Superseded");
    assert!(fixture.state.current_ctx().is_none());
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_B_NEW_PATH",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    assert_eq!(
        aws_set_account_impl(&fixture.state, demo("demo-b", ACCOUNT_B))
            .await
            .unwrap()["ok"],
        true
    );
    let snapshot = fixture
        .aws
        .resolved_snapshots
        .lock()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        snapshot.config_path,
        PathBuf::from("synthetic-config-b.ini")
    );
    assert_eq!(snapshot.settings_revision, revision);
    fixture.no_process();
}

#[tokio::test]
async fn selected_role_or_sso_session_change_blocks_work_before_new_resolution() {
    for change_role in [true, false] {
        let fixture = Fixture::new();
        fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
        {
            let mut snapshots = fixture.aws.snapshots.lock();
            let snapshot = snapshots.get_mut("demo-a").unwrap().as_mut().unwrap();
            if change_role {
                snapshot.role_name = "AnotherSyntheticRole".into();
            } else {
                snapshot.session_name = Some("changed-synthetic-session".into());
            }
        }
        let result = widget_fetch_impl(
            &fixture.state,
            json!({"widget": "logs-insights", "inputs": {}}),
        )
        .await
        .unwrap();
        assert_eq!(result["error_type"], "ConfigurationChanged");
        assert!(fixture.state.current_ctx().is_none());
        assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
        fixture.no_process();
    }
}

#[tokio::test]
async fn expired_credentials_refresh_same_principal_and_old_results_keep_new_active_session() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_A1", 2000).await;
    let resolved = resolve_widget_ctx(&fixture.state, &json!({})).unwrap();
    let first = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    fixture.clock.0.store(2000, Ordering::SeqCst);
    assert!(first
        .sdk
        .credentials_provider()
        .unwrap()
        .provide_credentials()
        .await
        .is_err());
    fixture
        .aws
        .ready("demo-a", "CB_SYNTHETIC_A2", ACCOUNT_A, "principal-a", 4000);
    let second = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    assert_ne!(first.provider_revision, second.provider_revision);
    assert_eq!(first.identity, second.identity);
    assert_eq!(
        fixture.aws.identity_keys.lock().as_slice(),
        ["CB_SYNTHETIC_A1", "CB_SYNTHETIC_A2"]
    );
    let stale = validate_request_context(&fixture.state, &resolved, &first).unwrap_err();
    assert_eq!(stale["error_type"], "Superseded");
    assert_eq!(
        fixture.state.current_ctx().unwrap().id(),
        resolved.context.id()
    );
    assert_eq!(fixture.state.connection.lock().status, "verified");
    assert!(validate_request_context(&fixture.state, &resolved, &second).is_ok());
    // A later refresh failure belongs to the current selection and must remain
    // visible as a failed login, rather than becoming a stale-attempt response.
    fixture.clock.0.store(4000, Ordering::SeqCst);
    fixture
        .aws
        .queue_credentials("demo-a")
        .send(Err("synthetic SSO token expired".into()))
        .unwrap();
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], false);
    assert_eq!(auth["connection_state"], "failed");
    assert_eq!(auth["error_type"], "CredentialsError");
    assert_eq!(auth["needs_sso_login"], true);
    assert!(auth["set_account_at"].is_null());
    assert!(fixture.state.current_ctx().is_none());
    fixture.no_process();
}

#[tokio::test]
async fn refreshed_principal_change_evicts_pinned_context_and_rejects_its_old_session() {
    let fixture = Fixture::new();
    let resolved = resolve_widget_ctx(&fixture.state, &pinned("demo-a", ACCOUNT_A)).unwrap();
    fixture
        .aws
        .ready("demo-a", "CB_SYNTHETIC_A1", ACCOUNT_A, "principal-a", 2000);
    let first = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap();
    fixture.clock.0.store(2000, Ordering::SeqCst);
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_A2",
        ACCOUNT_A,
        "different-principal",
        4000,
    );
    let error = verify_request_context(&fixture.state, &resolved, &fixture.policy())
        .await
        .unwrap_err();
    assert_eq!(error["error_type"], "IdentityChanged");
    assert!(fixture.state.connection.lock().overrides.is_empty());
    assert!(resolved.context.ensure_current(&first).is_err());
    assert_eq!(
        resolved
            .context
            .verified_session()
            .await
            .unwrap_err()
            .error_type,
        "StaleContext"
    );
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 2);
    fixture.no_process();
}

#[tokio::test]
async fn malformed_policy_and_sso_or_sts_denial_prevent_all_provider_and_process_work() {
    for text in [
        "not: [valid",
        "statements: []",
        "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: [sts:GetCallerIdentity]\n",
        "statements:\n  - effect: Allow\n    action: [sts:GetCallerIdentity]\n",
    ] {
        let fixture = Fixture::new();
        let _ = fixture.policy();
        // Deliberately malformed policy is written only inside this test's
        // isolated application directory, bypassing the editor's validation.
        std::fs::write(aws::policy::policy_path(&fixture.state.runtime.paths), text).unwrap();
        let result = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A)).await.unwrap();
        assert_eq!(result["error_type"], "PolicyDenied");
        let mut params = pinned("demo-a", ACCOUNT_A);
        params["widget"] = json!("cfn-stacks");
        let widget = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_eq!(widget["render"], "permission_denied");
        assert!(fixture.state.current_ctx().is_none());
        fixture.aws.assert_no_resolution();
        assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
        let audit = crate::audit::tail(&fixture.state.runtime.paths, 20);
        assert!(audit.iter().any(|entry| entry["kind"] == "aws-blocked"));
        assert!(audit.iter().all(|entry| entry["ts"] == 1234.0));
        fixture.no_process();
    }
}

#[tokio::test]
async fn command_dispatch_keeps_cli_unavailable_until_verified_child_handoff_exists() {
    let fixture = Fixture::new();
    let result = widget_fetch_impl(
        &fixture.state,
        json!({"widget": "aws-cli", "inputs": {"command": "aws sts get-caller-identity"}}),
    )
    .await
    .unwrap();
    assert_eq!(result["error_type"], "CliContextUnavailable");
    let unsupported = widget_fetch_impl(
        &fixture.state,
        json!({"widget": "aws-cli", "inputs": {"command": "aws ecr batch-delete-image"}}),
    )
    .await
    .unwrap();
    assert_eq!(unsupported["error_type"], "UnsupportedCommand");
    aws::policy::write_text(&fixture.state.runtime.paths, "statements: []").unwrap();
    let denied = widget_fetch_impl(
        &fixture.state,
        json!({"widget": "aws-cli", "inputs": {"command": "aws sts get-caller-identity"}}),
    )
    .await
    .unwrap();
    assert_eq!(denied["render"], "permission_denied");
    fixture.aws.assert_no_resolution();
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.no_process();
}

#[tokio::test]
async fn both_query_workflows_require_stop_permission_before_pinned_provider_work() {
    for deny_stop in [false, true] {
        let fixture = Fixture::new();
        let policy = if deny_stop {
            "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: [logs:StopQuery]\n"
        } else {
            "statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials, sts:GetCallerIdentity, logs:DescribeLogGroups, logs:StartQuery, logs:GetQueryResults]\n"
        };
        aws::policy::write_text(&fixture.state.runtime.paths, policy).unwrap();
        for widget in ["logs-insights", "errors-by-stack"] {
            let mut params = pinned("demo-a", ACCOUNT_A);
            params["widget"] = json!(widget);
            params["inputs"] = json!({"log_group": "/synthetic/group", "query": "fields @message", "stack_name": "synthetic-stack"});
            let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
            assert_eq!(result["render"], "permission_denied");
            assert_eq!(result["action"], "logs:StartQuery");
            assert!(result["reason"]
                .as_str()
                .unwrap()
                .contains("logs:StopQuery"));
        }
        fixture.aws.assert_no_resolution();
        assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
        fixture.no_process();
    }
}

#[tokio::test]
async fn verified_widget_input_errors_return_without_any_sdk_http_work() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
    let result = widget_fetch_impl(
        &fixture.state,
        json!({"widget": "logs-insights", "inputs": {}}),
    )
    .await
    .unwrap();
    assert_eq!(
        result,
        json!({"ok": false, "error": "log_group is required"})
    );
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    fixture.no_process();
}
