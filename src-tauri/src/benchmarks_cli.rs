//! Opt-in measurements through the actual CLI widget using a synthetic runner.
use crate::{
    aws::policy::Policy,
    process::{CliRequest, ProcessCancellation, ProcessOutput, ProcessRunner},
    runtime::{test_sdk_config, Runtime},
    test_support::TestDir,
    widgets::{self, CliAccess, WidgetCtx},
};
use futures::future::{join_all, BoxFuture};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[derive(Default)]
struct Counts {
    calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    bytes: AtomicUsize,
}
struct SyntheticRunner {
    counts: Arc<Counts>,
    output: Vec<u8>,
}
struct Active(Arc<Counts>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ProcessRunner for SyntheticRunner {
    fn run(&self, request: CliRequest) -> BoxFuture<'_, Result<ProcessOutput, String>> {
        Box::pin(async move {
            assert_eq!(request.region, "eu-west-1");
            self.counts.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.counts.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.counts.peak.fetch_max(active, Ordering::SeqCst);
            let _active = Active(self.counts.clone());
            tokio::select! {
                _ = request.cancellation.cancelled() => return Err("AWS CLI request cancelled".into()),
                _ = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
            self.counts
                .bytes
                .fetch_add(self.output.len(), Ordering::SeqCst);
            Ok(ProcessOutput {
                stdout: self.output.clone(),
                stderr: Vec::new(),
                success: true,
            })
        })
    }
}

async fn measure(scenario: &str) -> Value {
    let jobs = if scenario == "cli-50-pins" { 50 } else { 1 };
    let row_count = if jobs == 50 { 250 } else { 8000 };
    let payload = json!({"StackSummaries": (0..row_count).map(|index|json!({"StackName":format!("synthetic-stack-{index:05}"),"Description":"x".repeat(180)})).collect::<Vec<_>>()});
    let output = serde_json::to_vec(&payload).unwrap();
    assert!(output.len() <= 2 * 1024 * 1024);
    let output_bytes = output.len();
    let counts = Arc::new(Counts::default());
    let dir = TestDir::new();
    let mut runtime = Runtime::for_test(dir.paths());
    runtime.process = Arc::new(SyntheticRunner {
        counts: counts.clone(),
        output,
    });
    let contexts: Vec<_> = (0..jobs).map(|_|WidgetCtx {
        runtime: runtime.clone(), sdk:test_sdk_config(), account_id:"acct-fixture".into(), region:"eu-west-1".into(), widget_name:"aws-cli".into(),
        inputs:json!({"command":"aws cloudformation list-stacks"}),
        policy:Policy::parse("statements:\n  - effect: Allow\n    action: [sso:GetRoleCredentials, cloudformation:ListStacks]\n").map_err(|e|e.message),
        cli:Some(CliAccess { credentials:aws_credential_types::Credentials::new("CB_SYNTHETIC_KEY","CB_SYNTHETIC_SECRET",Some("CB_SYNTHETIC_TOKEN".into()),Some(UNIX_EPOCH+Duration::from_secs(1_800_000_000)),"widget-test-only"), cancellation:ProcessCancellation::new() })
    }).collect();
    let started = Instant::now();
    let results = join_all(contexts.iter().map(|ctx| widgets::fetch("aws-cli", ctx))).await;
    let complete_ms = started.elapsed().as_secs_f64() * 1000.0;
    let failed = results
        .iter()
        .filter(|r| r.get("error").is_some() || r["ok"] == false)
        .count();
    json!({"kind":"trial","suite":"cli","fixture_version":"p3-cli-fixture-v1","seed":7301,"scenario":scenario,"complete_ms":complete_ms,"jobs":jobs,"input_rows_per_job":row_count,"stdout_bytes_per_job":output_bytes,"fake_runner_calls":counts.calls.load(Ordering::SeqCst),"peak_active_fake_children":counts.peak.load(Ordering::SeqCst),"active_after_completion":counts.active.load(Ordering::SeqCst),"stdout_bytes_total":counts.bytes.load(Ordering::SeqCst),"returned_json_bytes":results.iter().map(|r|serde_json::to_vec(r).unwrap().len()).sum::<usize>(),"failed_jobs":failed,"returned_rows_total":results.iter().map(|r|r["rows"].as_array().map_or(0,Vec::len)).sum::<usize>(),"outcome":if failed==0 {"succeeded"} else {"failed"},"native_memory_bytes":null,"actual_cli_processes":0,"actual_aws_http_attempts":0,"measurement_scope":"real widget parser and table conversion; synthetic 1ms runner; shared Runtime; no subprocess or AWS"})
}

#[tokio::test]
#[ignore = "opt-in synthetic CLI performance measurement"]
async fn replay_cli_baseline() {
    let source =
        std::env::var("CLOUD_BURRITO_BENCH_SOURCE").unwrap_or_else(|_| "unrecorded".into());
    assert!(
        source == "unrecorded"
            || ((7..=64).contains(&source.len()) && source.bytes().all(|b| b.is_ascii_hexdigit()))
    );
    for scenario in ["cli-near-cap", "cli-50-pins"] {
        let mut times = Vec::new();
        for iteration in 0..35 {
            let mut result = measure(scenario).await;
            result["warmup"] = json!(iteration < 5);
            result["trial_index"] = json!(if iteration < 5 {
                iteration
            } else {
                iteration - 5
            });
            result["source_revision"] = json!(source);
            result["build_mode"] = json!(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            });
            if iteration >= 5 {
                times.push(result["complete_ms"].as_f64().unwrap());
            }
            println!("CB_BENCH_JSON {}", result);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "CB_BENCH_JSON {}",
            json!({"kind":"summary","suite":"cli","scenario":scenario,"source_revision":source,"fixture_version":"p3-cli-fixture-v1","seed":7301,"warmups":5,"trials":30,"median_ms":(times[14]+times[15])/2.0,"p95_ms":times[28],"maximum_ms":times[29]})
        );
    }
}
