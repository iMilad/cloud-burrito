//! Opt-in, repeatable synthetic producer measurements. No native runtime, real
//! provider, subprocess, personal configuration, or production data is used.
//! Run only the ignored entry point; normal tests validate the measurement seam.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{
    audit,
    paths::AppPaths,
    test_aws::{ExpectedRequest, ScriptedHttp},
    test_support::TestDir,
    widgets,
};

const FIXTURE_VERSION: &str = "p3-backend-fixture-v1";
const SEED: u64 = 7301;
const WARMUPS: usize = 5;
const TRIALS: usize = 30;
const SCENARIOS: &[&str] = &[
    "ca-progressive-50",
    "ca-progressive-1000",
    "ca-50-immediate",
    "ca-1000-immediate",
    "ca-1-delay-100",
    "ca-1-delay-500",
    "ca-1-outlier-5000",
    "ca-5-detail-denied",
    "ca-5-throttled",
    "ca-5-page-failed",
    "audit-1mib",
    "audit-10mib",
    "audit-100mib",
];

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DetailDenied,
    Throttled,
    PageFailed,
}

fn package_name(index: usize) -> String {
    format!("synthetic-package-{index:04}")
}

fn package_fixture(count: usize, delay_ms: u64, fault: Fault) -> Vec<ExpectedRequest> {
    let returned = if fault == Fault::PageFailed { 2 } else { count };
    let packages: Vec<_> = (0..returned)
        .map(|index| json!({"package":package_name(index)}))
        .collect();
    let mut requests = vec![ExpectedRequest::rest(
        "POST", "/v1/packages",
        json!({"domain":"synthetic-domain","repository":"synthetic-repository","format":"pypi","max-results":count.to_string(),"next-token":null}),
        json!({"packages":packages,"nextToken":if fault == Fault::PageFailed { Some("synthetic-next-page") } else { None }}),
    ).delay(Duration::from_millis(delay_ms))];
    if fault == Fault::PageFailed {
        requests.push(ExpectedRequest::rest(
            "POST", "/v1/packages",
            json!({"domain":"synthetic-domain","repository":"synthetic-repository","max-results":(count-returned).to_string(),"next-token":"synthetic-next-page"}),
            json!({"__type":"AccessDeniedException","message":"Synthetic page failure"}),
        ).status(403).delay(Duration::from_millis(delay_ms)));
        return requests;
    }
    for index in 0..count {
        let package = package_name(index);
        requests.push(ExpectedRequest::rest(
            "POST", "/v1/package/versions",
            json!({"domain":"synthetic-domain","repository":"synthetic-repository","package":package,"max-results":"10"}),
            json!({"versions":[{"version":"1.0.0","status":"Published"}]}),
        ).delay(Duration::from_millis(delay_ms)));
        let failed = index == count / 2 && fault != Fault::None;
        let response = if failed {
            json!({"__type":if fault == Fault::Throttled { "ThrottlingException" } else { "AccessDeniedException" },"message":"Synthetic detail failure"})
        } else {
            json!({"packageVersion":{"packageName":package,"version":"1.0.0","status":"Published","publishedTime":1.0}})
        };
        requests.push(ExpectedRequest::rest(
            "GET", "/v1/package/version",
            json!({"domain":"synthetic-domain","repository":"synthetic-repository","package":package,"version":"1.0.0"}),response,
        ).status(if failed { if fault == Fault::Throttled { 429 } else { 403 } } else { 200 })
            .delay(Duration::from_millis(delay_ms)));
    }
    requests
}

fn preflight_counts(paths: &AppPaths) -> (usize, usize) {
    let Ok(file) = File::open(paths.data_file("audit.log")) else {
        return (0, 0);
    };
    let mut resource = 0;
    let mut credentials = 0;
    for line in BufReader::new(file).lines() {
        let value: Value = serde_json::from_str(&line.expect("synthetic audit line readable"))
            .expect("synthetic audit JSON readable");
        if value["scope"] == "capability_preflight" {
            if value["service"] == "codeartifact" {
                resource += 1;
            } else {
                credentials += 1;
            }
        }
    }
    (resource, credentials)
}

