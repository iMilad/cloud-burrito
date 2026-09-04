//! Injectable boundary for the optional AWS CLI child process.

use futures::future::BoxFuture;

#[cfg(not(test))]
use std::{path::Path, process::Stdio, time::Duration};
#[cfg(not(test))]
use tokio::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub struct CliRequest {
    pub argv: Vec<String>,
    pub profile: String,
    pub region: String,
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
    pub status: String,
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

#[cfg(not(test))]
async fn run_native(request: CliRequest) -> Result<ProcessOutput, String> {
    const RUN_TIMEOUT: Duration = Duration::from_secs(30);

    let bin = aws_binary()
        .ok_or_else(|| "aws CLI not found — install AWS CLI v2 or add it to PATH".to_string())?;
    let mut cmd = Command::new(&bin);
    cmd.args(&request.argv)
        .env("AWS_PROFILE", &request.profile)
        .env("AWS_REGION", &request.region)
        .env("AWS_DEFAULT_REGION", &request.region)
        .env("AWS_PAGER", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not run {bin}: {e}"))?;
    let output = match tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("command failed to run: {e}")),
        Err(_) => return Err("command timed out after 30s (killed)".to_string()),
    };
    Ok(ProcessOutput {
        success: output.status.success(),
        status: output.status.to_string(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// PATH first; GUI-launched macOS apps often miss Homebrew's dirs, so fall
/// back to the usual install locations.
#[cfg(not(test))]
fn aws_binary() -> Option<String> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("aws").is_file() {
                return Some("aws".to_string());
            }
        }
    }
    ["/opt/homebrew/bin/aws", "/usr/local/bin/aws"]
        .iter()
        .find(|c| Path::new(c).is_file())
        .map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[should_panic(expected = "native process execution is forbidden in tests")]
    async fn native_runner_rejects_execution_before_discovery() {
        NativeProcessRunner
            .run(CliRequest {
                argv: vec!["sts".into(), "get-caller-identity".into()],
                profile: "fixture-profile".into(),
                region: "eu-west-1".into(),
            })
            .await
            .unwrap();
    }
}
