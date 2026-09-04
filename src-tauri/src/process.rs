//! Constrained AWS CLI v2 execution with frozen credentials and owned cleanup.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;

const MAX_STDOUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 256 * 1024;
const MAX_RUN_TIME: Duration = Duration::from_secs(30);
const CANCELLED: &str = "AWS CLI request cancelled";
const TIMED_OUT: &str = "AWS CLI deadline reached";
pub(crate) const CLEANUP_FAILED: &str =
    "AWS CLI cleanup failed; process exit could not be confirmed";
static NEXT_HOME: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct ProcessCancellation(watch::Sender<bool>);

impl ProcessCancellation {
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for ProcessCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProcessCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct CliRequest {
    pub argv: Vec<String>,
    pub region: String,
    pub credentials: Credentials,
    pub cancellation: ProcessCancellation,
}

impl std::fmt::Debug for CliRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CliRequest { argv: <redacted>, credentials: <redacted>, .. }")
    }
}

pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
}

impl std::fmt::Debug for ProcessOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .field("success", &self.success)
            .finish()
    }
}

pub trait ProcessRunner: Send + Sync {
    fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>>;
}

pub struct NativeProcessRunner;

impl ProcessRunner for NativeProcessRunner {
    fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
        #[cfg(test)]
        {
            let _ = request;
            Box::pin(async { panic!("native process execution is forbidden in tests") })
        }
        #[cfg(not(test))]
        {
            Box::pin(run_native(request))
        }
    }
}

/// Validate the complete frozen credential set before discovery or spawning.
/// A running child may use this identity only up to its captured expiry.
fn request_budget(request: &CliRequest, now: SystemTime) -> Result<Duration, String> {
    if request.cancellation.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if request.credentials.access_key_id().trim().is_empty()
        || request.credentials.secret_access_key().trim().is_empty()
        || request
            .credentials
            .session_token()
            .is_none_or(|token| token.trim().is_empty())
    {
        return Err("verified AWS CLI credentials are incomplete".into());
    }
    let remaining = request
        .credentials
        .expiry()
        .and_then(|expiry| expiry.duration_since(now).ok())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| "verified AWS CLI credentials expired or have no expiry".to_string())?;
    Ok(remaining.min(MAX_RUN_TIME))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Platform {
    Unix,
    Windows,
}

impl Platform {
    fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
    fn null_file(self) -> &'static str {
        if self == Self::Windows {
            "NUL"
        } else {
            "/dev/null"
        }
    }
}

fn env_value<'a>(
    ambient: &'a [(OsString, OsString)],
    name: &str,
    platform: Platform,
) -> Option<&'a OsStr> {
    ambient
        .iter()
        .find(|(key, _)| {
            key.to_str().is_some_and(|key| {
                if platform == Platform::Windows {
                    key.eq_ignore_ascii_case(name)
                } else {
                    key == name
                }
            })
        })
        .map(|(_, value)| value.as_os_str())
}

fn unambiguous_env<'a>(
    ambient: &'a [(OsString, OsString)],
    name: &str,
    platform: Platform,
) -> Result<Option<&'a OsStr>, String> {
    let mut found = None;
    for (key, value) in ambient {
        if key.to_str().is_some_and(|key| {
            if platform == Platform::Windows {
                key.eq_ignore_ascii_case(name)
            } else {
                key == name
            }
        }) {
            if found.is_some_and(|previous| previous != value.as_os_str()) {
                return Err("AWS CLI environment has conflicting runtime settings".into());
            }
            found = Some(value.as_os_str());
        }
    }
    Ok(found)
}

/// Native CLI v2 includes its runtime. No inherited PATH, loader/module search
/// path, profile, endpoint, plugin configuration or ambient credentials survive.
fn child_environment(
    request: &CliRequest,
    ambient: &[(OsString, OsString)],
    home: &Path,
    platform: Platform,
) -> Result<Vec<(OsString, OsString)>, String> {
    let mut environment = Vec::new();
    // Unix follows the conventional lowercase precedence and emits one key;
    // Windows names are case-insensitive, so conflicting aliases fail closed.
    for lower in ["https_proxy", "http_proxy", "all_proxy", "no_proxy"] {
        let upper = lower.to_ascii_uppercase();
        let (name, value) = if platform == Platform::Windows {
            (upper.as_str(), unambiguous_env(ambient, &upper, platform)?)
        } else {
            (
                lower,
                unambiguous_env(ambient, lower, platform)?
                    .or(unambiguous_env(ambient, &upper, platform)?),
            )
        };
        if let Some(value) = value {
            environment.push((name.into(), value.to_os_string()));
        }
    }
    for name in [
        "AWS_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = unambiguous_env(ambient, name, platform)? {
            if !Path::new(value).is_absolute() {
                return Err("AWS CLI certificate paths must be absolute".into());
            }
            environment.push((name.into(), value.to_os_string()));
        }
    }
    if platform == Platform::Windows {
        for name in ["SystemRoot", "WINDIR"] {
            if let Some(value) = unambiguous_env(ambient, name, platform)? {
                environment.push((name.into(), value.to_os_string()));
            }
        }
    }
    for name in ["HOME", "USERPROFILE", "TMPDIR", "TMP", "TEMP"] {
        environment.push((name.into(), home.as_os_str().to_os_string()));
    }
    for name in [
        "AWS_CONFIG_FILE",
        "AWS_SHARED_CREDENTIALS_FILE",
        "BOTO_CONFIG",
        "BOTO_PATH",
    ] {
        environment.push((name.into(), platform.null_file().into()));
    }
    for (name, value) in [
        ("AWS_ACCESS_KEY_ID", request.credentials.access_key_id()),
        (
            "AWS_SECRET_ACCESS_KEY",
            request.credentials.secret_access_key(),
        ),
        (
            "AWS_SESSION_TOKEN",
            request.credentials.session_token().unwrap_or(""),
        ),
        ("AWS_REGION", request.region.as_str()),
        ("AWS_DEFAULT_REGION", request.region.as_str()),
        ("AWS_EC2_METADATA_DISABLED", "true"),
        ("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "true"),
        ("AWS_CLI_AUTO_PROMPT", "off"),
        ("AWS_PAGER", ""),
        ("PYTHONNOUSERSITE", "1"),
        ("PYTHONIOENCODING", "utf-8"),
        ("LANG", "C"),
        ("LC_ALL", "C"),
    ] {
        environment.push((name.into(), value.into()));
    }
    Ok(environment)
}

