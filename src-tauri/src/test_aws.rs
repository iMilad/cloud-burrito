//! Test-only AWS configuration that rejects unplanned provider and HTTP work.
//!
//! Command tests control credential resolution and caller identity through the
//! injected backend. This configuration is a second fence for code that builds
//! a service client directly from `WidgetCtx::sdk` instead of that backend.

use aws_config::{identity::IdentityCache, retry::RetryConfig, BehaviorVersion, Region, SdkConfig};
use aws_credential_types::provider::{future, ProvideCredentials, SharedCredentialsProvider};
use aws_smithy_runtime_api::client::{
    http::{http_client_fn, HttpConnector, HttpConnectorFuture, SharedHttpConnector},
    orchestrator::{HttpRequest, HttpResponse},
    stalled_stream_protection::StalledStreamProtectionConfig,
};
use aws_smithy_types::body::SdkBody;
use aws_smithy_types::timeout::TimeoutConfig;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

/// Attempted boundary crossings, counted before a fence panics.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub credentials: AtomicUsize,
    pub transport: AtomicUsize,
}

#[derive(Debug)]
struct RejectCredentials(Arc<Counters>);

impl ProvideCredentials for RejectCredentials {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        self.0.credentials.fetch_add(1, Ordering::SeqCst);
        panic!("unexpected AWS credential provider work in test");
    }
}

/// Build without consulting environment, AWS profiles, token caches or IMDS.
///
/// A direct credential lookup panics. An SDK operation that is explicitly given
/// synthetic credentials still panics when selecting its HTTP connector, before
/// connector construction, DNS, TLS or network I/O. Returning a normal SDK error
/// here would let a widget swallow an unexpected boundary call as ordinary data.
pub(crate) fn sdk_config() -> SdkConfig {
    sdk_config_with_counters(Arc::new(Counters::default()))
}

/// Share counters across configured contexts to assert zero SDK boundary work.
pub(crate) fn sdk_config_with_counters(counters: Arc<Counters>) -> SdkConfig {
    let provider = SharedCredentialsProvider::new(RejectCredentials(counters.clone()));
    config_with_provider(provider, counters)
}

fn config_with_provider(provider: SharedCredentialsProvider, counters: Arc<Counters>) -> SdkConfig {
    SdkConfig::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url("https://cloud-burrito-test.invalid")
        .credentials_provider(provider)
        // Each unexpected SDK call must reach its provider fence rather than
        // reusing an identity loaded by an earlier operation or client clone.
        .identity_cache(IdentityCache::no_cache())
        .retry_config(RetryConfig::disabled())
        .timeout_config(TimeoutConfig::disabled())
        .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
        // `None` would allow service defaults to install a real HTTPS client.
        .http_client(http_client_fn(move |_, _| {
            counters.transport.fetch_add(1, Ordering::SeqCst);
            panic!("unexpected AWS HTTP transport in test")
        }))
        .build()
}

const TEST_ENDPOINT: &str = "https://cloud-burrito-test.invalid";

enum RequestMatch {
    Json {
        target: &'static str,
        fields: Value,
    },
    Xml {
        action: &'static str,
        fields: Value,
    },
    Rest {
        method: &'static str,
        path: String,
        fields: Value,
    },
}

/// Explicit synthetic request/response pair. No request headers or credentials
/// are retained or printed if matching fails.
pub(crate) struct ExpectedRequest {
    matcher: RequestMatch,
    status: u16,
    content_type: &'static str,
    response: String,
    delay: Duration,
}

impl std::fmt::Debug for ExpectedRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExpectedRequest (synthetic fixture)")
    }
}

