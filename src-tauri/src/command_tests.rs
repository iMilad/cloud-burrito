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

#[tokio::test]
async fn fifty_identical_cold_pins_share_one_verified_context_and_one_worker() {
    let fixture = Fixture::new();
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_SHARED_PIN",
        ACCOUNT_A,
        "principal-a",
        3000,
    );
    let finish = fixture.process.queue_output();
    let requests: Vec<_> = (0..50)
        .map(|index| {
            let mut params = pinned("demo-a", ACCOUNT_A);
            params["widget"] = json!("aws-cli");
            params["inputs"] = json!({"command":"aws sts get-caller-identity"});
            params["request_id"] = json!(format!("synthetic-pin-{index}"));
            widget_fetch_impl(&fixture.state, params)
        })
        .collect();
    let all = futures::future::join_all(requests);
    tokio::pin!(all);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.process.calls.load(Ordering::SeqCst) == 0 {
            assert!(futures::poll!(&mut all).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    finish
        .send(Ok(process_output("synthetic shared result")))
        .unwrap();
    let results = all.await;
    let mut context_ids = std::collections::HashSet::new();
    for (index, result) in results.into_iter().enumerate() {
        let result = result.unwrap();
        assert_eq!(result["render"], "table");
        assert_eq!(result["_request"]["id"], format!("synthetic-pin-{index}"));
        context_ids.insert(
            result["_request"]["context_id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(context_ids.len(), 1);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert!(fixture.state.connection.lock().pending_contexts.is_empty());
}

#[tokio::test]
async fn cancellation_that_overtakes_widget_dispatch_prevents_provider_and_cli_work() {
    let fixture = Fixture::new();
    fixture.state.work.cancel("synthetic-already-cancelled");
    let mut params = pinned("demo-a", ACCOUNT_A);
    params["widget"] = json!("aws-cli");
    params["inputs"] = json!({"command":"aws sts get-caller-identity"});
    params["request_id"] = json!("synthetic-already-cancelled");
    let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
    assert_eq!(result["error_type"], "Cancelled");
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[test]
fn pending_context_leases_bound_capacity_and_release_after_the_last_waiter_drops() {
    let fixture = Fixture::new();
    let params: Vec<_> = (0..129)
        .map(|index| pinned(&format!("synthetic-cold-{index}"), ACCOUNT_A))
        .collect();
    let mut reservations: Vec<_> = params[..128]
        .iter()
        .map(|params| resolve_widget_ctx(&fixture.state, params).unwrap())
        .collect();
    assert_eq!(fixture.state.connection.lock().pending_contexts.len(), 128);
    let overflow = resolve_widget_ctx(&fixture.state, &params[128]);
    assert!(matches!(overflow, Err(ref error) if error["error_type"] == "QueueFull"));

    // Existing waiters share a lease even at capacity. One dropped request
    // cannot release another waiter's reservation or create a duplicate context.
    let shared = resolve_widget_ctx(&fixture.state, &params[127]).unwrap();
    assert_eq!(shared.context.id(), reservations[127].context.id());
    drop(reservations.pop().unwrap());
    assert_eq!(fixture.state.connection.lock().pending_contexts.len(), 128);
    drop(shared);
    assert_eq!(fixture.state.connection.lock().pending_contexts.len(), 127);
    let replacement = resolve_widget_ctx(&fixture.state, &params[128]).unwrap();
    assert_eq!(fixture.state.connection.lock().pending_contexts.len(), 128);
    drop(replacement);
    drop(reservations);
    assert!(fixture.state.connection.lock().pending_contexts.is_empty());
    // This exercises reservation/drop only; it must never resolve credentials.
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

type Responses<T> = Mutex<HashMap<String, VecDeque<oneshot::Receiver<Result<T, String>>>>>;

#[derive(Default)]
struct ScriptedAws {
    inspection: Mutex<Option<Value>>,
    inspection_calls: Mutex<Vec<(String, Option<String>)>>,
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
    fn inspect_config(&self, path: &str, constraint: Option<&str>) -> Value {
        self.inspection_calls
            .lock()
            .push((path.into(), constraint.map(str::to_string)));
        self.inspection
            .lock()
            .clone()
            .expect("unexpected native-style profile inspection")
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

struct ProcessScript {
    completion: oneshot::Receiver<Result<ProcessOutput, String>>,
    cancellation_observed: Option<oneshot::Sender<()>>,
}

/// No request is executable unless the test queued its exact completion path.
/// Captures the production handoff without spawning a process or reading env.
#[derive(Default)]
struct RejectProcess {
    calls: AtomicUsize,
    completed: AtomicUsize,
    requests: Mutex<Vec<CliRequest>>,
    scripts: Mutex<VecDeque<ProcessScript>>,
}

impl RejectProcess {
    fn queue_output(&self) -> oneshot::Sender<Result<ProcessOutput, String>> {
        let (tx, rx) = oneshot::channel();
        self.scripts.lock().push_back(ProcessScript {
            completion: rx,
            cancellation_observed: None,
        });
        tx
    }

    fn ready(&self, output: ProcessOutput) {
        self.queue_output().send(Ok(output)).unwrap();
    }

    fn wait_for_cancellation(
        &self,
    ) -> (
        oneshot::Receiver<()>,
        oneshot::Sender<Result<ProcessOutput, String>>,
    ) {
        let (observed_tx, observed_rx) = oneshot::channel();
        let (finish_tx, finish_rx) = oneshot::channel();
        self.scripts.lock().push_back(ProcessScript {
            completion: finish_rx,
            cancellation_observed: Some(observed_tx),
        });
        (observed_rx, finish_tx)
    }
}

impl ProcessRunner for RejectProcess {
    fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let script = self
            .scripts
            .lock()
            .pop_front()
            .expect("unexpected process execution: no scripted completion");
        let cancellation = request.cancellation.clone();
        self.requests.lock().push(request);
        Box::pin(async move {
            if let Some(observed) = script.cancellation_observed {
                cancellation.cancelled().await;
                observed.send(()).expect("cancellation observer dropped");
            }
            let output = script
                .completion
                .await
                .expect("scripted process completion dropped");
            // Represents the injected runner's termination/reap completion.
            // Command tests cannot establish real OS process reaping.
            self.completed.fetch_add(1, Ordering::SeqCst);
            output
        })
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
        assert_eq!(self.process.calls.load(Ordering::SeqCst), 0);
        assert!(self.process.requests.lock().is_empty());
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
        format!("CB_SYNTHETIC_SECRET_{key}"),
        Some(format!("CB_SYNTHETIC_TOKEN_{key}")),
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

/// IPC settings writes replace the full form. Tests spell out changes while
/// explicitly providing all other fields from the backend defaults.
fn full_settings(changes: Value) -> Value {
    let mut form = settings::defaults();
    form.extend(changes.as_object().unwrap().clone());
    Value::Object(form)
}

fn pinned(profile: &str, account: &str) -> Value {
    json!({"context": {"mode": "pinned", "profile": profile, "account_id": account, "region": "us-east-1"}})
}

fn cli_params() -> Value {
    json!({"widget": "aws-cli", "inputs": {"command": "aws sts get-caller-identity"}})
}

fn pinned_cli(profile: &str, account: &str) -> Value {
    let mut params = cli_params();
    params["context"] = pinned(profile, account)["context"].clone();
    params
}

fn process_output(label: &str) -> ProcessOutput {
    ProcessOutput {
        stdout: serde_json::to_vec(&json!({"label": label})).unwrap(),
        stderr: Vec::new(),
        success: true,
    }
}

fn assert_cli_request(request: &CliRequest, key: &str, region: &str, expiry: u64) {
    assert_eq!(
        request.argv,
        ["sts", "get-caller-identity", "--output", "json"]
    );
    assert_eq!(request.region, region);
    assert_eq!(request.credentials.access_key_id(), key);
    assert_eq!(
        request.credentials.secret_access_key(),
        format!("CB_SYNTHETIC_SECRET_{key}")
    );
    assert_eq!(
        request.credentials.session_token(),
        Some(format!("CB_SYNTHETIC_TOKEN_{key}").as_str())
    );
    assert_eq!(
        request.credentials.expiry(),
        Some(UNIX_EPOCH + Duration::from_secs(expiry))
    );
}

fn assert_no_handoff_secrets(fixture: &Fixture, result: &Value, keys: &[&str]) {
    let result = result.to_string();
    let audit =
        serde_json::to_string(&crate::audit::tail(&fixture.state.runtime.paths, 200)).unwrap();
    for key in keys {
        for secret in [
            key.to_string(),
            format!("CB_SYNTHETIC_SECRET_{key}"),
            format!("CB_SYNTHETIC_TOKEN_{key}"),
        ] {
            assert!(!result.contains(&secret));
            assert!(!audit.contains(&secret));
        }
    }
}

#[tokio::test]
async fn ping_and_unverified_auth_status_report_no_aws_work() {
    let fixture = Fixture::new();
    assert_eq!(ping().await.unwrap()["version"], env!("CARGO_PKG_VERSION"));
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], false);
    assert_eq!(auth["has_context"], false);
    assert_eq!(auth["connection_state"], "disconnected");
    assert!(auth["_request"]["context_id"].is_null());
    assert!(resolve_widget_ctx(&fixture.state, &json!({})).is_err());
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn request_ids_are_validated_before_provider_work_and_failures_keep_no_identity() {
    let fixture = Fixture::new();
    for id in [
        json!("bad\nidentifier"),
        json!("x".repeat(129)),
        json!(7),
        json!(""),
    ] {
        let params = json!({"request_id": id, "widget": "aws-cli", "inputs": {"command": "aws sts get-caller-identity"}});
        for result in [
            widget_fetch_impl(&fixture.state, params.clone())
                .await
                .unwrap(),
            aws_set_account_impl(&fixture.state, params.clone())
                .await
                .unwrap(),
            aws_list_pipelines_impl(&fixture.state, params)
                .await
                .unwrap(),
        ] {
            assert_eq!(result["error_type"], "InvalidRequest");
            assert!(result["_request"]["id"].is_null());
            assert!(result["_request"]["account_id"].is_null());
        }
    }
    let mut params = cli_params();
    params["request_id"] = json!("tile.1:refresh-2");
    let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
    assert_eq!(result["_request"]["id"], "tile.1:refresh-2");
    assert!(result["_request"]["context_id"].is_null());
    assert!(result["_request"]["profile"].is_null());
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn connection_auth_and_widget_responses_share_the_verified_context_envelope() {
    let fixture = Fixture::new();
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_ENVELOPE",
        ACCOUNT_A,
        "principal-a",
        3000,
    );
    let mut params = demo("demo-a", ACCOUNT_A);
    params["request_id"] = json!("selection-1");
    let connection = aws_set_account_impl(&fixture.state, params).await.unwrap();
    assert_eq!(connection["_request"]["id"], "selection-1");
    assert_eq!(connection["_request"]["profile"], "demo-a");
    assert_eq!(connection["_request"]["account_id"], ACCOUNT_A);
    assert_eq!(connection["_request"]["region"], "us-east-1");
    for key in ["context_id", "provider_revision", "settings_revision"] {
        assert!(connection["_request"][key].is_string());
    }
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert!(auth["_request"]["id"].is_null());
    fixture
        .process
        .ready(process_output("synthetic envelope response"));
    let mut params = cli_params();
    params["request_id"] = json!("tile-2");
    let widget = widget_fetch_impl(&fixture.state, params).await.unwrap();
    assert_eq!(widget["_request"]["id"], "tile-2");
    for key in [
        "context_id",
        "provider_revision",
        "settings_revision",
        "profile",
        "account_id",
        "region",
    ] {
        assert_eq!(connection["_request"][key], auth["_request"][key]);
        assert_eq!(connection["_request"][key], widget["_request"][key]);
    }
    assert_no_handoff_secrets(&fixture, &widget, &["CB_SYNTHETIC_ENVELOPE"]);
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
        &fixture.state.runtime.storage,
        &json!({"aws_config_path": "synthetic-config-b.ini"}),
    )
    .unwrap();
    let revision = refresh_configuration_revision(&fixture.state).unwrap();
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
            json!({"widget": "logs-insights", "inputs": {"log_group":"/synthetic/group", "query":"fields @message"}}),
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
async fn inherited_cli_receives_exact_sts_verified_credentials_without_payload_fallback() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_CLI_A1", 3000).await;
    fixture
        .process
        .ready(process_output("synthetic inherited result"));
    let mut params = cli_params();
    // These are untrusted widget payload, not configuration or credentials.
    params["inputs"]["profile"] = json!("demo-b");
    params["inputs"]["region"] = json!("eu-west-1");
    params["inputs"]["aws_config_path"] = json!("synthetic-untrusted.ini");
    params["inputs"]["credentials"] = json!({"access_key_id": "CB_SYNTHETIC_UNTRUSTED"});
    let rejected = widget_fetch_impl(&fixture.state, params).await.unwrap();
    assert_eq!(rejected["error_type"], "InvalidRequest");
    fixture.no_process();
    // The clean supported request still receives the exact verified session.
    let result = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(result["render"], "table");
    assert_eq!(result["action"], "sts:GetCallerIdentity");
    assert_eq!(result["account_id"], ACCOUNT_A);
    assert_eq!(result["region"], "us-east-1");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 1);
    {
        let requests = fixture.process.requests.lock();
        assert_cli_request(&requests[0], "CB_SYNTHETIC_CLI_A1", "us-east-1", 3000);
    }
    assert_eq!(
        fixture.aws.identity_keys.lock().as_slice(),
        ["CB_SYNTHETIC_CLI_A1"]
    );
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_no_handoff_secrets(&fixture, &result, &["CB_SYNTHETIC_CLI_A1"]);
}

#[tokio::test]
async fn pinned_cli_uses_its_own_verified_keys_instead_of_active_account_credentials() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_CLI_A1", 3000).await;
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_CLI_B_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    fixture
        .process
        .ready(process_output("synthetic pinned result"));
    let result = widget_fetch_impl(&fixture.state, pinned_cli("demo-b", ACCOUNT_B))
        .await
        .unwrap();
    assert_eq!(result["render"], "table");
    assert_eq!(result["account_id"], ACCOUNT_B);
    assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_A);
    assert_eq!(
        fixture.aws.identity_keys.lock().as_slice(),
        ["CB_SYNTHETIC_CLI_A1", "CB_SYNTHETIC_CLI_B_PIN"]
    );
    {
        let requests = fixture.process.requests.lock();
        assert_eq!(requests.len(), 1);
        assert_cli_request(&requests[0], "CB_SYNTHETIC_CLI_B_PIN", "us-east-1", 3000);
    }
    assert_no_handoff_secrets(
        &fixture,
        &result,
        &["CB_SYNTHETIC_CLI_A1", "CB_SYNTHETIC_CLI_B_PIN"],
    );
}

#[tokio::test]
async fn cli_operation_argument_and_policy_denials_do_no_provider_or_process_work() {
    for command in [
        "aws ecr batch-delete-image",
        "aws sts get-role-credentials",
        "aws sts get-caller-identity --profile demo-b",
        "aws sts get-caller-identity --region eu-west-1",
        "aws sts get-caller-identity --endpoint-url https://example.invalid",
    ] {
        let fixture = Fixture::new();
        aws::policy::write_text(
            &fixture.state.runtime.paths,
            "statements:\n  - effect: Allow\n    action: ['*']\n",
        )
        .unwrap();
        let mut params = pinned_cli("demo-a", ACCOUNT_A);
        params["inputs"]["command"] = json!(command);
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_eq!(result["error_type"], "UnsupportedCommand");
        fixture.aws.assert_no_resolution();
        assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
        fixture.no_process();
    }
    for text in [
        "not: [valid",
        "statements: []",
        "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: [sso:GetRoleCredentials]\n",
        "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: [sts:GetCallerIdentity]\n",
        "statements:\n  - effect: Allow\n    action: ['*']\n  - effect: Deny\n    action: [cloudformation:ListStacks]\n",
    ] {
        let fixture = Fixture::new();
        let _ = fixture.policy();
        std::fs::write(aws::policy::policy_path(&fixture.state.runtime.paths), text).unwrap();
        let mut params = pinned_cli("demo-a", ACCOUNT_A);
        params["inputs"]["command"] = json!("aws cloudformation list-stacks");
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_eq!(result["render"], "permission_denied");
        fixture.aws.assert_no_resolution();
        assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
        fixture.no_process();
    }
}

#[tokio::test]
async fn cli_cannot_use_absent_or_invalid_context_as_an_ambient_credentials_fallback() {
    let fixture = Fixture::new();
    let missing = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(missing["error_type"], "NoVerifiedContext");
    let mut incomplete = cli_params();
    incomplete["context"] = json!({"mode": "pinned", "profile": "demo-a", "region": "us-east-1"});
    assert_eq!(
        widget_fetch_impl(&fixture.state, incomplete).await.unwrap()["error_type"],
        "InvalidRequest"
    );
    let invalid = widget_fetch_impl(&fixture.state, pinned_cli("demo-a", ACCOUNT_B))
        .await
        .unwrap();
    assert_eq!(invalid["error_type"], "UnsupportedProfile");
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn cli_rejects_missing_or_expired_provider_credentials_before_process_handoff() {
    for provided in [
        Err("synthetic SSO credentials unavailable".to_string()),
        Ok(Credentials::new(
            "CB_SYNTHETIC_NO_EXPIRY",
            "CB_SYNTHETIC_SECRET",
            Some("CB_SYNTHETIC_TOKEN".into()),
            None,
            "command-test-only",
        )),
        Ok(credentials("CB_SYNTHETIC_ALREADY_EXPIRED", 1234)),
    ] {
        let fixture = Fixture::new();
        fixture
            .aws
            .queue_credentials("demo-a")
            .send(provided)
            .unwrap();
        let result = widget_fetch_impl(&fixture.state, pinned_cli("demo-a", ACCOUNT_A))
            .await
            .unwrap();
        assert_eq!(result["ok"], false);
        assert!(matches!(
            result["error_type"].as_str(),
            Some("CredentialsError" | "CredentialsExpired")
        ));
        assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 0);
        fixture.no_process();
    }
}

#[tokio::test]
async fn cli_handoff_rejects_incomplete_temporary_credentials_even_after_fake_sts_success() {
    for (key, secret, token) in [
        ("", "CB_SYNTHETIC_SECRET", Some("CB_SYNTHETIC_TOKEN")),
        ("CB_SYNTHETIC_EMPTY_SECRET", "", Some("CB_SYNTHETIC_TOKEN")),
        ("CB_SYNTHETIC_NO_TOKEN", "CB_SYNTHETIC_SECRET", None),
        ("CB_SYNTHETIC_EMPTY_TOKEN", "CB_SYNTHETIC_SECRET", Some("")),
    ] {
        let fixture = Fixture::new();
        fixture.aws.ready_identity(key, ACCOUNT_A, "principal-a");
        fixture
            .aws
            .queue_credentials("demo-a")
            .send(Ok(Credentials::new(
                key,
                secret,
                token.map(str::to_string),
                Some(UNIX_EPOCH + Duration::from_secs(3000)),
                "command-test-only",
            )))
            .unwrap();
        let result = widget_fetch_impl(&fixture.state, pinned_cli("demo-a", ACCOUNT_A))
            .await
            .unwrap();
        assert_eq!(result["error_type"], "CliCredentialsUnavailable");
        assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
        fixture.no_process();
    }
}

#[tokio::test]
async fn cli_after_refresh_receives_the_new_sts_verified_temporary_credentials() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_CLI_A1", 2000).await;
    fixture
        .process
        .ready(process_output("synthetic first session"));
    let first = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(first["render"], "table");
    fixture.clock.0.store(2000, Ordering::SeqCst);
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_CLI_A2",
        ACCOUNT_A,
        "principal-a",
        4000,
    );
    fixture
        .process
        .ready(process_output("synthetic refreshed session"));
    let result = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(result["render"], "table");
    assert_eq!(
        result["_request"]["context_id"],
        first["_request"]["context_id"]
    );
    assert_ne!(
        result["_request"]["provider_revision"],
        first["_request"]["provider_revision"]
    );
    assert_eq!(
        fixture.aws.identity_keys.lock().as_slice(),
        ["CB_SYNTHETIC_CLI_A1", "CB_SYNTHETIC_CLI_A2"]
    );
    {
        let requests = fixture.process.requests.lock();
        assert_eq!(requests.len(), 2);
        assert_cli_request(&requests[0], "CB_SYNTHETIC_CLI_A1", "us-east-1", 2000);
        assert_cli_request(&requests[1], "CB_SYNTHETIC_CLI_A2", "us-east-1", 4000);
    }
    assert_no_handoff_secrets(
        &fixture,
        &result,
        &["CB_SYNTHETIC_CLI_A1", "CB_SYNTHETIC_CLI_A2"],
    );
}

#[tokio::test]
async fn invalidated_cli_waits_for_cancellation_and_runner_completion_before_returning() {
    for change in ["selection", "path", "profile", "expiry"] {
        let fixture = Fixture::new();
        fixture.connect_a("CB_SYNTHETIC_CLI_PENDING", 3000).await;
        let (mut cancelled, finish_process) = fixture.process.wait_for_cancellation();
        let pending = widget_fetch_impl(&fixture.state, cli_params());
        tokio::pin!(pending);
        tokio::time::timeout(Duration::from_secs(3), async {
            while fixture.process.calls.load(Ordering::SeqCst) == 0 {
                assert!(futures::poll!(&mut pending).is_pending());
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("owned CLI worker did not reach the fake runner");
        assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
        match change {
            "selection" => {
                fixture.aws.ready(
                    "demo-b",
                    "CB_SYNTHETIC_NEW_ACTIVE",
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
            }
            "path" => {
                settings::save(
                    &fixture.state.runtime.storage,
                    &json!({"aws_config_path": "synthetic-changed-config.ini"}),
                )
                .unwrap();
            }
            "profile" => {
                fixture
                    .aws
                    .snapshots
                    .lock()
                    .get_mut("demo-a")
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .role_name = "ChangedSyntheticRole".into();
            }
            "expiry" => {
                fixture.clock.0.store(3000, Ordering::SeqCst);
            }
            _ => unreachable!(),
        }
        // Drive the real command monitor until the fake child sees cancellation.
        // The timeout is a failure bound, not a sequencing sleep.
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = &mut pending => panic!("command returned before the runner completed: {result:?}"),
                observed = &mut cancelled => observed.expect("runner never observed cancellation"),
            }
        }).await.expect("context change did not cancel the running CLI");
        assert!(fixture.process.requests.lock()[0]
            .cancellation
            .is_cancelled());
        assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 0);
        assert!(futures::poll!(&mut pending).is_pending());
        finish_process
            .send(Ok(process_output("SYNTHETIC_STALE_RESULT_MUST_NOT_ESCAPE")))
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 1);
        let expected = match change {
            "profile" => "ConfigurationChanged",
            "expiry" => "CredentialsExpired",
            _ => "Superseded",
        };
        assert_eq!(result["error_type"], expected, "change: {change}");
        assert!(!result
            .to_string()
            .contains("SYNTHETIC_STALE_RESULT_MUST_NOT_ESCAPE"));
        if change == "selection" {
            assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_B);
        }
        assert_no_handoff_secrets(&fixture, &result, &["CB_SYNTHETIC_CLI_PENDING"]);
    }
}