#[derive(Debug, PartialEq, Eq)]
struct LaunchSpec {
    program: PathBuf,
    prefix: Vec<OsString>,
}

fn configured_command(
    launch: &LaunchSpec,
    request: &CliRequest,
    ambient: &[(OsString, OsString)],
    home: &Path,
) -> Result<Command, String> {
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.prefix)
        .args(&request.argv)
        .env_clear()
        .envs(child_environment(request, ambient, home, Platform::host())?)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

/// Inspect native image headers without executing a version probe. A trusted
/// local installation remains required; image type is not publisher attestation.
fn native_executable(candidate: &Path, platform: Platform) -> Option<PathBuf> {
    if !candidate.is_absolute() {
        return None;
    }
    if platform == Platform::Windows
        && !candidate
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let path = candidate.canonicalize().ok()?;
    if platform == Platform::Windows
        && !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    if platform == Platform::Unix {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    let mut header = [0; 4];
    std::fs::File::open(&path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    let valid = if platform == Platform::Windows {
        header[..2] == *b"MZ"
    } else {
        header == *b"\x7fELF"
            || matches!(
                header,
                [0xfe, 0xed, 0xfa, 0xce]
                    | [0xce, 0xfa, 0xed, 0xfe]
                    | [0xfe, 0xed, 0xfa, 0xcf]
                    | [0xcf, 0xfa, 0xed, 0xfe]
                    | [0xca, 0xfe, 0xba, 0xbe]
                    | [0xbe, 0xba, 0xfe, 0xca]
                    | [0xca, 0xfe, 0xba, 0xbf]
                    | [0xbf, 0xba, 0xfe, 0xca]
            )
    };
    valid.then_some(path)
}

/// Homebrew CLI v2 uses an absolute-Python wrapper. Isolated Python mode avoids
/// ambient import paths, while retaining the shebang interpreter's original
/// absolute path preserves its virtual environment. No shell/env wrappers.
fn launch_spec(candidate: &Path, platform: Platform) -> Option<LaunchSpec> {
    if let Some(program) = native_executable(candidate, platform) {
        return Some(LaunchSpec {
            program,
            prefix: Vec::new(),
        });
    }
    if platform != Platform::Unix || !candidate.is_absolute() {
        return None;
    }
    let wrapper = candidate.canonicalize().ok()?;
    let metadata = wrapper.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&wrapper)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    if end > 4096 {
        return None;
    }
    let shebang = std::str::from_utf8(&bytes[..end])
        .ok()?
        .strip_prefix("#!")?
        .trim();
    let mut words = shebang.split_whitespace();
    let interpreter = Path::new(words.next()?);
    if words.next().is_some() || !interpreter.is_absolute() {
        return None;
    }
    let name = interpreter.file_name()?.to_str()?;
    let python = matches!(name, "python" | "python3")
        || name.strip_prefix("python3.").is_some_and(|version| {
            !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
        });
    if !python {
        return None;
    }
    native_executable(interpreter, Platform::Unix)?;
    Some(LaunchSpec {
        program: interpreter.to_path_buf(),
        prefix: vec!["-I".into(), wrapper.into_os_string()],
    })
}

fn discover_binary(ambient: &[(OsString, OsString)], platform: Platform) -> Option<LaunchSpec> {
    if let Some(path) = env_value(ambient, "PATH", platform) {
        for directory in std::env::split_paths(path).filter(|directory| directory.is_absolute()) {
            let candidate = directory.join(if platform == Platform::Windows {
                "aws.exe"
            } else {
                "aws"
            });
            if let Some(binary) = launch_spec(&candidate, platform) {
                return Some(binary);
            }
        }
    }
    if platform == Platform::Windows {
        for name in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(directory) = env_value(ambient, name, platform) {
                if let Some(binary) = launch_spec(
                    &Path::new(directory).join("Amazon/AWSCLIV2/aws.exe"),
                    platform,
                ) {
                    return Some(binary);
                }
            }
        }
    } else {
        for candidate in [
            "/opt/homebrew/bin/aws",
            "/usr/local/bin/aws",
            "/usr/bin/aws",
        ] {
            if let Some(binary) = launch_spec(Path::new(candidate), platform) {
                return Some(binary);
            }
        }
    }
    None
}

struct IsolatedHome(PathBuf);

