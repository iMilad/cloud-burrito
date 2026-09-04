//! Injectable app storage, explicit SSO/STS boundaries and process execution.
//! Production never loads the AWS default credential/configuration chain.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_config::{identity::IdentityCache, BehaviorVersion, Region, SdkConfig};
use aws_credential_types::provider::{
    error::CredentialsError, future, token::ProvideToken, ProvideCredentials,
    SharedCredentialsProvider,
};
use aws_credential_types::Credentials;
use aws_smithy_types::{date_time::Format, DateTime};
use futures::future::BoxFuture;
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::aws::config_file::{self, SsoProfileSelection, SsoProfileSnapshot};
use crate::aws::AwsContext;
use crate::paths::AppPaths;
use crate::process::{NativeProcessRunner, ProcessRunner};

pub trait Clock: Send + Sync {
    fn now_epoch(&self) -> f64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch(&self) -> f64 {
        crate::audit::now_epoch()
    }
}

pub type CallerIdentity = aws_sdk_sts::operation::get_caller_identity::GetCallerIdentityOutput;

pub trait AwsBackend: Send + Sync {
    fn inspect_config(&self, path: &str) -> Value;
    fn snapshot_sso(&self, ctx: &AwsContext) -> Result<SsoProfileSnapshot, String>;
    fn resolve_sso<'a>(
        &'a self,
        snapshot: &'a SsoProfileSnapshot,
    ) -> BoxFuture<'a, Result<Credentials, String>>;
    fn caller_identity<'a>(
        &'a self,
        config: &'a SdkConfig,
    ) -> BoxFuture<'a, Result<CallerIdentity, String>>;
}

struct NativeAwsBackend;

fn require_live_aws() {
    #[cfg(test)]
    panic!("unexpected live AWS/provider or personal configuration access in a test");
}

/// No loader, profile, endpoint override, service configuration, credentials or
/// environment defaults. Service clients use their built-in regional endpoints.
fn safe_sdk_config(region: &str) -> SdkConfig {
    let mut builder = SdkConfig::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.to_string()));
    builder.set_time_source(Some(Default::default()));
    builder.build()
}

fn bounded_read(path: &std::path::Path, limit: u64) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|_| "selected AWS file could not be read".to_string())?;
    let mut content = String::new();
    file.take(limit + 1)
        .read_to_string(&mut content)
        .map_err(|_| "selected AWS file is not valid UTF-8".to_string())?;
    if content.len() as u64 > limit {
        return Err("selected AWS file exceeds the supported size".into());
    }
    Ok(content)
}

/// Match the locked SDK's token-cache home selection, including Windows HOME
/// precedence. The injected lookup keeps path tests independent of real homes.
fn sdk_token_home(windows: bool, get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get("HOME").or_else(|| {
        if windows {
            get("USERPROFILE").or_else(|| {
                let mut drive = get("HOMEDRIVE")?;
                drive.push_str(&get("HOMEPATH")?);
                Some(drive)
            })
        } else {
            None
        }
    })
}

fn named_sso_cache_path(session: &str) -> Result<PathBuf, String> {
    let home = sdk_token_home(cfg!(windows), |name| std::env::var(name).ok())
        .filter(|home| !home.is_empty())
        .ok_or_else(|| "SSO token cache home is unavailable".to_string())?;
    let hash = hex::encode(Sha1::digest(session.as_bytes()));
    Ok(PathBuf::from(home)
        .join(".aws/sso/cache")
        .join(format!("{hash}.json")))
}

/// Token expiry is deliberately not checked here: a named session may renew an
/// expired access token using the registration in this same cache document.
fn validate_sso_cache_metadata(text: &str, snapshot: &SsoProfileSnapshot) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|_| "SSO token cache is not valid JSON".to_string())?;
    if value.get("startUrl").and_then(Value::as_str) != Some(snapshot.start_url.as_str())
        || value.get("region").and_then(Value::as_str) != Some(snapshot.sso_region.as_str())
    {
        return Err("SSO token cache does not match the selected profile; sign in again".into());
    }
    Ok(value)
}

