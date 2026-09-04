//! Test-only AWS configuration that rejects unplanned provider and HTTP work.
//!
//! Command tests control credential resolution and caller identity through the
//! injected backend. This configuration is a second fence for code that builds
//! a service client directly from `WidgetCtx::sdk` instead of that backend.

use aws_config::{identity::IdentityCache, retry::RetryConfig, BehaviorVersion, Region, SdkConfig};
use aws_credential_types::provider::{future, ProvideCredentials, SharedCredentialsProvider};
use aws_smithy_runtime_api::client::{
    http::http_client_fn, stalled_stream_protection::StalledStreamProtectionConfig,
};
use aws_smithy_types::timeout::TimeoutConfig;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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