async fn measure_packages(scenario: &str, measured_index: Option<usize>) -> Value {
    let (count, delay_ms, fault) = match scenario {
        "ca-50-immediate" => (50, 0, Fault::None),
        "ca-1000-immediate" => (1000, 0, Fault::None),
        "ca-1-delay-100" => (1, 100, Fault::None),
        "ca-1-delay-500" => (1, 500, Fault::None),
        "ca-1-outlier-5000" => (1, 0, Fault::None),
        "ca-5-detail-denied" => (5, 0, Fault::DetailDenied),
        "ca-5-throttled" => (5, 0, Fault::Throttled),
        "ca-5-page-failed" => (5, 0, Fault::PageFailed),
        _ => unreachable!("validated scenario"),
    };
    let mut expected = package_fixture(count, delay_ms, fault);
    // A selected measured trial has one 5s response; do not multiply a
    // 1000-package serial baseline by 5s per response or hide the outlier.
    let outlier = scenario == "ca-1-outlier-5000" && measured_index == Some(17);
    if outlier {
        let last = expected.pop().unwrap();
        expected.push(last.delay(Duration::from_secs(5)));
    }
    let planned_sdk_calls = expected.len();
    let directory = TestDir::new();
    let http = ScriptedHttp::new(expected);
    let ctx = http.context(&directory,"codeartifact-packages",json!({
        "domain":"synthetic-domain","repository":"synthetic-repository","package_prefix":"synthetic-package-","max_packages":count,
    }));
    http.begin_measurement();
    let started = Instant::now();
    let result = widgets::fetch("codeartifact-packages", &ctx).await;
    let complete_ms = started.elapsed().as_secs_f64() * 1000.0;
    http.assert_finished();
    let transport = http.snapshot();
    let (preflights, credential_preflights) = preflight_counts(&directory.paths());
    let rows = result["rows"].as_array().map_or(0, Vec::len);
    let outcome = if result["partial"] == true {
        "partial"
    } else if result.get("error").is_some() || result["ok"] == false {
        "failed"
    } else {
        "succeeded"
    };
    json!({
        "scenario":scenario,"component":"codeartifact_producer","complete_ms":complete_ms,
        "first_useful_ms":if rows>0 { Some(complete_ms) } else { None },
        "first_useful_definition":"current fetch returns one final table; no earlier UI delivery is measured",
        "package_count":count,"returned_rows":rows,"delay_per_http_response_ms":delay_ms,"outlier_response_ms":if outlier { 5000 } else { 0 },
        "planned_sdk_calls":planned_sdk_calls,"capability_preflight_count":preflights,"credential_capability_preflight_count":credential_preflights,
        "logical_operations_note":"capability preflights are application checks, not SDK wire attempts; planned SDK calls come from the fixed fixture",
        "transport":transport,"sdk_retry_mode":"disabled","actual_retry_attempts":0,
        "outcome":outcome,"failed_rows":result["coverage"]["sections"]["enrichment"]["counts"]["failed"].as_u64().unwrap_or(0),
        "successful_package_pages":result["coverage"]["sections"]["packages"]["counts"]["pages"].as_u64().unwrap_or(0),
        "coverage_completeness":result["coverage"]["completeness"].as_str(),
        "returned_json_bytes":serde_json::to_vec(&result).unwrap().len(),
        "preflight_file_write_in_timed_scope":true,"fixture_construction_in_timed_scope":false,
        "native_memory_bytes":null,"memory_note":"native backend plus webview and retained allocator memory are unmeasured; transport body counts are not process memory",
    })
}

fn prepare_audit(bytes: usize) -> TestDir {
    let directory = TestDir::new();
    let path = directory.paths().data_file("audit.log");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::new(File::create(path).unwrap());
    // Valid fixed-size 1KiB records, including whitespace padding and newline.
    // Every fixture therefore contains more than the requested 300 entries.
    for index in 0..bytes / 1024 {
        let mut row = format!("{{\"kind\":\"request\",\"event\":\"succeeded\",\"request_id\":\"synthetic-benchmark\",\"ts\":{index}}}").into_bytes();
        row.resize(1023, b' ');
        row.push(b'\n');
        writer.write_all(&row).unwrap();
    }
    writer.flush().unwrap();
    writer.get_ref().sync_all().unwrap();
    directory
}