/// Legacy profiles have no refresh registration. The selected start URL is the
/// cache key; expired tokens use the existing external `aws sso login` workflow.
fn legacy_sso_token(snapshot: &SsoProfileSnapshot) -> Result<String, String> {
    let path = config_file::sso_cache_path(snapshot.token_cache_key());
    let text = bounded_read(&path, 128 * 1024)?;
    let value = validate_sso_cache_metadata(&text, snapshot)?;
    let expiry = value
        .get("expiresAt")
        .and_then(Value::as_str)
        .ok_or_else(|| "legacy SSO token has no expiry".to_string())?;
    let normalized = expiry.strip_suffix("UTC").map(|date| format!("{date}Z"));
    let expires_at = DateTime::from_str(normalized.as_deref().unwrap_or(expiry), Format::DateTime)
        .ok()
        .and_then(|date| SystemTime::try_from(date).ok())
        .ok_or_else(|| "legacy SSO token expiry is invalid".to_string())?;
    if expires_at <= SystemTime::now() {
        return Err("legacy SSO token expired; sign in again".into());
    }
    value
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "legacy SSO cache has no access token".to_string())
}

impl AwsBackend for NativeAwsBackend {
    fn inspect_config(&self, path: &str) -> Value {
        require_live_aws();
        config_file::inspect(path)
    }

    fn snapshot_sso(&self, ctx: &AwsContext) -> Result<SsoProfileSnapshot, String> {
        require_live_aws();
        let config_path = config_file::expand(&ctx.aws_config_path);
        let text = bounded_read(&config_path, 2 * 1024 * 1024)?;
        SsoProfileSnapshot::parse(
            &text,
            &SsoProfileSelection {
                config_path,
                profile: ctx.profile.clone(),
                expected_account_id: ctx.account_id.clone(),
                requested_region: ctx.region.clone(),
                session_override: ctx.sso_session_name.clone(),
                settings_revision: ctx.settings_revision,
            },
        )
    }

    fn resolve_sso<'a>(
        &'a self,
        snapshot: &'a SsoProfileSnapshot,
    ) -> BoxFuture<'a, Result<Credentials, String>> {
        Box::pin(async move {
            require_live_aws();
            let config = safe_sdk_config(&snapshot.sso_region);
            let access_token = if let Some(session) = &snapshot.session_name {
                // The SDK selects this file by session name without checking
                // its origin metadata. Reject a repointed session before any
                // provider refresh or SSO request can use the old cache.
                let cache_path = named_sso_cache_path(session)?;
                validate_sso_cache_metadata(&bounded_read(&cache_path, 128 * 1024)?, snapshot)?;
                // Providing the entire safe SDK config bypasses load_defaults.
                // The provider reads/writes only the normal named SSO token
                // cache and preserves supported OIDC refresh behavior.
                let provider = aws_config::sso::SsoTokenProvider::builder()
                    .configure(&config)
                    .session_name(session)
                    .start_url(&snapshot.start_url)
                    .region(Region::new(snapshot.sso_region.clone()))
                    .build()
                    .await;
                let token = provider.provide_token().await.map_err(|_| {
                    "SSO cached token could not be loaded or refreshed; sign in again".to_string()
                })?;
                if token
                    .expiration()
                    .is_none_or(|expiry| expiry <= SystemTime::now())
                {
                    return Err(
                        "SSO token expired and could not be refreshed; sign in again".into(),
                    );
                }
                token.token().to_string()
            } else {
                legacy_sso_token(snapshot)?
            };
            let response = aws_sdk_sso::Client::new(&config)
                .get_role_credentials()
                .account_id(&snapshot.account_id)
                .role_name(&snapshot.role_name)
                .access_token(access_token)
                .send()
                .await
                .map_err(crate::widgets::err_msg)?;
            let role = response
                .role_credentials()
                .ok_or_else(|| "SSO returned no role credentials".to_string())?;
            let field = |value: Option<&str>| {
                value
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| "SSO returned incomplete role credentials".to_string())
            };
            let expiry = SystemTime::try_from(DateTime::from_millis(role.expiration()))
                .map_err(|_| "SSO returned an invalid credential expiry".to_string())?;
            if expiry <= SystemTime::now() {
                return Err("SSO returned expired role credentials".into());
            }
            Ok(Credentials::builder()
                .access_key_id(field(role.access_key_id())?)
                .secret_access_key(field(role.secret_access_key())?)
                .session_token(field(role.session_token())?)
                .expiry(expiry)
                .account_id(snapshot.account_id.clone())
                .provider_name("cloud-burrito-explicit-sso")
                .build())
        })
    }

    fn caller_identity<'a>(
        &'a self,
        config: &'a SdkConfig,
    ) -> BoxFuture<'a, Result<CallerIdentity, String>> {
        Box::pin(async move {
            require_live_aws();
            aws_sdk_sts::Client::new(config)
                .get_caller_identity()
                .send()
                .await
                .map_err(crate::widgets::err_msg)
        })
    }
}