impl IsolatedHome {
    fn create_in(parent: &Path) -> Result<Self, String> {
        let parent = parent
            .canonicalize()
            .map_err(|_| "AWS CLI temporary directory is unavailable".to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..32 {
            let sequence = NEXT_HOME.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                "cloud-burrito-cli-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("AWS CLI temporary directory could not be created".into()),
            }
        }
        Err("AWS CLI temporary directory could not be allocated".into())
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        // Only the exact directory created by this owner, and only if empty.
        // No credentials are written here; do not recursively delete child files.
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[derive(Clone)]
struct ChildExit {
    success: bool,
}

trait ChildControl: Send + 'static {
    fn start_kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> BoxFuture<'_, io::Result<ChildExit>>;
}

#[cfg(not(test))]
impl ChildControl for tokio::process::Child {
    fn start_kill(&mut self) -> io::Result<()> {
        tokio::process::Child::start_kill(self)
    }
    fn wait(&mut self) -> BoxFuture<'_, io::Result<ChildExit>> {
        Box::pin(async move {
            let status = tokio::process::Child::wait(self).await?;
            Ok(ChildExit {
                success: status.success(),
            })
        })
    }
}

#[derive(Clone, Copy)]
struct OutputLimits {
    stdout: usize,
    stderr: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            stdout: MAX_STDOUT_BYTES,
            stderr: MAX_STDERR_BYTES,
        }
    }
}

async fn read_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    maximum: usize,
    message: &'static str,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(maximum.min(8192));
    let mut buffer = [0u8; 8192];
    loop {
        // At the boundary, read one extra byte solely to distinguish exact-cap
        // EOF from overflow. The extra byte is never retained in output.
        let available = buffer
            .len()
            .min(maximum.saturating_sub(output.len()).saturating_add(1));
        let count = reader
            .read(&mut buffer[..available])
            .await
            .map_err(|_| "AWS CLI output could not be read".to_string())?;
        if count == 0 {
            return Ok(output);
        }
        if count > maximum - output.len() {
            return Err(message.into());
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

async fn supervise<C, O, E>(
    mut child: C,
    stdout: O,
    stderr: E,
    cancellation: ProcessCancellation,
    deadline: BoxFuture<'static, ()>,
    limits: OutputLimits,
) -> Result<ProcessOutput, String>
where
    C: ChildControl,
    O: AsyncRead + Unpin + Send + 'static,
    E: AsyncRead + Unpin + Send + 'static,
{
    let result = {
        let collect = async {
            let (stdout, stderr, status) = tokio::try_join!(
                read_capped(
                    stdout,
                    limits.stdout,
                    "AWS CLI stdout exceeded its 2 MiB limit"
                ),
                read_capped(
                    stderr,
                    limits.stderr,
                    "AWS CLI stderr exceeded its 256 KiB limit"
                ),
                async {
                    child
                        .wait()
                        .await
                        .map_err(|_| "AWS CLI exit status could not be read".to_string())
                },
            )?;
            Ok(ProcessOutput {
                stdout,
                stderr,
                success: status.success,
            })
        };
        tokio::pin!(collect, deadline);
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(CANCELLED.to_string()),
            _ = &mut deadline => Err(TIMED_OUT.to_string()),
            result = &mut collect => result,
        }
    };
    if result.is_err() {
        // Even if kill reports an error (including an exit race), always await
        // wait. Returning without confirmed exit would conceal cleanup failure.
        let _ = child.start_kill();
        if child.wait().await.is_err() {
            return Err(CLEANUP_FAILED.into());
        }
    }
    result
}

struct CancelOnDrop(Option<ProcessCancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.0 {
            cancellation.cancel();
        }
    }
}

/// The owner task outlives an abandoned caller and retains the child and home
/// until kill/wait cleanup completes. The caller can cancel and await this same
/// future to obtain confirmation; dropping it still initiates cleanup.
async fn supervise_owned<C, O, E>(
    child: C,
    stdout: O,
    stderr: E,
    cancellation: ProcessCancellation,
    deadline: BoxFuture<'static, ()>,
    limits: OutputLimits,
    home: Option<IsolatedHome>,
) -> Result<ProcessOutput, String>
where
    C: ChildControl,
    O: AsyncRead + Unpin + Send + 'static,
    E: AsyncRead + Unpin + Send + 'static,
{
    let mut guard = CancelOnDrop(Some(cancellation.clone()));
    let owner = tokio::spawn(async move {
        let result = supervise(child, stdout, stderr, cancellation, deadline, limits).await;
        drop(home);
        result
    });
    let result = owner.await.map_err(|_| CLEANUP_FAILED.to_string());
    guard.0 = None;
    result?
}