fn measure_audit(scenario: &str, directory: &TestDir, bytes: usize) -> Value {
    let started = Instant::now();
    let result = audit::try_tail(&directory.paths(), 300);
    let complete_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (outcome, rows, returned_bytes) = match result {
        Ok(rows) => (
            "succeeded",
            rows.len(),
            serde_json::to_vec(&rows).unwrap().len(),
        ),
        Err(()) => ("failed", 0, 0),
    };
    json!({
        "scenario":scenario,"component":"audit_tail","complete_ms":complete_ms,
        "fixture_bytes":bytes,"fixture_record_bytes":1024,"requested_entries":300,"returned_rows":rows,
        "outcome":outcome,"returned_json_bytes":returned_bytes,
        "bytes_read":null,"bytes_read_note":"unmeasured; unchanged try_tail scans to EOF according to source, not an OS I/O counter",
        "filesystem_cache":"warm after five repeated reads; no OS cache flush", "fixture_construction_in_timed_scope":false,
        "native_memory_bytes":null,"memory_note":"native backend plus webview and retained allocator memory are unmeasured",
    })
}

fn emit(mut value: Value, source: &str, warmup: bool, index: usize) {
    value["kind"] = json!("trial");
    value["suite"] = json!("backend");
    value["fixture_version"] = json!(FIXTURE_VERSION);
    value["seed"] = json!(SEED);
    value["source_revision"] = json!(source);
    value["build_mode"] = json!(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    value["warmup"] = json!(warmup);
    value["trial_index"] = json!(index);
    println!("CB_BENCH_JSON {}", value);
}

fn summary(scenario: &str, measured: &[Value], source: &str) {
    let mut values: Vec<_> = measured
        .iter()
        .map(|value| value["complete_ms"].as_f64().unwrap())
        .collect();
    values.sort_by(f64::total_cmp);
    let mut outcomes = BTreeMap::<String, usize>::new();
    for value in measured {
        *outcomes
            .entry(value["outcome"].as_str().unwrap().to_string())
            .or_default() += 1;
    }
    println!(
        "CB_BENCH_JSON {}",
        json!({
            "kind":"summary","suite":"backend","scenario":scenario,"source_revision":source,"fixture_version":FIXTURE_VERSION,"seed":SEED,
            "build_mode":if cfg!(debug_assertions) { "debug" } else { "release" },"warmups":WARMUPS,"trials":measured.len(),
            "median_ms":(values[14]+values[15])/2.0,"p95_ms":values[28],"maximum_ms":values[29],"outcomes":outcomes,
            "method":"30 measured trials; empirical nearest-rank p95; small-sample synthetic warm-process benchmark",
        })
    );
}

#[tokio::test]
#[ignore = "opt-in synthetic performance measurement: emits raw metadata only"]
async fn replay_backend_baseline() {
    let source =
        std::env::var("CLOUD_BURRITO_BENCH_SOURCE").unwrap_or_else(|_| "unrecorded".into());
    assert!(
        source == "unrecorded"
            || ((7..=64).contains(&source.len())
                && source.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "source revision must be an opaque hexadecimal revision"
    );
    let requested = std::env::var("CLOUD_BURRITO_BENCH_SCENARIOS").ok();
    let scenarios: Vec<_> = requested
        .as_deref()
        .map(|names| names.split(',').collect())
        .unwrap_or_else(|| SCENARIOS.to_vec());
    assert!(
        !scenarios.is_empty() && scenarios.iter().all(|name| SCENARIOS.contains(name)),
        "unknown synthetic benchmark scenario"
    );
    for scenario in scenarios {
        let bytes = match scenario {
            "audit-1mib" => 1024 * 1024,
            "audit-10mib" => 10 * 1024 * 1024,
            "audit-100mib" => 100 * 1024 * 1024,
            _ => 0,
        };
        let audit_dir = (bytes > 0).then(|| prepare_audit(bytes));
        let mut measured = Vec::new();
        for iteration in 0..WARMUPS + TRIALS {
            let warmup = iteration < WARMUPS;
            let index = if warmup {
                iteration
            } else {
                iteration - WARMUPS
            };
            let value = if let Some(directory) = &audit_dir {
                measure_audit(scenario, directory, bytes)
            } else if scenario.starts_with("ca-progressive-") {
                Box::pin(measure_progressive_packages(scenario)).await
            } else {
                Box::pin(measure_packages(scenario, (!warmup).then_some(index))).await
            };
            emit(value.clone(), &source, warmup, index);
            if !warmup {
                measured.push(value);
            }
        }
        summary(scenario, &measured, &source);
    }
}

#[tokio::test]
async fn transport_measurements_distinguish_attempts_completion_bytes_and_dropped_work() {
    use aws_smithy_runtime_api::client::http::HttpConnector;
    let http = ScriptedHttp::new(vec![ExpectedRequest::rest(
        "GET",
        "/synthetic-metrics",
        json!({}),
        json!({"synthetic":true}),
    )
    .delay(Duration::from_secs(5))]);
    http.begin_measurement();
    let mut request = aws_smithy_runtime_api::client::orchestrator::HttpRequest::new(
        aws_smithy_types::body::SdkBody::from(""),
    );
    request.set_method("GET").unwrap();
    request
        .set_uri("https://cloud-burrito-test.invalid/synthetic-metrics")
        .unwrap();
    let pending = http.call(request);
    assert_eq!(http.snapshot().active_http_attempts, 1);
    drop(pending);
    http.assert_finished();
    let snapshot = http.snapshot();
    assert_eq!(snapshot.actual_http_attempts, 1);
    assert_eq!(snapshot.completed_http_attempts, 0);
    assert_eq!(snapshot.dropped_http_attempts, 1);
    assert_eq!(snapshot.active_http_attempts, 0);
    assert_eq!(snapshot.peak_active_http_attempts, 1);
    assert_eq!(snapshot.response_body_bytes, 0);
    assert!(snapshot.first_response_ms.is_none());

    let value = measure_packages("ca-5-detail-denied", None).await;
    assert_eq!(value["outcome"], "partial");
    assert_eq!(value["failed_rows"], 1);
    assert_eq!(value["transport"]["actual_http_attempts"], 11);
    assert_eq!(value["transport"]["completed_http_attempts"], 11);
    assert_eq!(value["transport"]["active_http_attempts"], 0);
    assert_eq!(value["transport"]["peak_active_http_attempts"], 1);
    assert!(value["transport"]["first_response_ms"].as_f64().is_some());
    assert!(value["transport"]["response_body_bytes"].as_u64().unwrap() > 0);
}

async fn measure_progressive_packages(scenario: &str) -> Value {
    let total = if scenario.contains("1000") { 1000 } else { 50 };
    let directory = TestDir::new();
    let mut expected = package_fixture(25, 0, Fault::None);
    expected[0] = ExpectedRequest::rest(
        "POST",
        "/v1/packages",
        json!({"domain":"synthetic-domain","repository":"synthetic-repository","format":"pypi","max-results":"50","next-token":null}),
        json!({"packages":(0..50).map(|i|json!({"package":package_name(i)})).collect::<Vec<_>>(),
            "nextToken": if total > 50 { Some("synthetic-next-page") } else { None }}),
    );
    let http = ScriptedHttp::new(expected).unordered();
    let mut ctx = http.context(&directory,"codeartifact-packages",json!({
        "mode":"list","domain":"synthetic-domain","repository":"synthetic-repository","package_prefix":"synthetic-package-","max_packages":total,
    }));
    http.begin_measurement();
    let started = Instant::now();
    let list = widgets::fetch("codeartifact-packages", &ctx).await;
    let first_useful_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(http.calls(), 1);
    assert_eq!(list["rows"].as_array().unwrap().len(), 50);
    ctx.inputs = json!({"mode":"enrich","domain":"synthetic-domain","repository":"synthetic-repository",
        "package_prefix":"synthetic-package-","packages":(0..25).map(package_name).collect::<Vec<_>>()});
    let enrichment = widgets::fetch("codeartifact-packages", &ctx).await;
    let complete_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(enrichment["rows"].as_array().unwrap().len(), 25);
    assert_ne!(enrichment["partial"], true);
    http.assert_finished();
    json!({"scenario":scenario,"component":"codeartifact_progressive_producer","complete_ms":complete_ms,
        "first_useful_ms":first_useful_ms,"first_useful_definition":"50 package identities returned before any enrichment request",
        "complete_definition":"first visible25 metadata rows enriched; remaining rows/pages intentionally deferred, not full-dataset completion",
        "package_count":total,"returned_rows":50,"enriched_rows":25,"planned_sdk_calls":51,
        "transport":http.snapshot(),"outcome":"succeeded","sdk_retry_mode":"disabled","actual_retry_attempts":0,
        "returned_json_bytes":serde_json::to_vec(&list).unwrap().len()+serde_json::to_vec(&enrichment).unwrap().len(),
        "native_memory_bytes":null,"fixture_construction_in_timed_scope":false})
}