pub(crate) fn unexpired(expires_at: SystemTime, now: f64, margin: f64) -> bool {
    now.is_finite()
        && now >= 0.0
        && expires_at
            .duration_since(UNIX_EPOCH)
            .is_ok_and(|expiry| expiry.as_secs_f64() > now + margin)
}

/// A fixed credential value with an expiry fence; no refresh/fallback methods.
struct FrozenCredentials {
    credentials: Credentials,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for FrozenCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrozenCredentials(<redacted>)")
    }
}

impl ProvideCredentials for FrozenCredentials {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async move {
            match self.credentials.expiry() {
                Some(expiry) if unexpired(expiry, self.clock.now_epoch(), 0.0) => {
                    Ok(self.credentials.clone())
                }
                _ => Err(CredentialsError::provider_error(
                    "verified SSO credential snapshot expired; reverify the context",
                )),
            }
        })
    }
}

pub(crate) fn fixed_sdk_config(
    credentials: Credentials,
    region: &str,
    clock: Arc<dyn Clock>,
) -> SdkConfig {
    #[cfg(not(test))]
    let builder = safe_sdk_config(region).into_builder();
    #[cfg(test)]
    let builder = crate::test_aws::sdk_config().into_builder();
    builder
        .region(Region::new(region.to_string()))
        .credentials_provider(SharedCredentialsProvider::new(FrozenCredentials {
            credentials,
            clock,
        }))
        .identity_cache(IdentityCache::no_cache())
        .build()
}

#[derive(Clone)]
pub struct Runtime {
    pub paths: AppPaths,
    pub aws: Arc<dyn AwsBackend>,
    pub process: Arc<dyn ProcessRunner>,
    pub clock: Arc<dyn Clock>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            paths: AppPaths::default(),
            aws: Arc::new(NativeAwsBackend),
            process: Arc::new(NativeProcessRunner),
            clock: Arc::new(SystemClock),
        }
    }
}

impl Runtime {
    pub fn audit(&self, entry: Value) {
        crate::audit::append(&self.paths, entry, self.clock.now_epoch());
    }

    #[cfg(test)]
    pub fn for_test(paths: AppPaths) -> Self {
        Self {
            paths,
            aws: Arc::new(NativeAwsBackend),
            process: Arc::new(NativeProcessRunner),
            clock: Arc::new(FixedClock(1_700_000_000.0)),
        }
    }
}

#[cfg(test)]
pub struct FixedClock(pub f64);

#[cfg(test)]
impl Clock for FixedClock {
    fn now_epoch(&self) -> f64 {
        self.0
    }
}