#[tokio::test]
async fn cli_cleanup_failure_is_visible_even_after_the_context_is_superseded() {
    for supersede in [false, true] {
        let fixture = Fixture::new();
        fixture.connect_a("CB_SYNTHETIC_CLEANUP", 3000).await;
        if !supersede {
            fixture
                .process
                .queue_output()
                .send(Err(crate::process::CLEANUP_FAILED.into()))
                .unwrap();
            let result = widget_fetch_impl(&fixture.state, cli_params())
                .await
                .unwrap();
            assert_eq!(result["error_type"], "CliCleanupFailed");
            assert_eq!(result["_request"]["outcome"], "failed");
        } else {
            let (mut cancelled, finish) = fixture.process.wait_for_cancellation();
            let pending = widget_fetch_impl(&fixture.state, cli_params());
            tokio::pin!(pending);
            tokio::time::timeout(Duration::from_secs(3), async {
                while fixture.process.calls.load(Ordering::SeqCst) == 0 {
                    assert!(futures::poll!(&mut pending).is_pending());
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("owned CLI worker did not reach the fake runner");
            fixture.aws.ready(
                "demo-b",
                "CB_SYNTHETIC_NEW_ACTIVE",
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
            tokio::time::timeout(Duration::from_secs(2), async {
                tokio::select! {
                    result = &mut pending => panic!("cleanup returned before its outcome: {result:?}"),
                    observed = &mut cancelled => observed.unwrap(),
                }
            }).await.unwrap();
            finish
                .send(Err(crate::process::CLEANUP_FAILED.into()))
                .unwrap();
            let result = pending.await.unwrap();
            assert_eq!(result["error_type"], "CliCleanupFailed");
            assert_eq!(result["_request"]["outcome"], "failed");
            assert_eq!(result["_request"]["profile"], "demo-a");
            assert_eq!(result["_request"]["account_id"], ACCOUNT_A);
            assert_eq!(fixture.state.current_ctx().unwrap().account_id, ACCOUNT_B);
        }
        let audit = crate::audit::tail(&fixture.state.runtime.paths, 50);
        let failure = audit
            .iter()
            .find(|entry| entry["error_type"] == "CliCleanupFailed")
            .unwrap();
        assert_eq!(failure["account_id"], ACCOUNT_A);
        assert!(!failure.to_string().contains("CB_SYNTHETIC_CLEANUP"));
    }
}

#[tokio::test]
async fn unrelated_topbar_switch_does_not_cancel_a_pinned_cli_request() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_ACTIVE_A1", 3000).await;
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_INDEPENDENT_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    let finish_process = fixture.process.queue_output();
    let pending = widget_fetch_impl(&fixture.state, pinned_cli("demo-b", ACCOUNT_B));
    tokio::pin!(pending);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.process.calls.load(Ordering::SeqCst) == 0 {
            assert!(futures::poll!(&mut pending).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pinned CLI worker did not reach the fake runner");
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_ACTIVE_A2",
        ACCOUNT_A,
        "principal-a",
        3000,
    );
    let mut changed_topbar = demo("demo-a", ACCOUNT_A);
    changed_topbar["region"] = json!("eu-west-1");
    assert_eq!(
        aws_set_account_impl(&fixture.state, changed_topbar)
            .await
            .unwrap()["ok"],
        true
    );
    assert!(futures::poll!(&mut pending).is_pending());
    assert!(!fixture.process.requests.lock()[0]
        .cancellation
        .is_cancelled());
    finish_process
        .send(Ok(process_output("synthetic independent pinned result")))
        .unwrap();
    let result = pending.await.unwrap();
    assert_eq!(result["render"], "table");
    assert_eq!(result["account_id"], ACCOUNT_B);
    assert_eq!(result["_request"]["account_id"], ACCOUNT_B);
    assert_eq!(result["_request"]["profile"], "demo-b");
    assert_eq!(result["_request"]["region"], "us-east-1");
    assert_eq!(result["region"], "us-east-1");
    assert_eq!(fixture.state.current_ctx().unwrap().region, "eu-west-1");
    assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 1);
    {
        let requests = fixture.process.requests.lock();
        assert_cli_request(
            &requests[0],
            "CB_SYNTHETIC_INDEPENDENT_PIN",
            "us-east-1",
            3000,
        );
    }
}

#[tokio::test]
async fn cli_runner_error_does_not_expose_the_verified_temporary_credentials() {
    let fixture = Fixture::new();
    let key = "CB_SYNTHETIC_CLI_ERROR";
    fixture.connect_a(key, 3000).await;
    fixture
        .process
        .queue_output()
        .send(Err(format!(
            "synthetic runner error: {key} CB_SYNTHETIC_SECRET_{key} CB_SYNTHETIC_TOKEN_{key}"
        )))
        .unwrap();
    let result = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert_no_handoff_secrets(&fixture, &result, &[key]);
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
            params["inputs"] = if widget == "logs-insights" {
                json!({"log_group":"/synthetic/group", "query":"fields @message"})
            } else {
                json!({})
            };
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
async fn widget_input_errors_return_before_any_additional_provider_work() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_A1", 3000).await;
    let result = widget_fetch_impl(
        &fixture.state,
        json!({"widget": "logs-insights", "inputs": {}}),
    )
    .await
    .unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["error_type"], "InvalidRequest");
    assert!(result["_request"]["account_id"].is_null());
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    fixture.no_process();
}

include!("command_input_tests.rs");
include!("command_audit_tests.rs");

fn audit_events_for(fixture: &Fixture, id: &Value) -> Vec<Value> {
    crate::audit::tail(&fixture.state.runtime.paths, 1000)
        .into_iter()
        .filter(|entry| entry["kind"] == "request" && entry["request_id"] == *id)
        .collect()
}

#[tokio::test]
async fn request_lifecycle_has_one_terminal_outcome_and_no_client_arguments_in_audit() {
    let fixture = Fixture::new();
    let mut denied = cli_params();
    denied["inputs"]["command"] = json!("aws sts get-caller-identity SYNTHETIC_PRIVATE_ARGUMENT");
    denied["request_id"] = json!("synthetic-private-client-id");
    let denied = widget_fetch_impl(&fixture.state, denied).await.unwrap();
    assert_eq!(denied["_request"]["outcome"], "denied");
    assert!(!denied.to_string().contains("SYNTHETIC_PRIVATE_ARGUMENT"));
    let events = audit_events_for(&fixture, &denied["_request"]["audit_id"]);
    assert_eq!(
        events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["denied"]
    );
    fixture.aws.assert_no_resolution();
    fixture.no_process();

    fixture.connect_a("CB_SYNTHETIC_AUDIT", 3000).await;
    for succeeds in [true, false] {
        if succeeds {
            fixture
                .process
                .ready(process_output("synthetic resource value"));
        } else {
            fixture
                .process
                .queue_output()
                .send(Err("SYNTHETIC_PRIVATE_RUNNER_ARGUMENT".into()))
                .unwrap();
        }
        let mut params = cli_params();
        params["request_id"] = json!("synthetic-private-client-id");
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        let expected = if succeeds { "succeeded" } else { "failed" };
        assert_eq!(result["_request"]["outcome"], expected);
        assert!(!result
            .to_string()
            .contains("SYNTHETIC_PRIVATE_RUNNER_ARGUMENT"));
        let events = audit_events_for(&fixture, &result["_request"]["audit_id"]);
        assert_eq!(
            events
                .iter()
                .map(|e| e["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["started", expected]
        );
        assert_eq!(events[1]["account_id"], ACCOUNT_A);
        let preflights = crate::audit::tail(&fixture.state.runtime.paths, 1000);
        assert!(preflights
            .iter()
            .any(|e| e["request_id"] == result["_request"]["audit_id"]
                && e["scope"] == "capability_preflight"));
    }
    let audit =
        std::fs::read_to_string(fixture.state.runtime.paths.data_file("audit.log")).unwrap();
    for marker in [
        "synthetic-private-client-id",
        "SYNTHETIC_PRIVATE_ARGUMENT",
        "SYNTHETIC_PRIVATE_RUNNER_ARGUMENT",
        "CB_SYNTHETIC_AUDIT",
    ] {
        assert!(!audit.contains(marker));
    }
}

#[tokio::test]
async fn dropping_a_pending_command_records_logical_cancellation_once() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_DROP_AUDIT", 3000).await;
    let (cancelled, finish) = fixture.process.wait_for_cancellation();
    let mut pending = Box::pin(widget_fetch_impl(&fixture.state, cli_params()));
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.process.calls.load(Ordering::SeqCst) == 0 {
            assert!(futures::poll!(&mut pending).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned CLI worker did not reach the fake runner");
    let started = crate::audit::tail(&fixture.state.runtime.paths, 1000)
        .into_iter()
        .find(|e| {
            e["kind"] == "request" && e["command"] == "widget_fetch" && e["event"] == "started"
        })
        .unwrap();
    drop(pending);
    tokio::time::timeout(Duration::from_secs(3), cancelled)
        .await
        .expect("dropping the last waiter did not cancel the worker")
        .expect("fake runner dropped its cancellation observer");
    assert!(fixture.process.requests.lock()[0]
        .cancellation
        .is_cancelled());
    assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 0);
    // The owned worker retains the runner until termination/reap completes,
    // even after the UI request and its one logical audit envelope are dropped.
    assert!(!finish.is_closed());
    finish
        .send(Ok(process_output("synthetic dropped request cleanup")))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.process.completed.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned CLI worker did not retain its cleanup completion");
    assert_eq!(fixture.process.completed.load(Ordering::SeqCst), 1);
    let events = audit_events_for(&fixture, &started["request_id"]);
    assert_eq!(
        events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["started", "cancelled"]
    );
}

#[tokio::test]
async fn audit_write_failure_warns_without_blocking_inspection_and_remains_visible() {
    let fixture = Fixture::new();
    let path = fixture.state.runtime.paths.data_file("audit.log");
    std::fs::create_dir_all(&path).unwrap();
    fixture.connect_a("CB_SYNTHETIC_AUDIT_FAILURE", 3000).await;
    fixture
        .process
        .ready(process_output("synthetic usable result"));
    let result = widget_fetch_impl(&fixture.state, cli_params())
        .await
        .unwrap();
    assert_eq!(result["render"], "table");
    assert_eq!(result["_diagnostics"]["audit_write_failed"], true);
    let unreadable = audit_tail_impl(&fixture.state, json!({"limit":200}));
    assert_eq!(unreadable["error_type"], "AuditReadFailed");
    assert!(unreadable.get("entries").is_none());
    std::fs::remove_dir(path).unwrap(); // Only this empty synthetic directory.
    let auth = aws_auth_status_impl(&fixture.state).await.unwrap();
    assert_eq!(auth["logged_in"], true);
    assert_eq!(auth["_diagnostics"]["audit_write_failed"], true);
    assert!(!crate::audit::tail(&fixture.state.runtime.paths, 100).is_empty());
}

#[tokio::test]
async fn provider_and_policy_failures_do_not_echo_private_source_messages() {
    let fixture = Fixture::new();
    fixture
        .aws
        .queue_credentials("demo-a")
        .send(Err("synthetic-private-provider-marker".into()))
        .unwrap();
    let result = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A))
        .await
        .unwrap();
    assert_eq!(result["error_type"], "CredentialsError");
    assert_eq!(result["_request"]["outcome"], "failed");
    assert_eq!(result["needs_sso_login"], true);
    assert!(!result
        .to_string()
        .contains("synthetic-private-provider-marker"));
    let audit =
        std::fs::read_to_string(fixture.state.runtime.paths.data_file("audit.log")).unwrap();
    assert!(!audit.contains("synthetic-private-provider-marker"));
    for text in [
        "statements: [synthetic-private-policy-marker]",
        "statements:\n  - effect: synthetic-private-policy-marker\n    action: ['*']\n",
    ] {
        let error = aws::policy::Policy::parse(text).unwrap_err();
        assert!(!error.message.contains("synthetic-private-policy-marker"));
    }
}

#[tokio::test]
async fn broken_settings_block_every_provider_entry_and_clear_verified_context() {
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_STORAGE", 3000).await;
    let snapshot_calls = fixture.aws.snapshot_calls.load(Ordering::SeqCst);
    let path = fixture.state.runtime.paths.data_file("settings.json");
    std::fs::write(&path, "{synthetic-invalid-json").unwrap();
    let results = [
        settings_get_impl(&fixture.state),
        aws_list_profiles_impl(&fixture.state),
        aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A))
            .await
            .unwrap(),
        aws_auth_status_impl(&fixture.state).await.unwrap(),
        widget_fetch_impl(&fixture.state, cli_params())
            .await
            .unwrap(),
    ];
    for result in results {
        assert_eq!(result["ok"], false);
        assert_eq!(result["error_type"], "StorageInvalid");
        assert!(!result.to_string().contains("synthetic-invalid-json"));
        assert!(!result.to_string().contains(path.to_str().unwrap()));
    }
    assert!(fixture.state.current_ctx().is_none());
    assert!(fixture.state.connection.lock().overrides.is_empty());
    assert_eq!(
        fixture.aws.snapshot_calls.load(Ordering::SeqCst),
        snapshot_calls
    );
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{synthetic-invalid-json"
    );
    let failed_revision = fixture.state.connection.lock().settings_revision;
    let recovered = settings_set_impl(
        &fixture.state,
        full_settings(json!({"default_region":"us-east-1"})),
    );
    assert_eq!(recovered["_storage"]["status"], "saved");
    assert!(fixture.state.connection.lock().settings_revision > failed_revision);
    assert!(fixture.state.current_ctx().is_none());
    fixture
        .connect_a("CB_SYNTHETIC_RECOVERED_STORAGE", 3000)
        .await;
    assert!(fixture.state.current_ctx().is_some());
}