impl ExpectedRequest {
    pub(crate) fn json(target: &'static str, required_body_fields: Value, response: Value) -> Self {
        Self {
            matcher: RequestMatch::Json {
                target,
                fields: required_body_fields,
            },
            status: 200,
            content_type: "application/x-amz-json-1.1",
            response: response.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn xml(
        action: &'static str,
        required_form_fields: Value,
        response_xml: &str,
    ) -> Self {
        Self {
            matcher: RequestMatch::Xml {
                action,
                fields: required_form_fields,
            },
            status: 200,
            content_type: "text/xml",
            response: response_xml.into(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn rest(
        method: &'static str,
        path: &str,
        required_query_fields: Value,
        response: Value,
    ) -> Self {
        Self {
            matcher: RequestMatch::Rest {
                method,
                path: path.into(),
                fields: required_query_fields,
            },
            status: 200,
            content_type: "application/json",
            response: response.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub(crate) fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn matches(&self, request: &HttpRequest, route: &str) -> bool {
        let (path, query) = route.split_once('?').unwrap_or((route, ""));
        match &self.matcher {
            RequestMatch::Json { target, fields } => {
                request.method() == "POST"
                    && path == "/"
                    && request.headers().get("x-amz-target") == Some(*target)
                    && request
                        .body()
                        .bytes()
                        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
                        .is_some_and(|body| object_contains(&body, fields))
            }
            RequestMatch::Xml { action, fields } => {
                if request.method() != "POST" || path != "/" {
                    return false;
                }
                let body = request
                    .body()
                    .bytes()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok());
                body.is_some_and(|body| {
                    let form = form_fields(body);
                    form.get("Action")
                        .is_some_and(|value| value.as_str() == *action)
                        && form_contains(&form, fields)
                })
            }
            RequestMatch::Rest {
                method,
                path: expected_path,
                fields,
            } => {
                request.method() == *method
                    && path == expected_path
                    && form_contains(&form_fields(query), fields)
            }
        }
    }
}

fn object_contains(actual: &Value, required: &Value) -> bool {
    required
        .as_object()
        .expect("fixture request fields must be an object")
        .iter()
        .all(|(key, value)| {
            if value.is_null() {
                actual.get(key).is_none()
            } else {
                actual.get(key) == Some(value)
            }
        })
}

fn form_contains(actual: &BTreeMap<String, String>, required: &Value) -> bool {
    required
        .as_object()
        .expect("fixture request fields must be an object")
        .iter()
        .all(|(key, value)| {
            if value.is_null() {
                actual.get(key).is_none()
            } else {
                actual
                    .get(key)
                    .is_some_and(|actual| Some(actual.as_str()) == value.as_str())
            }
        })
}

fn form_fields(text: &str) -> BTreeMap<String, String> {
    fn decode(value: &str) -> String {
        let mut out = Vec::new();
        let mut bytes = value.as_bytes().iter().copied();
        while let Some(byte) = bytes.next() {
            out.push(match byte {
                b'+' => b' ',
                b'%' => {
                    let high = (bytes.next().expect("fixture percent escape") as char)
                        .to_digit(16)
                        .expect("fixture percent escape");
                    let low = (bytes.next().expect("fixture percent escape") as char)
                        .to_digit(16)
                        .expect("fixture percent escape");
                    (high * 16 + low) as u8
                }
                other => other,
            });
        }
        String::from_utf8(out).expect("fixture form must be UTF-8")
    }
    text.split('&')
        .filter(|field| !field.is_empty())
        .map(|field| {
            let (key, value) = field.split_once('=').unwrap_or((field, ""));
            (decode(key), decode(value))
        })
        .collect()
}

/// Opt-in synthetic transport for real producer tests. Default configurations
/// above still panic on every HTTP attempt. Strict ordering is the default;
/// unordered matching is only for explicitly concurrent, independently keyed calls.
#[derive(Clone, Debug)]
pub(crate) struct ScriptedHttp {
    expected: Arc<parking_lot::Mutex<VecDeque<ExpectedRequest>>>,
    unordered: bool,
    calls: Arc<AtomicUsize>,
    stats: Arc<TransportStats>,
}

#[derive(Debug, Default)]
struct TransportStats {
    active: AtomicUsize,
    peak_active: AtomicUsize,
    completed: AtomicUsize,
    cancelled: AtomicUsize,
    request_body_bytes: AtomicUsize,
    response_body_bytes: AtomicUsize,
    timing: parking_lot::Mutex<(Option<Instant>, Option<Duration>)>,
}

#[derive(serde::Serialize)]
pub(crate) struct TransportSnapshot {
    pub actual_http_attempts: usize,
    pub completed_http_attempts: usize,
    pub dropped_http_attempts: usize,
    pub active_http_attempts: usize,
    pub peak_active_http_attempts: usize,
    pub request_body_bytes: usize,
    pub response_body_bytes: usize,
    pub first_response_ms: Option<f64>,
}

struct AttemptGuard {
    stats: Arc<TransportStats>,
    completed: bool,
}

impl AttemptGuard {
    fn complete(&mut self, response_bytes: usize) {
        self.completed = true;
        self.stats.completed.fetch_add(1, Ordering::SeqCst);
        self.stats
            .response_body_bytes
            .fetch_add(response_bytes, Ordering::SeqCst);
        let mut timing = self.stats.timing.lock();
        if timing.1.is_none() {
            timing.1 = timing.0.map(|started| started.elapsed());
        }
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.stats.active.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.stats.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl ScriptedHttp {
    pub(crate) fn new(expected: Vec<ExpectedRequest>) -> Self {
        Self {
            expected: Arc::new(parking_lot::Mutex::new(expected.into())),
            unordered: false,
            calls: Arc::new(AtomicUsize::new(0)),
            stats: Arc::new(TransportStats::default()),
        }
    }

    pub(crate) fn unordered(mut self) -> Self {
        self.unordered = true;
        self
    }

    pub(crate) fn sdk_config(&self) -> SdkConfig {
        let connector = SharedHttpConnector::new(self.clone());
        SdkConfig::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(TEST_ENDPOINT)
            .credentials_provider(SharedCredentialsProvider::new(
                aws_credential_types::Credentials::new(
                    "CB_SYNTHETIC_ACCESS_KEY",
                    "CB_SYNTHETIC_SECRET_KEY",
                    Some("CB_SYNTHETIC_SESSION_TOKEN".into()),
                    None,
                    "cloud-burrito-producer-test-only",
                ),
            ))
            .identity_cache(IdentityCache::no_cache())
            .retry_config(RetryConfig::disabled())
            .timeout_config(TimeoutConfig::disabled())
            .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
            .http_client(http_client_fn(move |_, _| connector.clone()))
            .build()
    }

    pub(crate) fn context(
        &self,
        dir: &crate::test_support::TestDir,
        widget: &str,
        inputs: Value,
    ) -> crate::widgets::WidgetCtx {
        crate::widgets::WidgetCtx {
            runtime: crate::runtime::Runtime::for_test(dir.paths()),
            sdk: self.sdk_config(),
            account_id: "acct-producer-fixture".into(),
            region: "us-east-1".into(),
            widget_name: widget.into(),
            inputs,
            policy: crate::aws::policy::Policy::parse(
                "statements:\n  - effect: Allow\n    action: ['*']\n",
            )
            .map_err(|error| error.message),
            cli: None,
        }
    }

    pub(crate) fn assert_finished(&self) {
        assert!(
            self.expected.lock().is_empty(),
            "synthetic HTTP responses were not consumed"
        );
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Called after fixture/config construction, immediately before timing the
    /// producer. Counters describe this synthetic transport, not SDK intent.
    pub(crate) fn begin_measurement(&self) {
        assert_eq!(self.calls(), 0, "measurement must precede HTTP work");
        *self.stats.timing.lock() = (Some(Instant::now()), None);
    }

    pub(crate) fn snapshot(&self) -> TransportSnapshot {
        TransportSnapshot {
            actual_http_attempts: self.calls(),
            completed_http_attempts: self.stats.completed.load(Ordering::SeqCst),
            dropped_http_attempts: self.stats.cancelled.load(Ordering::SeqCst),
            active_http_attempts: self.stats.active.load(Ordering::SeqCst),
            peak_active_http_attempts: self.stats.peak_active.load(Ordering::SeqCst),
            request_body_bytes: self.stats.request_body_bytes.load(Ordering::SeqCst),
            response_body_bytes: self.stats.response_body_bytes.load(Ordering::SeqCst),
            first_response_ms: self
                .stats
                .timing
                .lock()
                .1
                .map(|elapsed| elapsed.as_secs_f64() * 1000.0),
        }
    }
}

impl HttpConnector for ScriptedHttp {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let route = request
            .uri()
            .strip_prefix(TEST_ENDPOINT)
            .filter(|route| route.starts_with('/'))
            .expect("synthetic HTTP request escaped its fixed invalid endpoint");
        let mut queue = self.expected.lock();
        let index = if self.unordered {
            queue
                .iter()
                .position(|expected| expected.matches(&request, route))
        } else {
            queue
                .front()
                .is_some_and(|expected| expected.matches(&request, route))
                .then_some(0)
        }
        .expect("unexpected synthetic HTTP operation or request fields");
        let expected = queue
            .remove(index)
            .expect("matched synthetic request exists");
        drop(queue);
        self.stats.request_body_bytes.fetch_add(
            request.body().bytes().map_or(0, <[u8]>::len),
            Ordering::SeqCst,
        );
        let active = self.stats.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.stats.peak_active.fetch_max(active, Ordering::SeqCst);
        let mut guard = AttemptGuard {
            stats: self.stats.clone(),
            completed: false,
        };
        let response_bytes = expected.response.len();
        let delay = expected.delay;
        let mut response = HttpResponse::new(
            expected
                .status
                .try_into()
                .expect("valid synthetic HTTP status"),
            SdkBody::from(expected.response),
        );
        response
            .headers_mut()
            .insert("content-type", expected.content_type);
        HttpConnectorFuture::new(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            guard.complete(response_bytes);
            Ok(response)
        })
    }
}

fn synthetic_request(uri: &str, target: &'static str, body: Value) -> HttpRequest {
    let mut request = HttpRequest::new(SdkBody::from(body.to_string()));
    request.set_uri(uri).unwrap();
    request.set_method("POST").unwrap();
    request.headers_mut().insert("x-amz-target", target);
    request
}

#[test]
fn scripted_transport_rejects_wrong_operation_fields_and_external_endpoints() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    for (uri, target, body) in [
        (
            "https://cloud-burrito-test.invalid/",
            "Logs_20140328.StartQuery",
            serde_json::json!({"queryId":"synthetic-query"}),
        ),
        (
            "https://cloud-burrito-test.invalid/",
            "Logs_20140328.GetQueryResults",
            serde_json::json!({"queryId":"unexpected-query"}),
        ),
        (
            "https://example.invalid/",
            "Logs_20140328.GetQueryResults",
            serde_json::json!({"queryId":"synthetic-query"}),
        ),
        (
            "https://cloud-burrito-test.invalid.other.invalid/",
            "Logs_20140328.GetQueryResults",
            serde_json::json!({"queryId":"synthetic-query"}),
        ),
    ] {
        let script = ScriptedHttp::new(vec![ExpectedRequest::json(
            "Logs_20140328.GetQueryResults",
            serde_json::json!({"queryId":"synthetic-query"}),
            serde_json::json!({"status":"Complete"}),
        )]);
        assert!(catch_unwind(AssertUnwindSafe(
            || script.call(synthetic_request(uri, target, body))
        ))
        .is_err());
        assert_eq!(script.calls(), 1);
        assert_eq!(script.expected.lock().len(), 1);
    }
}

#[tokio::test]
async fn scripted_transport_consumes_one_response_then_panics_on_an_extra_call() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let script = ScriptedHttp::new(vec![ExpectedRequest::json(
        "Logs_20140328.GetQueryResults",
        serde_json::json!({"queryId":"synthetic-query"}),
        serde_json::json!({"status":"Complete"}),
    )]);
    let request = || {
        synthetic_request(
            "https://cloud-burrito-test.invalid/",
            "Logs_20140328.GetQueryResults",
            serde_json::json!({"queryId":"synthetic-query"}),
        )
    };
    let result = script.call(request()).await.unwrap();
    assert_eq!(result.status().as_u16(), 200);
    script.assert_finished();
    assert!(catch_unwind(AssertUnwindSafe(|| script.call(request()))).is_err());
}

#[tokio::test]
async fn direct_credential_lookup_is_rejected() {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let counters = Arc::new(Counters::default());
    let config = sdk_config_with_counters(counters.clone());
    let provider = config.credentials_provider().expect("test provider is set");
    let outcome = AssertUnwindSafe(async { provider.provide_credentials().await })
        .catch_unwind()
        .await;
    assert!(outcome.is_err(), "unexpected credential lookup must panic");
    assert_eq!(counters.credentials.load(Ordering::SeqCst), 1);
    assert_eq!(counters.transport.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn direct_service_call_cannot_bypass_sdk_fences() {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let counters = Arc::new(Counters::default());
    let config = sdk_config_with_counters(counters.clone());
    let outcome = AssertUnwindSafe(async {
        let client = aws_sdk_sts::Client::new(&config);
        client.get_caller_identity().send().await
    })
    .catch_unwind()
    .await;
    assert!(outcome.is_err(), "unexpected SDK boundary work must panic");
    // Client/operation setup order is not part of this application's contract.
    // Either fence is sufficient, but a normal SDK error without a fence is not.
    assert_eq!(
        counters.credentials.load(Ordering::SeqCst) + counters.transport.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn synthetic_credentials_cannot_bypass_the_transport_fence() {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let counters = Arc::new(Counters::default());
    // These intentionally non-secret values exist only to reach the HTTP fence.
    let credentials = aws_credential_types::Credentials::new(
        "CB_SYNTHETIC_ACCESS_KEY",
        "CB_SYNTHETIC_SECRET_KEY",
        Some("CB_SYNTHETIC_SESSION_TOKEN".to_string()),
        None,
        "cloud-burrito-test-only",
    );
    let config = config_with_provider(
        SharedCredentialsProvider::new(credentials),
        counters.clone(),
    );
    let outcome = AssertUnwindSafe(async {
        let client = aws_sdk_sts::Client::new(&config);
        client.get_caller_identity().send().await
    })
    .catch_unwind()
    .await;
    assert!(outcome.is_err(), "unexpected HTTP transport must panic");
    assert_eq!(counters.credentials.load(Ordering::SeqCst), 0);
    assert_eq!(counters.transport.load(Ordering::SeqCst), 1);
}