#[cfg(test)]
pub fn test_sdk_config() -> SdkConfig {
    crate::test_aws::sdk_config()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_snapshot(session: Option<&str>) -> SsoProfileSnapshot {
        SsoProfileSnapshot {
            config_path: PathBuf::from("synthetic-config.ini"),
            profile: "DemoReadOnly".into(),
            account_id: "acct-demo-fixture".into(),
            role_name: "DemoReadOnly".into(),
            region: "eu-west-1".into(),
            sso_region: "us-east-1".into(),
            start_url: "https://example.awsapps.com/start".into(),
            session_name: session.map(str::to_string),
            registration_scopes: None,
            settings_revision: 1,
        }
    }

    #[test]
    fn cache_metadata_binding_accepts_both_sso_forms_without_blocking_renewal() {
        let text = serde_json::json!({
            "startUrl": "https://example.awsapps.com/start",
            "region": "us-east-1",
            "accessToken": "CB_SYNTHETIC_EXPIRED_TOKEN",
            "expiresAt": "2000-01-01T00:00:00Z",
            "refreshToken": "CB_SYNTHETIC_REFRESH_TOKEN",
            "clientId": "CB_SYNTHETIC_CLIENT_ID",
            "registrationExpiresAt": "2099-01-01T00:00:00Z"
        })
        .to_string();
        for session in [None, Some("demo-session")] {
            let value = validate_sso_cache_metadata(&text, &cache_snapshot(session)).unwrap();
            assert_eq!(value["expiresAt"], "2000-01-01T00:00:00Z");
            assert_eq!(value["refreshToken"], "CB_SYNTHETIC_REFRESH_TOKEN");
        }
    }

    #[test]
    fn cache_metadata_binding_rejects_repointed_missing_or_invalid_origins() {
        let matching = serde_json::json!({
            "startUrl": "https://example.awsapps.com/start",
            "region": "us-east-1",
            "accessToken": "CB_SYNTHETIC_TOKEN_MUST_NOT_APPEAR_IN_ERRORS"
        });
        for session in [None, Some("demo-session")] {
            let snapshot = cache_snapshot(session);
            for (field, replacement) in [
                (
                    "startUrl",
                    serde_json::json!("https://other.example.invalid/start"),
                ),
                ("startUrl", Value::Null),
                ("startUrl", serde_json::json!(42)),
                ("region", serde_json::json!("eu-west-1")),
                ("region", Value::Null),
                ("region", serde_json::json!([])),
            ] {
                let mut value = matching.clone();
                value[field] = replacement;
                let error = validate_sso_cache_metadata(&value.to_string(), &snapshot).unwrap_err();
                assert!(error.contains("does not match"));
                assert!(!error.contains("CB_SYNTHETIC"));
            }
            for text in ["{}", "[]", "null", "CB_SYNTHETIC_INVALID_JSON"] {
                let error = validate_sso_cache_metadata(text, &snapshot).unwrap_err();
                assert!(!error.contains("CB_SYNTHETIC"));
            }
        }
    }

    #[test]
    fn named_token_home_follows_sdk_precedence_without_reading_environment() {
        let lookup = |entries: &[(&str, &str)], windows| {
            sdk_token_home(windows, |name| {
                entries
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            })
        };
        let all = [
            ("HOME", "synthetic-home"),
            ("USERPROFILE", "synthetic-profile"),
            ("HOMEDRIVE", "X:"),
            ("HOMEPATH", "/synthetic-user"),
        ];
        assert_eq!(lookup(&all, true).as_deref(), Some("synthetic-home"));
        assert_eq!(lookup(&all, false).as_deref(), Some("synthetic-home"));
        assert_eq!(
            lookup(&all[1..], true).as_deref(),
            Some("synthetic-profile")
        );
        assert_eq!(
            lookup(&all[2..], true).as_deref(),
            Some("X:/synthetic-user")
        );
        assert!(lookup(&all[1..], false).is_none());
        assert!(lookup(&all[2..3], true).is_none());
        assert!(lookup(&[], true).is_none());
    }

    #[test]
    #[should_panic(expected = "unexpected live AWS/provider or personal configuration access")]
    fn native_config_boundary_rejects_personal_configuration_access_in_tests() {
        NativeAwsBackend.inspect_config("synthetic-config.ini");
    }

    #[tokio::test]
    #[should_panic(expected = "unexpected live AWS/provider or personal configuration access")]
    async fn native_sdk_boundary_rejects_provider_discovery_in_tests() {
        let dir = crate::test_support::TestDir::new();
        let context = AwsContext::new(
            "demo-a".into(),
            "acct-demo-fixture".into(),
            "us-east-1".into(),
            None,
            "synthetic-config.ini".into(),
            Runtime::for_test(dir.paths()),
        );
        let _ = context.verified_session().await;
    }

    #[test]
    fn explicit_sdk_config_has_no_ambient_endpoint_or_credential_sources() {
        let config = safe_sdk_config("us-east-1");
        assert!(config.service_config().is_none());
        assert!(config.endpoint_url().is_none());
        assert!(config.credentials_provider().is_none());
        assert!(config.time_source().is_some());
        assert_eq!(config.region().unwrap().as_ref(), "us-east-1");
    }

    #[tokio::test]
    async fn fixed_credentials_preserve_exact_values_and_fail_after_expiry() {
        use std::sync::atomic::{AtomicU64, Ordering};
        struct MutableClock(AtomicU64);
        impl Clock for MutableClock {
            fn now_epoch(&self) -> f64 {
                self.0.load(Ordering::SeqCst) as f64
            }
        }
        let clock = Arc::new(MutableClock(AtomicU64::new(1000)));
        let credentials = Credentials::new(
            "CB_SYNTHETIC_ACCESS_KEY",
            "CB_SYNTHETIC_SECRET_KEY",
            Some("CB_SYNTHETIC_SESSION_TOKEN".into()),
            Some(UNIX_EPOCH + std::time::Duration::from_secs(2000)),
            "test-only",
        );
        let config = fixed_sdk_config(credentials, "us-east-1", clock.clone());
        let provider = config.credentials_provider().unwrap();
        let first = provider.provide_credentials().await.unwrap();
        assert_eq!(first.access_key_id(), "CB_SYNTHETIC_ACCESS_KEY");
        assert_eq!(first.session_token(), Some("CB_SYNTHETIC_SESSION_TOKEN"));
        clock.0.store(2000, Ordering::SeqCst);
        assert!(provider.provide_credentials().await.is_err());
        assert!(!unexpired(UNIX_EPOCH, f64::NAN, 0.0));
    }
}