#[test]
fn persistence_commands_distinguish_first_run_empty_read_and_write_failure() {
    let fixture = Fixture::new();
    assert_eq!(
        dashboard_get_impl(&fixture.state)["_storage"]["status"],
        "missing"
    );
    assert_eq!(
        dashboard_set_impl(&fixture.state, json!({"tiles":[]}))["_storage"]["status"],
        "saved"
    );
    assert_eq!(
        dashboard_get_impl(&fixture.state)["_storage"]["status"],
        "loaded"
    );
    let path = fixture.state.runtime.paths.data_file("settings.json");
    std::fs::create_dir_all(&path).unwrap();
    assert_eq!(
        settings_get_impl(&fixture.state)["error_type"],
        "StorageReadFailed"
    );
    let response = settings_set_impl(
        &fixture.state,
        full_settings(json!({"default_region":"us-east-1"})),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error_type"], "StorageWriteFailed");
    assert_eq!(response["_storage"]["status"], "failed");
    assert!(path.is_dir());
}

#[test]
fn settings_metadata_is_available_on_success_validation_and_storage_failure() {
    let fixture = Fixture::new();
    let initial = settings_get_impl(&fixture.state);
    assert_eq!(initial["default_profile"], "");
    assert_eq!(initial["theme"], "dark");
    assert_eq!(
        initial["_settings"]["defaults"],
        Value::Object(settings::defaults())
    );
    assert_eq!(
        initial["_settings"]["allowed_regions"],
        json!(settings::ALLOWED_REGIONS)
    );
    let path = fixture.state.runtime.paths.data_file("settings.json");
    for (field, invalid) in [
        ("default_region", json!("ap-south-1")),
        ("theme", json!("system")),
        ("default_profile", json!(12)),
    ] {
        let mut params = full_settings(json!({}));
        params[field] = invalid;
        let response = settings_set_impl(&fixture.state, params);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error_type"], "InvalidRequest");
        assert!(response["_settings"]["field_errors"][field].is_string());
        assert!(!path.exists());
    }
    let blanks = settings_set_impl(
        &fixture.state,
        json!({"aws_config_path":" \t", "sso_session_name":" ", "default_profile":" ", "default_region":" ", "theme":"\t"}),
    );
    assert_eq!(blanks["_storage"]["status"], "saved");
    assert_eq!(blanks["theme"], "dark");
    assert_eq!(blanks["default_region"], "eu-west-1");
    assert_eq!(blanks["_settings"]["field_errors"], json!({}));
    fixture
        .state
        .runtime
        .storage
        .write_json(
            "settings.json",
            &json!({"default_region":"ap-south-1", "theme":"light"}),
        )
        .unwrap();
    let visible = settings_get_impl(&fixture.state);
    assert_eq!(visible["default_region"], "ap-south-1");
    assert_eq!(visible["_storage"]["status"], "loaded");
    assert!(visible["_settings"]["field_errors"]["default_region"].is_string());
    let before = std::fs::read(&path).unwrap();
    let rejected = settings_set_impl(
        &fixture.state,
        full_settings(json!({"default_region":"ap-south-1", "theme":"dark"})),
    );
    assert_eq!(rejected["ok"], false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let repaired = settings_set_impl(
        &fixture.state,
        full_settings(json!({"default_region":"us-east-1", "theme":" light "})),
    );
    assert_eq!(repaired["_storage"]["status"], "saved");
    assert_eq!(settings_get_impl(&fixture.state)["theme"], "light");
    std::fs::write(&path, "{synthetic-broken-settings").unwrap();
    let error = settings_get_impl(&fixture.state);
    assert_eq!(error["error_type"], "StorageInvalid");
    assert_eq!(error["_settings"]["defaults"]["theme"], "dark");
    assert_eq!(error["_settings"]["field_errors"], json!({}));
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn unsupported_default_and_pinned_regions_are_preserved_but_never_executed() {
    let fixture = Fixture::new();
    fixture
        .state
        .runtime
        .storage
        .write_json("settings.json", &json!({"default_region":"ap-south-1"}))
        .unwrap();
    let mut selection = demo("demo-a", ACCOUNT_A);
    selection.as_object_mut().unwrap().remove("region");
    let denied = aws_set_account_impl(&fixture.state, selection)
        .await
        .unwrap();
    assert_eq!(denied["error_type"], "UnsupportedRegion");
    assert_eq!(denied["connection_state"], "failed");
    let mut pin = pinned("demo-a", ACCOUNT_A);
    pin["context"]["region"] = json!("ap-south-1");
    let tile = json!({"id":"unsupported-region-fixture", "widget":"aws-cli", "x":0,"y":0,"w":4,"h":3,"config":pin});
    let persisted = dashboard_set_impl(&fixture.state, json!({"tiles":[tile.clone()]}));
    assert_eq!(persisted["_storage"]["status"], "saved");
    assert_eq!(dashboard_get_impl(&fixture.state)["tiles"][0], tile);
    let mut request = cli_params();
    request["context"] = pin["context"].clone();
    let denied_pin = widget_fetch_impl(&fixture.state, request).await.unwrap();
    assert_eq!(denied_pin["error_type"], "UnsupportedRegion");
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.aws.assert_no_resolution();
    fixture.no_process();
    // An explicit supported region is usable without rewriting the saved default.
    fixture
        .connect_a("CB_SYNTHETIC_EXPLICIT_REGION", 3000)
        .await;
    assert_eq!(fixture.state.current_ctx().unwrap().region, "us-east-1");
    assert_eq!(
        settings_get_impl(&fixture.state)["default_region"],
        "ap-south-1"
    );
}

#[tokio::test]
async fn saved_session_constraint_applies_to_topbar_and_pinned_verification() {
    let fixture = Fixture::new();
    let settings = settings_set_impl(
        &fixture.state,
        full_settings(json!({"sso_session_name":"synthetic-session"})),
    );
    assert_eq!(settings["_storage"]["status"], "saved");
    let mut conflicting = demo("demo-a", ACCOUNT_A);
    conflicting["sso_session_name"] = json!("synthetic-other");
    let conflict = aws_set_account_impl(&fixture.state, conflicting)
        .await
        .unwrap();
    assert_eq!(conflict["error_type"], "SsoSessionConflict");
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.aws.assert_no_resolution();
    let mut matching = demo("demo-a", ACCOUNT_A);
    matching["sso_session_name"] = json!("synthetic-session");
    fixture.aws.ready(
        "demo-a",
        "CB_SYNTHETIC_SESSION_A",
        ACCOUNT_A,
        "principal-a",
        3000,
    );
    assert_eq!(
        aws_set_account_impl(&fixture.state, matching)
            .await
            .unwrap()["ok"],
        true
    );
    assert_eq!(
        fixture
            .state
            .current_ctx()
            .unwrap()
            .sso_session_name
            .as_deref(),
        Some("synthetic-session")
    );
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_SESSION_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    let pin = resolve_widget_ctx(&fixture.state, &pinned("demo-b", ACCOUNT_B)).unwrap();
    assert_eq!(
        pin.context.sso_session_name.as_deref(),
        Some("synthetic-session")
    );
    let verified = verify_request_context(&fixture.state, &pin, &fixture.policy())
        .await
        .unwrap();
    assert_eq!(
        verified.snapshot.session_name.as_deref(),
        Some("synthetic-session")
    );
    fixture.no_process();
}

#[tokio::test]
async fn saved_session_never_overrides_a_different_selected_profile_session() {
    for pinned_request in [false, true] {
        let fixture = Fixture::new();
        settings_set_impl(
            &fixture.state,
            full_settings(json!({"sso_session_name":"synthetic-different"})),
        );
        let error = if pinned_request {
            let context = resolve_widget_ctx(&fixture.state, &pinned("demo-a", ACCOUNT_A)).unwrap();
            verify_request_context(&fixture.state, &context, &fixture.policy())
                .await
                .unwrap_err()
        } else {
            aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A))
                .await
                .unwrap()
        };
        assert_eq!(error["error_type"], "UnsupportedProfile");
        assert!(fixture.state.current_ctx().is_none());
        assert!(fixture.state.connection.lock().overrides.is_empty());
        fixture.aws.assert_no_resolution();
        fixture.no_process();
    }
}

#[tokio::test]
async fn only_credential_settings_invalidate_verified_and_pinned_contexts() {
    let fixture = Fixture::new();
    let initial = settings_set_impl(
        &fixture.state,
        full_settings(json!({
            "aws_config_path":"synthetic-custom-config.ini", "sso_session_name":"synthetic-session"
        })),
    );
    assert_eq!(initial["_storage"]["status"], "saved");
    fixture.connect_a("CB_SYNTHETIC_PREF_A", 3000).await;
    fixture.aws.ready(
        "demo-b",
        "CB_SYNTHETIC_PREF_PIN",
        ACCOUNT_B,
        "principal-b",
        3000,
    );
    let pin = resolve_widget_ctx(&fixture.state, &pinned("demo-b", ACCOUNT_B)).unwrap();
    let pinned_session = verify_request_context(&fixture.state, &pin, &fixture.policy())
        .await
        .unwrap();
    let old_id = fixture.state.current_ctx().unwrap().id();
    let old_revision = fixture.state.connection.lock().settings_revision;
    let path = fixture.state.runtime.paths.data_file("settings.json");
    let saved_bytes = std::fs::read(&path).unwrap();
    let snapshots = fixture.aws.snapshot_calls.load(Ordering::SeqCst);
    let partial = settings_set_impl(&fixture.state, json!({"theme":"light"}));
    assert_eq!(partial["error_type"], "InvalidRequest");
    assert_eq!(
        partial["error"],
        "Submit every settings field; use a blank value to reset a field"
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    assert_eq!(fixture.state.current_ctx().unwrap().id(), old_id);
    assert_eq!(
        fixture.state.connection.lock().settings_revision,
        old_revision
    );
    assert_eq!(fixture.state.connection.lock().overrides.len(), 1);
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), snapshots);
    let preference_save = settings_set_impl(
        &fixture.state,
        full_settings(
            json!({"aws_config_path":"synthetic-custom-config.ini", "sso_session_name":"synthetic-session", "theme":"light", "default_region":"eu-west-1", "default_profile":"demo-b"}),
        ),
    );
    assert_eq!(preference_save["_storage"]["status"], "saved");
    assert_eq!(fixture.state.current_ctx().unwrap().id(), old_id);
    assert_eq!(
        fixture.state.connection.lock().settings_revision,
        old_revision
    );
    assert_eq!(fixture.state.connection.lock().overrides.len(), 1);
    assert_eq!(
        fixture.state.current_ctx().unwrap().aws_config_path,
        "synthetic-custom-config.ini"
    );
    assert_eq!(
        fixture
            .state
            .current_ctx()
            .unwrap()
            .sso_session_name
            .as_deref(),
        Some("synthetic-session")
    );
    validate_request_context(&fixture.state, &pin, &pinned_session).unwrap();
    let constraint_save = settings_set_impl(
        &fixture.state,
        full_settings(
            json!({"aws_config_path":"synthetic-custom-config.ini", "sso_session_name":"", "theme":"light"}),
        ),
    );
    assert_eq!(constraint_save["_storage"]["status"], "saved");
    assert!(fixture.state.connection.lock().settings_revision > old_revision);
    assert!(fixture.state.current_ctx().is_none());
    assert!(fixture.state.connection.lock().overrides.is_empty());
    assert_eq!(
        validate_request_context(&fixture.state, &pin, &pinned_session).unwrap_err()["error_type"],
        "Superseded"
    );
    let next = resolve_widget_ctx(&fixture.state, &pinned("demo-b", ACCOUNT_B)).unwrap();
    assert_ne!(next.context.id(), pin.context.id());
    assert!(next.context.sso_session_name.is_none());
    fixture.no_process();
}