#[cfg(not(test))]
async fn run_native(request: CliRequest) -> Result<ProcessOutput, String> {
    let budget = request_budget(&request, SystemTime::now())?;
    let expires = tokio::time::Instant::now() + budget;
    let ambient: Vec<_> = std::env::vars_os().collect();
    let binary = discover_binary(&ambient, Platform::host())
        .ok_or_else(|| "AWS CLI v2 executable not found; use a native install or a supported absolute-Python wrapper".to_string())?;
    let home = IsolatedHome::create_in(&std::env::temp_dir())?;
    // Repeat both checks immediately before the sole process-spawn boundary.
    request_budget(&request, SystemTime::now())?;
    if tokio::time::Instant::now() >= expires {
        return Err(TIMED_OUT.into());
    }
    let mut child = configured_command(&binary, &request, &ambient, &home.0)?
        .spawn()
        .map_err(|_| "AWS CLI process could not be started".to_string())?;
    // Piped stdout/stderr are set above; missing handles still enter owned
    // cleanup rather than dropping an unobserved child.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    match (stdout, stderr) {
        (Some(stdout), Some(stderr)) => {
            supervise_owned(
                child,
                stdout,
                stderr,
                request.cancellation,
                Box::pin(async move { tokio::time::sleep_until(expires).await }),
                OutputLimits::default(),
                Some(home),
            )
            .await
        }
        _ => {
            request.cancellation.cancel();
            let result = supervise_owned(
                child,
                tokio::io::empty(),
                tokio::io::empty(),
                request.cancellation,
                Box::pin(std::future::pending()),
                OutputLimits::default(),
                Some(home),
            )
            .await;
            match result {
                Err(error) if error != CANCELLED => Err(error),
                _ => Err("AWS CLI output pipes could not be opened".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncWriteExt, ReadBuf};
    use tokio::sync::{oneshot, Notify};

    fn request() -> CliRequest {
        CliRequest {
            argv: vec![
                "sts".into(),
                "get-caller-identity".into(),
                "--query".into(),
                "CB_PRIVATE_QUERY".into(),
            ],
            region: "eu-west-1".into(),
            credentials: Credentials::new(
                "CB_SYNTHETIC_ACCESS",
                "CB_SYNTHETIC_SECRET",
                Some("CB_SYNTHETIC_TOKEN".into()),
                Some(UNIX_EPOCH + Duration::from_secs(5000)),
                "process-test-only",
            ),
            cancellation: ProcessCancellation::new(),
        }
    }

    #[tokio::test]
    #[should_panic(expected = "native process execution is forbidden in tests")]
    async fn native_runner_rejects_execution_before_discovery() {
        NativeProcessRunner.run(request()).await.unwrap();
    }

    #[test]
    fn request_budget_rejects_incomplete_expired_or_cancelled_credentials() {
        let now = UNIX_EPOCH + Duration::from_secs(4985);
        let valid = request();
        assert_eq!(
            request_budget(&valid, now).unwrap(),
            Duration::from_secs(15)
        );
        assert_eq!(request_budget(&valid, UNIX_EPOCH).unwrap(), MAX_RUN_TIME);
        assert!(request_budget(&valid, UNIX_EPOCH + Duration::from_secs(5000)).is_err());
        for credentials in [
            Credentials::new(
                "",
                "CB_SECRET",
                Some("CB_TOKEN".into()),
                Some(now + MAX_RUN_TIME),
                "test",
            ),
            Credentials::new(
                "CB_ACCESS",
                "",
                Some("CB_TOKEN".into()),
                Some(now + MAX_RUN_TIME),
                "test",
            ),
            Credentials::new(
                "CB_ACCESS",
                "CB_SECRET",
                None,
                Some(now + MAX_RUN_TIME),
                "test",
            ),
            Credentials::new(
                "CB_ACCESS",
                "CB_SECRET",
                Some("".into()),
                Some(now + MAX_RUN_TIME),
                "test",
            ),
            Credentials::new(
                "CB_ACCESS",
                "CB_SECRET",
                Some("CB_TOKEN".into()),
                None,
                "test",
            ),
        ] {
            let mut incomplete = request();
            incomplete.credentials = credentials;
            assert!(request_budget(&incomplete, now).is_err());
        }
        valid.cancellation.cancel();
        assert_eq!(request_budget(&valid, now).unwrap_err(), CANCELLED);
    }

    #[test]
    fn request_and_output_debug_never_include_values() {
        let debug = format!("{:?}", request());
        for value in [
            "CB_SYNTHETIC_ACCESS",
            "CB_SYNTHETIC_SECRET",
            "CB_SYNTHETIC_TOKEN",
            "CB_PRIVATE_QUERY",
        ] {
            assert!(!debug.contains(value));
        }
        let output = ProcessOutput {
            stdout: b"CB_PRIVATE_OUTPUT".to_vec(),
            stderr: b"CB_PRIVATE_ERROR".to_vec(),
            success: false,
        };
        assert!(!format!("{output:?}").contains("CB_PRIVATE"));
    }

    #[test]
    fn isolated_environment_passes_only_frozen_identity_and_reviewed_runtime_values() {
        let mut input: Vec<(OsString, OsString)> = [
            ("AWS_ACCESS_KEY_ID", "AMBIENT_ACCESS"),
            ("AWS_SECRET_ACCESS_KEY", "AMBIENT_SECRET"),
            ("AWS_SESSION_TOKEN", "AMBIENT_TOKEN"),
            ("AWS_PROFILE", "ambient-profile"),
            ("AWS_DEFAULT_PROFILE", "ambient-default"),
            ("AWS_CONFIG_FILE", "ambient-config"),
            ("AWS_SHARED_CREDENTIALS_FILE", "ambient-credentials"),
            ("AWS_ENDPOINT_URL", "https://example.invalid"),
            ("AWS_ENDPOINT_URL_STS", "https://example.invalid"),
            ("AWS_DATA_PATH", "ambient-models"),
            ("AWS_ROLE_ARN", "ambient-role"),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "ambient-web-token"),
            (
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                "https://example.invalid",
            ),
            ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/ambient"),
            ("HOME", "ambient-home"),
            ("USERPROFILE", "ambient-home"),
            ("PATH", "ambient-path"),
            ("PYTHONPATH", "ambient-python"),
            ("PYTHONHOME", "ambient-python"),
            ("LD_PRELOAD", "ambient-loader"),
            ("DYLD_INSERT_LIBRARIES", "ambient-loader"),
            ("BOTO_CONFIG", "ambient-boto"),
            ("HTTPS_PROXY", "https://proxy.example.invalid"),
            ("NO_PROXY", "localhost"),
            ("SystemRoot", "C:\\Windows"),
            ("WINDIR", "C:\\Windows"),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
        let dir = crate::test_support::TestDir::new();
        let home = dir.path();
        let ca_file = home.join("synthetic-enterprise-ca.pem");
        input.push(("AWS_CA_BUNDLE".into(), ca_file.as_os_str().into()));
        input.push((
            "SSL_CERT_FILE".into(),
            home.join("synthetic-system-ca.pem").into(),
        ));
        for platform in [Platform::Unix, Platform::Windows] {
            let environment = child_environment(&request(), &input, home, platform).unwrap();
            let get = |name| env_value(&environment, name, platform).map(OsStr::to_os_string);
            assert_eq!(get("AWS_ACCESS_KEY_ID"), Some("CB_SYNTHETIC_ACCESS".into()));
            assert_eq!(
                get("AWS_SECRET_ACCESS_KEY"),
                Some("CB_SYNTHETIC_SECRET".into())
            );
            assert_eq!(get("AWS_SESSION_TOKEN"), Some("CB_SYNTHETIC_TOKEN".into()));
            assert_eq!(get("AWS_REGION"), Some("eu-west-1".into()));
            assert_eq!(get("HOME"), Some(home.as_os_str().into()));
            assert_eq!(get("USERPROFILE"), Some(home.as_os_str().into()));
            assert_eq!(get("AWS_CONFIG_FILE"), Some(platform.null_file().into()));
            assert_eq!(
                get("AWS_SHARED_CREDENTIALS_FILE"),
                Some(platform.null_file().into())
            );
            assert_eq!(get("BOTO_CONFIG"), Some(platform.null_file().into()));
            assert_eq!(
                get("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS"),
                Some("true".into())
            );
            assert_eq!(get("AWS_EC2_METADATA_DISABLED"), Some("true".into()));
            assert_eq!(
                get(if platform == Platform::Windows {
                    "HTTPS_PROXY"
                } else {
                    "https_proxy"
                }),
                Some("https://proxy.example.invalid".into())
            );
            assert_eq!(get("AWS_CA_BUNDLE"), Some(ca_file.as_os_str().into()));
            assert_eq!(get("SystemRoot").is_some(), platform == Platform::Windows);
            for name in [
                "AWS_PROFILE",
                "AWS_DEFAULT_PROFILE",
                "AWS_ENDPOINT_URL",
                "AWS_ENDPOINT_URL_STS",
                "AWS_DATA_PATH",
                "AWS_ROLE_ARN",
                "AWS_WEB_IDENTITY_TOKEN_FILE",
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
                "PATH",
                "PYTHONPATH",
                "PYTHONHOME",
                "LD_PRELOAD",
                "DYLD_INSERT_LIBRARIES",
            ] {
                assert!(
                    get(name).is_none(),
                    "unexpected inherited environment key: {name}"
                );
            }
        }
    }

    #[test]
    fn proxy_precedence_and_certificate_paths_are_explicit() {
        let dir = crate::test_support::TestDir::new();
        let proxies = vec![
            ("HTTPS_PROXY".into(), "https://upper.example.invalid".into()),
            ("https_proxy".into(), "https://lower.example.invalid".into()),
        ];
        let unix = child_environment(&request(), &proxies, dir.path(), Platform::Unix).unwrap();
        assert_eq!(
            env_value(&unix, "https_proxy", Platform::Unix),
            Some(OsStr::new("https://lower.example.invalid"))
        );
        assert_eq!(
            unix.iter()
                .filter(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("https_proxy"))
                .count(),
            1
        );
        assert!(child_environment(&request(), &proxies, dir.path(), Platform::Windows).is_err());
        let identical = vec![
            ("HTTPS_PROXY".into(), "https://same.example.invalid".into()),
            ("https_proxy".into(), "https://same.example.invalid".into()),
        ];
        let windows =
            child_environment(&request(), &identical, dir.path(), Platform::Windows).unwrap();
        assert_eq!(
            windows
                .iter()
                .filter(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("https_proxy"))
                .count(),
            1
        );
        for key in [
            "AWS_CA_BUNDLE",
            "REQUESTS_CA_BUNDLE",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
        ] {
            for value in ["", "relative-certificate.pem"] {
                let ambient = vec![(key.into(), value.into())];
                assert_eq!(
                    child_environment(&request(), &ambient, dir.path(), Platform::host())
                        .unwrap_err(),
                    "AWS CLI certificate paths must be absolute"
                );
            }
        }
    }

    fn fake_image(path: &Path, bytes: &[u8], executable: bool) {
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
            )
            .unwrap();
        }
        #[cfg(not(unix))]
        let _ = executable;
    }

    #[test]
    fn discovery_returns_the_absolute_native_image_and_rejects_scripts() {
        let dir = crate::test_support::TestDir::new();
        let binary = dir.path().join("aws");
        fake_image(&binary, b"\x7fELFsynthetic-not-executed", true);
        let ambient = vec![(
            OsString::from("PATH"),
            std::env::join_paths([Path::new(""), Path::new("relative"), dir.path()]).unwrap(),
        )];
        assert_eq!(
            discover_binary(&ambient, Platform::Unix),
            Some(LaunchSpec {
                program: binary.canonicalize().unwrap(),
                prefix: Vec::new()
            })
        );
        assert!(native_executable(Path::new("aws"), Platform::Unix).is_none());
        let script = dir.path().join("script-aws");
        fake_image(&script, b"#!/usr/bin/env python3\nsynthetic", true);
        assert!(native_executable(&script, Platform::Unix).is_none());
        #[cfg(unix)]
        {
            let no_exec = dir.path().join("not-executable");
            fake_image(&no_exec, b"\x7fELFsynthetic", false);
            assert!(native_executable(&no_exec, Platform::Unix).is_none());
            let alias = dir.path().join("native-link");
            std::os::unix::fs::symlink(&binary, &alias).unwrap();
            assert_eq!(
                native_executable(&alias, Platform::Unix),
                Some(binary.canonicalize().unwrap())
            );
        }
        let exe = dir.path().join("aws.exe");
        let batch = dir.path().join("aws.cmd");
        fake_image(&exe, b"MZsynthetic-not-executed", false);
        fake_image(&batch, b"MZsynthetic-batch-not-executed", false);
        assert_eq!(
            native_executable(&exe, Platform::Windows),
            Some(exe.canonicalize().unwrap())
        );
        assert!(native_executable(&batch, Platform::Windows).is_none());
        #[cfg(unix)]
        {
            let alias = dir.path().join("batch-link.exe");
            std::os::unix::fs::symlink(&batch, &alias).unwrap();
            assert!(native_executable(&alias, Platform::Windows).is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn absolute_python_wrappers_preserve_virtualenv_path_and_force_isolated_mode() {
        let dir = crate::test_support::TestDir::new();
        let base = dir.path().join("synthetic-base-interpreter");
        fake_image(&base, b"\x7fELFsynthetic-not-executed", true);
        let venv = dir.path().join("libexec/bin");
        std::fs::create_dir_all(&venv).unwrap();
        let wrapper = dir.path().join("aws");
        for name in ["python", "python3", "python3.13"] {
            let interpreter = venv.join(name);
            std::os::unix::fs::symlink(&base, &interpreter).unwrap();
            fake_image(
                &wrapper,
                format!(
                    "#!{}\n# synthetic wrapper: never execute\n",
                    interpreter.display()
                )
                .as_bytes(),
                true,
            );
            let launch = launch_spec(&wrapper, Platform::Unix).unwrap();
            assert_eq!(launch.program, interpreter);
            assert_ne!(
                launch.program,
                interpreter.canonicalize().unwrap(),
                "canonicalizing the interpreter would lose its venv path"
            );
            assert_eq!(
                launch.prefix,
                vec![
                    OsString::from("-I"),
                    wrapper.canonicalize().unwrap().into_os_string()
                ]
            );
            let command = configured_command(&launch, &request(), &[], dir.path()).unwrap();
            let args: Vec<_> = command.as_std().get_args().collect();
            assert_eq!(args[0], "-I");
            assert_eq!(args[1], wrapper.canonicalize().unwrap().as_os_str());
            assert_eq!(args[2], "sts");
        }
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_discovery_rejects_env_shell_options_and_non_native_interpreters() {
        let dir = crate::test_support::TestDir::new();
        let wrapper = dir.path().join("aws");
        let python = dir.path().join("python3");
        fake_image(&python, b"\x7fELFsynthetic-not-executed", true);
        for shebang in [
            format!("#!{} -s", python.display()),
            format!("#!{} python3", dir.path().join("env").display()),
            format!("#!{}", dir.path().join("bash").display()),
            format!("#!{}", dir.path().join("sh").display()),
            format!("#!{}", dir.path().join("python3.13-config").display()),
            "#!relative/python3".into(),
            format!("#!{}", "x".repeat(4097)),
        ] {
            fake_image(
                &wrapper,
                format!("{shebang}\n# do not execute\n").as_bytes(),
                true,
            );
            assert!(launch_spec(&wrapper, Platform::Unix).is_none());
        }
        fake_image(
            &wrapper,
            format!("#!{}\n", python.display()).as_bytes(),
            false,
        );
        assert!(launch_spec(&wrapper, Platform::Unix).is_none());
        fake_image(
            &wrapper,
            format!("#!{}\n", python.display()).as_bytes(),
            true,
        );
        fake_image(
            &python,
            b"#!/synthetic/sh\n# not a native interpreter",
            true,
        );
        assert!(launch_spec(&wrapper, Platform::Unix).is_none());
    }

    #[test]
    fn command_plan_uses_absolute_program_and_empty_owned_home_without_spawning() {
        let dir = crate::test_support::TestDir::new();
        let home = IsolatedHome::create_in(dir.path()).unwrap();
        let path = home.0.clone();
        let binary = dir.path().join("synthetic-aws");
        let command = configured_command(
            &LaunchSpec {
                program: binary.clone(),
                prefix: Vec::new(),
            },
            &request(),
            &[],
            &path,
        )
        .unwrap();
        assert_eq!(command.as_std().get_program(), binary.as_os_str());
        assert_eq!(command.as_std().get_current_dir(), Some(path.as_path()));
        assert!(std::fs::read_dir(&path).unwrap().next().is_none());
        let values: Vec<_> = command.as_std().get_envs().collect();
        assert!(values
            .iter()
            .any(|(name, value)| *name == "AWS_ACCESS_KEY_ID"
                && *value == Some(OsStr::new("CB_SYNTHETIC_ACCESS"))));
        drop(command);
        drop(home);
        assert!(!path.exists());
        let retained = IsolatedHome::create_in(dir.path()).unwrap();
        let retained_path = retained.0.clone();
        std::fs::write(
            retained_path.join("unowned-child-file"),
            b"non-secret fixture",
        )
        .unwrap();
        drop(retained);
        assert!(
            retained_path.join("unowned-child-file").exists(),
            "cleanup must not recursively remove files"
        );
    }

    #[derive(Default)]
    struct Events {
        kills: AtomicUsize,
        waits: AtomicUsize,
        reaped: AtomicBool,
        dropped_before_reap: AtomicBool,
        started: Notify,
        killed: Notify,
        completed: Notify,
    }

    struct FakeChild {
        events: Arc<Events>,
        exit: watch::Receiver<Option<ChildExit>>,
        exit_sender: watch::Sender<Option<ChildExit>>,
        auto_exit_on_kill: bool,
        kill_error: bool,
        wait_failures: usize,
    }

    impl ChildControl for FakeChild {
        fn start_kill(&mut self) -> io::Result<()> {
            self.events.kills.fetch_add(1, Ordering::SeqCst);
            self.events.killed.notify_one();
            if self.auto_exit_on_kill {
                self.exit_sender.send_replace(Some(exit(false)));
            }
            if self.kill_error {
                Err(io::Error::other("CB_SECRET_KILL_ERROR"))
            } else {
                Ok(())
            }
        }

        fn wait(&mut self) -> BoxFuture<'_, io::Result<ChildExit>> {
            Box::pin(async move {
                self.events.waits.fetch_add(1, Ordering::SeqCst);
                self.events.started.notify_one();
                if self.wait_failures > 0 {
                    self.wait_failures -= 1;
                    return Err(io::Error::other("CB_SECRET_WAIT_ERROR"));
                }
                loop {
                    if let Some(exit) = self.exit.borrow_and_update().clone() {
                        if !self.events.reaped.swap(true, Ordering::SeqCst) {
                            self.events.completed.notify_one();
                        }
                        return Ok(exit);
                    }
                    self.exit
                        .changed()
                        .await
                        .map_err(|_| io::Error::other("synthetic child exit sender closed"))?;
                }
            })
        }
    }

    impl Drop for FakeChild {
        fn drop(&mut self) {
            self.events
                .dropped_before_reap
                .store(!self.events.reaped.load(Ordering::SeqCst), Ordering::SeqCst);
        }
    }

    fn exit(success: bool) -> ChildExit {
        ChildExit { success }
    }

    fn child() -> (FakeChild, Arc<Events>, watch::Sender<Option<ChildExit>>) {
        let events = Arc::new(Events::default());
        let (tx, rx) = watch::channel(None);
        (
            FakeChild {
                events: events.clone(),
                exit: rx,
                exit_sender: tx.clone(),
                auto_exit_on_kill: true,
                kill_error: false,
                wait_failures: 0,
            },
            events,
            tx,
        )
    }

    fn no_deadline() -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }

    fn assert_reaped(events: &Events) {
        assert!(events.reaped.load(Ordering::SeqCst));
        assert!(!events.dropped_before_reap.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn exact_caps_and_nonzero_exit_are_collected_without_killing() {
        for success in [true, false] {
            let (child, events, finish) = child();
            finish.send_replace(Some(exit(success)));
            let output = supervise_owned(
                child,
                std::io::Cursor::new(vec![b'a'; MAX_STDOUT_BYTES]),
                std::io::Cursor::new(vec![b'b'; MAX_STDERR_BYTES]),
                ProcessCancellation::new(),
                no_deadline(),
                OutputLimits::default(),
                None,
            )
            .await
            .unwrap();
            assert_eq!(output.stdout.len(), MAX_STDOUT_BYTES);
            assert_eq!(output.stderr.len(), MAX_STDERR_BYTES);
            assert_eq!(output.success, success);
            assert_eq!(events.kills.load(Ordering::SeqCst), 0);
            assert_reaped(&events);
        }
    }

    #[tokio::test]
    async fn each_stream_overflow_is_stopped_and_reaped() {
        for stdout_overflows in [true, false] {
            let (child, events, _) = child();
            let stdout = vec![
                b'a';
                if stdout_overflows {
                    MAX_STDOUT_BYTES + 1
                } else {
                    1
                }
            ];
            let stderr = vec![
                b'b';
                if stdout_overflows {
                    1
                } else {
                    MAX_STDERR_BYTES + 1
                }
            ];
            let error = supervise_owned(
                child,
                std::io::Cursor::new(stdout),
                std::io::Cursor::new(stderr),
                ProcessCancellation::new(),
                no_deadline(),
                OutputLimits::default(),
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                if stdout_overflows {
                    "AWS CLI stdout exceeded its 2 MiB limit"
                } else {
                    "AWS CLI stderr exceeded its 256 KiB limit"
                }
            );
            assert_eq!(events.kills.load(Ordering::SeqCst), 1);
            assert_reaped(&events);
        }
    }

    struct EndlessReader(Arc<AtomicUsize>);
    impl AsyncRead for EndlessReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let count = buffer.remaining();
            buffer.put_slice(&vec![b'x'; count]);
            self.0.fetch_add(count, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn endless_output_without_newlines_stops_at_cap_plus_one_byte() {
        let (child, events, _) = child();
        let bytes = Arc::new(AtomicUsize::new(0));
        let error = supervise_owned(
            child,
            EndlessReader(bytes.clone()),
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(error.contains("stdout"));
        assert_eq!(bytes.load(Ordering::SeqCst), MAX_STDOUT_BYTES + 1);
        assert_reaped(&events);
    }

    #[tokio::test]
    async fn both_pipes_are_drained_concurrently_before_process_exit() {
        let (child, events, finish) = child();
        let (stdout, mut stdout_writer) = tokio::io::duplex(8);
        let (stderr, mut stderr_writer) = tokio::io::duplex(8);
        let run = supervise_owned(
            child,
            stdout,
            stderr,
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        );
        let write = async move {
            stdout_writer
                .write_all(b"synthetic stdout content")
                .await
                .unwrap();
            stderr_writer
                .write_all(b"synthetic stderr content")
                .await
                .unwrap();
            stdout_writer.shutdown().await.unwrap();
            stderr_writer.shutdown().await.unwrap();
            finish.send_replace(Some(exit(true)));
        };
        let (result, ()) = tokio::join!(run, write);
        let output = result.unwrap();
        assert_eq!(output.stdout, b"synthetic stdout content");
        assert_eq!(output.stderr, b"synthetic stderr content");
        assert_reaped(&events);
    }

    #[tokio::test]
    async fn exit_status_does_not_discard_output_that_has_not_reached_eof() {
        let (child, events, finish) = child();
        finish.send_replace(Some(exit(true)));
        let (stdout, mut writer) = tokio::io::duplex(8);
        let run = supervise_owned(
            child,
            stdout,
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        );
        tokio::pin!(run);
        assert!(futures::poll!(&mut run).is_pending());
        events.completed.notified().await;
        assert!(futures::poll!(&mut run).is_pending());
        writer.write_all(b"late").await.unwrap();
        writer.shutdown().await.unwrap();
        assert_eq!(run.await.unwrap().stdout, b"late");
        assert_reaped(&events);
    }

    #[tokio::test]
    async fn output_eof_still_waits_for_process_exit() {
        let (child, events, finish) = child();
        let run = supervise_owned(
            child,
            tokio::io::empty(),
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        );
        tokio::pin!(run);
        assert!(futures::poll!(&mut run).is_pending());
        events.started.notified().await;
        assert!(futures::poll!(&mut run).is_pending());
        finish.send_replace(Some(exit(true)));
        assert!(run.await.unwrap().success);
        assert_reaped(&events);
    }

    #[tokio::test]
    async fn cancellation_and_injected_deadline_await_reaping_before_returning() {
        for cancel in [true, false] {
            let (mut child, events, finish) = child();
            child.auto_exit_on_kill = false;
            let cancellation = ProcessCancellation::new();
            let (deadline_tx, deadline_rx) = oneshot::channel::<()>();
            let (stdout, _writer) = tokio::io::duplex(8);
            let run = supervise_owned(
                child,
                stdout,
                tokio::io::empty(),
                cancellation.clone(),
                Box::pin(async move {
                    let _ = deadline_rx.await;
                }),
                OutputLimits::default(),
                None,
            );
            tokio::pin!(run);
            assert!(futures::poll!(&mut run).is_pending());
            events.started.notified().await;
            if cancel {
                cancellation.cancel();
            } else {
                deadline_tx.send(()).unwrap();
            }
            events.killed.notified().await;
            assert!(
                futures::poll!(&mut run).is_pending(),
                "must wait for cleanup, not merely send kill"
            );
            finish.send_replace(Some(exit(false)));
            assert_eq!(
                run.await.unwrap_err(),
                if cancel { CANCELLED } else { TIMED_OUT }
            );
            assert_reaped(&events);
        }
    }

    #[tokio::test]
    async fn dropped_caller_still_cancels_and_reaps_before_home_cleanup() {
        let dir = crate::test_support::TestDir::new();
        let home = IsolatedHome::create_in(dir.path()).unwrap();
        let home_path = home.0.clone();
        let (mut child, events, finish) = child();
        child.auto_exit_on_kill = false;
        let cancellation = ProcessCancellation::new();
        let (stdout, _writer) = tokio::io::duplex(8);
        let mut run = Box::pin(supervise_owned(
            child,
            stdout,
            tokio::io::empty(),
            cancellation.clone(),
            no_deadline(),
            OutputLimits::default(),
            Some(home),
        ));
        assert!(futures::poll!(&mut run).is_pending());
        events.started.notified().await;
        drop(run);
        events.killed.notified().await;
        assert!(cancellation.is_cancelled());
        assert!(
            home_path.exists(),
            "home must stay owned until the child exits"
        );
        finish.send_replace(Some(exit(false)));
        events.completed.notified().await;
        // Completion notification is emitted synchronously by wait; the owner
        // then drops its home before yielding back to this task.
        assert!(!home_path.exists());
        assert_reaped(&events);
    }

    struct FailedReader;
    impl AsyncRead for FailedReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("CB_SECRET_READ_ERROR")))
        }
    }

    #[tokio::test]
    async fn read_or_wait_failures_are_sanitized_and_still_reaped() {
        let (mut child, events, _) = self::child();
        child.kill_error = true;
        let error = supervise_owned(
            child,
            FailedReader,
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "AWS CLI output could not be read");
        assert_eq!(events.kills.load(Ordering::SeqCst), 1);
        assert!(events.waits.load(Ordering::SeqCst) >= 1);
        assert_reaped(&events);
        let (mut child, events, _) = self::child();
        child.wait_failures = 1;
        let error = supervise_owned(
            child,
            tokio::io::empty(),
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "AWS CLI exit status could not be read");
        assert_eq!(events.waits.load(Ordering::SeqCst), 2);
        assert_reaped(&events);
    }

    #[tokio::test]
    async fn inability_to_confirm_reaping_is_reported_instead_of_claiming_cleanup() {
        let (mut child, events, _) = child();
        child.wait_failures = 2;
        let error = supervise_owned(
            child,
            tokio::io::empty(),
            tokio::io::empty(),
            ProcessCancellation::new(),
            no_deadline(),
            OutputLimits::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "AWS CLI cleanup failed; process exit could not be confirmed"
        );
        assert!(!events.reaped.load(Ordering::SeqCst));
        assert_eq!(events.waits.load(Ordering::SeqCst), 2);
        assert_eq!(events.kills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn already_cancelled_signal_cannot_be_missed_by_a_new_waiter() {
        let cancellation = ProcessCancellation::new();
        cancellation.cancel();
        cancellation.cancelled().await;
        let (child, events, _) = child();
        assert_eq!(
            supervise_owned(
                child,
                tokio::io::empty(),
                tokio::io::empty(),
                cancellation,
                no_deadline(),
                OutputLimits::default(),
                None
            )
            .await
            .unwrap_err(),
            CANCELLED
        );
        assert_reaped(&events);
    }
}