#[tokio::test]
async fn changing_only_saved_session_fences_an_older_pending_selection() {
    let fixture = Fixture::new();
    let finish = fixture.aws.queue_credentials("demo-a");
    fixture
        .aws
        .ready_identity("CB_SYNTHETIC_PENDING_SESSION", ACCOUNT_A, "principal-a");
    let pending = aws_set_account_impl(&fixture.state, demo("demo-a", ACCOUNT_A));
    tokio::pin!(pending);
    assert!(futures::poll!(&mut pending).is_pending());
    let saved = settings_set_impl(
        &fixture.state,
        full_settings(json!({"sso_session_name":"synthetic-session"})),
    );
    assert_eq!(saved["_storage"]["status"], "saved");
    finish
        .send(Ok(credentials("CB_SYNTHETIC_PENDING_SESSION", 3000)))
        .unwrap();
    assert_eq!(pending.await.unwrap()["error_type"], "Superseded");
    assert!(fixture.state.current_ctx().is_none());
    fixture.no_process();
}

#[test]
fn profile_discovery_uses_saved_path_and_session_constraint_without_credentials() {
    let fixture = Fixture::new();
    let saved = settings_set_impl(
        &fixture.state,
        full_settings(json!({
            "aws_config_path":"synthetic-discovery.ini", "sso_session_name":" synthetic-session "
        })),
    );
    assert_eq!(saved["_storage"]["status"], "saved");
    *fixture.aws.inspection.lock() = Some(
        json!({"ok":true, "discovery_state":"ready", "profiles":[
            {"name":"demo-a", "account_id":ACCOUNT_A, "role_name":"SyntheticReadOnly", "region":"us-east-1", "sso_session":"synthetic-session", "eligibility":"supported_sso", "eligibility_reason":null}
        ]}),
    );
    let discovered = aws_list_profiles_impl(&fixture.state);
    assert_eq!(discovered["discovery_state"], "ready");
    assert_eq!(discovered["profiles"][0]["eligibility"], "supported_sso");
    assert_eq!(
        discovered["allowed_regions"],
        json!(settings::ALLOWED_REGIONS)
    );
    assert_eq!(
        *fixture.aws.inspection_calls.lock(),
        vec![(
            "synthetic-discovery.ini".into(),
            Some("synthetic-session".into())
        )]
    );
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn policy_change_keeps_unknown_query_cleanup_and_recovery_in_final_envelope() {
    use crate::test_aws::{ExpectedRequest, ScriptedHttp};
    let fixture = Fixture::new();
    fixture.connect_a("CB_SYNTHETIC_QUERY_CLEANUP", 3000).await;
    let resolved = resolve_widget_ctx(&fixture.state, &json!({})).unwrap();
    let policy = fixture.policy();
    let revision = fixture.state.observe_policy(&policy);
    let session = verify_request_context(&fixture.state, &resolved, &policy)
        .await
        .unwrap();
    let http = ScriptedHttp::new(vec![ExpectedRequest::json(
        "Logs_20140328.StartQuery",
        json!({"logGroupName":"/synthetic/policy-change"}),
        json!({"queryId":"synthetic-late-query"}),
    )
    .delay(Duration::from_millis(150))]);
    let mut ctx = http.context(
        &fixture._dir,
        "logs-insights",
        json!({"log_group":"/synthetic/policy-change","query":"fields @message"}),
    );
    let scope = crate::scheduler::WorkScope::new(
        "synthetic-verified-query".into(),
        ACCOUNT_A.into(),
        "us-east-1".into(),
        crate::process::ProcessCancellation::new(),
        crate::scheduler::WorkBudget::for_widget("logs-insights"),
    );
    ctx.runtime = fixture.state.runtime.with_work(scope);
    ctx.account_id = ACCOUNT_A.into();
    ctx.policy = policy;
    let job = Box::pin(run_widget_job(
        fixture.state.clone(),
        resolved,
        session,
        ctx,
        revision,
    ));
    let revoke = async {
        while http.calls() == 0 {
            tokio::task::yield_now().await;
        }
        aws::policy::write_text(
            &fixture.state.runtime.paths,
            "statements:\n  - effect: Deny\n    action: ['*']\n",
        )
        .unwrap();
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(job, revoke) })
            .await
            .unwrap();
    assert_eq!(result["error_type"], "PolicyChanged");
    assert_eq!(result["cleanup"]["status"], "denied");
    assert_eq!(result["cleanup"]["remote_queries_may_still_run"], true);
    assert_eq!(result["recovery_required"], true);
    assert_eq!(result["recovery_pending"].as_array().unwrap().len(), 1);
    http.assert_finished();
    fixture.no_process();
}
async fn seed_verified_detail(fixture: &Fixture) -> (Value, String) {
    fixture.connect_a("CB_SYNTHETIC_CACHE_A", 3000).await;
    let params = json!({"widget":"cfn-stack-detail","inputs":{"stack_name":"synthetic-stack"},"reuse_result":true,"request_id":"synthetic-cache-read"});
    let policy = fixture.policy();
    let revision = fixture.state.observe_policy(&policy);
    let resolved = resolve_widget_ctx(&fixture.state, &params).unwrap();
    let session = verify_request_context(&fixture.state, &resolved, &policy)
        .await
        .unwrap();
    let authority = json!([
        resolved.context.id(),
        session.provider_revision,
        session.snapshot.settings_revision,
        revision,
        session.identity.account_id,
        session.identity.arn,
        session.identity.user_id,
        session.snapshot.profile,
        session.snapshot.config_path,
        session.snapshot.region
    ])
    .to_string();
    let key = json!([authority, "cfn-stack-detail", params["inputs"]]).to_string();
    fixture.state.results.insert(key.clone(),&json!({"render":"table","columns":["stack"],"rows":[{"stack":"synthetic-cache-evidence"}],"coverage":{"completeness":"complete"}}),1200.0);
    (params, key)
}

#[tokio::test]
async fn cached_detail_is_reverified_and_preserves_capture_time_and_request_owner() {
    let fixture = Fixture::new();
    let (params, _) = seed_verified_detail(&fixture).await;
    let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
    assert_eq!(result["_cache"]["hit"], true);
    assert_eq!(result["_cache"]["captured_at"], 1200.0);
    assert_eq!(result["_request"]["id"], "synthetic-cache-read");
    assert_eq!(result["_request"]["account_id"], ACCOUNT_A);
    assert_eq!(fixture.aws.credential_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.aws.identity_calls.load(Ordering::SeqCst), 1);
    fixture.no_process();
    let audit = crate::audit::try_tail(&fixture.state.runtime.paths, 100).unwrap();
    assert_eq!(
        audit.iter().filter(|e| e["event"] == "cache_hit").count(),
        1
    );
}

#[tokio::test]
async fn cached_protected_data_cannot_survive_a_new_deny_or_refreshed_principal() {
    for change in ["policy", "principal"] {
        let fixture = Fixture::new();
        let (params, key) = seed_verified_detail(&fixture).await;
        if change == "policy" {
            std::fs::write(
                fixture.state.runtime.paths.data_file("policy.yaml"),
                "statements:\n  - effect: Deny\n    action: ['*:*']\n",
            )
            .unwrap();
        } else {
            fixture.clock.0.store(2980, Ordering::SeqCst);
            fixture.aws.ready(
                "demo-a",
                "CB_SYNTHETIC_CACHE_CHANGED",
                ACCOUNT_A,
                "changed-principal",
                5000,
            );
        }
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert!(result.get("_cache").is_none(), "{change}: {result}");
        assert!(result.get("rows").is_none());
        assert!(fixture.state.results.get(&key).is_none());
        fixture.no_process();
    }
}

#[tokio::test]
async fn saved_configuration_change_clears_detail_results_before_reuse() {
    let fixture = Fixture::new();
    let (_, key) = seed_verified_detail(&fixture).await;
    settings::save(
        &fixture.state.runtime.storage,
        &json!({"aws_config_path":"synthetic-other-config.ini"}),
    )
    .unwrap();
    load_settings(&fixture.state).unwrap();
    assert!(fixture.state.results.get(&key).is_none());
}

#[tokio::test]
async fn cached_detail_commands_isolate_account_profile_and_region() {
    for dimension in ["account", "profile", "region"] {
        let fixture = Fixture::new();
        let (original, original_key) = seed_verified_detail(&fixture).await;
        let (profile, account, region) = match dimension {
            "account" => ("demo-b", ACCOUNT_B, "us-east-1"),
            "profile" => ("synthetic-profile-alias", ACCOUNT_A, "us-east-1"),
            _ => ("demo-a", ACCOUNT_A, "eu-west-1"),
        };
        if dimension == "profile" {
            fixture.aws.profile(profile, account);
        }
        fixture.aws.ready(
            profile,
            "CB_SYNTHETIC_CACHE_ISOLATED",
            account,
            "principal-a",
            3000,
        );
        let mut params = original.clone();
        params["context"] =
            json!({"mode":"pinned","profile":profile,"account_id":account,"region":region});
        params["request_id"] = json!("synthetic-isolated-cache-read");
        let policy = fixture.policy();
        let revision = fixture.state.observe_policy(&policy);
        let resolved = resolve_widget_ctx(&fixture.state, &params).unwrap();
        let session = verify_request_context(&fixture.state, &resolved, &policy)
            .await
            .unwrap();
        let authority = json!([
            resolved.context.id(),
            session.provider_revision,
            session.snapshot.settings_revision,
            revision,
            session.identity.account_id,
            session.identity.arn,
            session.identity.user_id,
            session.snapshot.profile,
            session.snapshot.config_path,
            session.snapshot.region
        ])
        .to_string();
        let isolated_key = json!([authority, "cfn-stack-detail", params["inputs"]]).to_string();
        assert_ne!(original_key, isolated_key, "{dimension}");
        fixture.state.results.insert(isolated_key, &json!({"render":"table","columns":["stack"],
            "rows":[{"stack":"synthetic-isolated-evidence"}],"coverage":{"completeness":"complete"}}), 1230.0);
        let isolated = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_eq!(
            isolated["rows"][0]["stack"], "synthetic-isolated-evidence",
            "{dimension}"
        );
        assert_eq!(isolated["_cache"]["captured_at"], 1230.0);
        assert_eq!(isolated["_request"]["account_id"], account);
        assert_eq!(isolated["_request"]["region"], region);
        // Returning to the original verified context finds its own evidence.
        let original_result = widget_fetch_impl(&fixture.state, original).await.unwrap();
        assert_eq!(
            original_result["rows"][0]["stack"], "synthetic-cache-evidence",
            "{dimension}"
        );
        assert_eq!(original_result["_cache"]["captured_at"], 1200.0);
        fixture.no_process();
    }
}
