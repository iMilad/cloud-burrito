// Included by command_tests.rs: these exercise production command handlers and
// its isolated, fail-closed fixtures. No real files outside TestDir, AWS,
// subprocess, process environment, provider or service transport are used.

fn assert_input_gate_rejected(result: &Value) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error_type"], "InvalidRequest", "{result}");
    assert!(result["error"].as_str().is_some_and(|error| !error.is_empty()));
    assert!(result["_request"]["context_id"].is_null());
    assert!(result["_request"]["provider_revision"].is_null());
    assert!(result["_request"]["settings_revision"].is_null());
}

fn assert_input_gate_no_aws_or_process(fixture: &Fixture) {
    assert_eq!(fixture.aws.snapshot_calls.load(Ordering::SeqCst), 0);
    fixture.aws.assert_no_resolution();
    assert!(fixture.aws.resolved_snapshots.lock().is_empty());
    fixture.no_process();
}

fn assert_input_gate_no_mutation_files(fixture: &Fixture) {
    // Audit history can be created to record rejection. These are the exact
    // persistence targets that an invalid mutation must never create.
    for name in ["settings.json", "dashboard.json", "dashboard.json.tmp", "policy.yaml"] {
        assert!(!fixture.state.runtime.paths.data_file(name).exists(), "unexpected persistence target: {name}");
    }
}

fn input_gate_widget(name: &str, inputs: Value) -> Value {
    json!({"request_id":"input-gate-fixture", "widget":name, "inputs":inputs,
        "context":{"mode":"pinned","profile":"demo-a","account_id":ACCOUNT_A,"region":"us-east-1"}})
}

#[tokio::test]
async fn command_input_gate_rejects_widget_shapes_before_snapshot_or_policy_creation() {
    let mut deep = Value::Null;
    for _ in 0..18 { deep = json!([deep]); }
    let cases = [
        Value::Null,
        json!([]),
        json!({"inputs":{}}),
        input_gate_widget("unknown-widget-fixture", json!({})),
        input_gate_widget("cfn-stacks", Value::Null),
        input_gate_widget("cfn-stacks", json!([])),
        input_gate_widget("cfn-stacks", json!({"name_prefix":42})),
        input_gate_widget("cfn-stacks", json!({"status_filter":[42]})),
        input_gate_widget("cfn-stacks", json!({"status_filter":vec!["CREATE_COMPLETE";65]})),
        input_gate_widget("cfn-stacks", json!({"name_prefix":"x".repeat(2049)})),
        input_gate_widget("cloudwatch-logs", json!({"mode":"unknown-fixture"})),
        input_gate_widget("log-tail", json!({"mode":"groups"})),
        input_gate_widget("log-tail", json!({"mode":"events","log_group":"/fixture/group"})),
        input_gate_widget("logs-insights", json!({"mode":"groups","query":"fields @message"})),
        input_gate_widget("logs-insights", json!({"mode":"query","log_group":"/fixture/group","query":"x".repeat(10*1024+1)})),
        input_gate_widget("cfn-stacks", json!({"unreviewed":deep})),
        input_gate_widget("cfn-stacks", json!({"unreviewed":vec![Value::Null;50_001]})),
        input_gate_widget("cfn-stacks", json!({"unreviewed":vec!["x".repeat(64*1024);17]})),
        input_gate_widget("aws-cli", json!({"command":"x".repeat(16*1024+1)})),
        input_gate_widget("aws-cli", json!({"command":"aws sts get-caller-identity", "credentials":{"key":"CB_SYNTHETIC_NOT_A_KEY"}})),
    ];
    for params in cases {
        let fixture = Fixture::new();
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_input_gate_rejected(&result);
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
}

#[tokio::test]
async fn command_input_gate_rejects_numeric_coercion_time_overflow_and_work_expansion() {
    let mut cases = Vec::new();
    for hours in [json!("24"), json!(24.5), json!(0), json!(-1), json!(169), json!(i64::MAX), json!(u64::MAX)] {
        cases.push(input_gate_widget("errors-by-stack", json!({"hours":hours})));
    }
    cases.extend([
        input_gate_widget("log-tail", json!({"log_group":"/fixture/group", "tail_minutes":i64::MAX})),
        input_gate_widget("log-tail", json!({"log_group":"/fixture/group", "tail_minutes":10081})),
        input_gate_widget("log-tail", json!({"mode":"events","log_group":"/fixture/group","log_stream":"stream-fixture","limit":10001})),
        input_gate_widget("cloudwatch-logs", json!({"mode":"groups","max_groups":"500"})),
        input_gate_widget("cloudwatch-logs", json!({"mode":"streams","log_group":"/fixture/group","max_streams":101})),
        input_gate_widget("logs-insights", json!({"log_group":"/fixture/group","query":"fields @message","range_seconds":604801})),
        input_gate_widget("pipeline-runs", json!({"pipeline_name":"pipeline-fixture","max_results":100.5})),
        input_gate_widget("resource-lookup", json!({"query":"resource-fixture","max_results":1001})),
        input_gate_widget("codeartifact-packages", json!({"max_packages":0})),
        input_gate_widget("codeartifact-package-version-history", json!({"domain":"domain-fixture","repository":"repo-fixture","package":"package-fixture","versions":vec!["1.0.0";11]})),
        input_gate_widget("codeartifact-package-version-history", json!({"domain":"domain-fixture","repository":"repo-fixture","package":"package-fixture","versions":[{"version":"1.0.0","published":42}]})),
    ]);
    for params in cases {
        let fixture = Fixture::new();
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_input_gate_rejected(&result);
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
}

#[tokio::test]
async fn command_input_gate_rejects_selection_and_pipeline_context_shapes_before_resolution() {
    for params in [
        json!([]),
        json!({"profile":"demo-a","region":"us-east-1"}),
        json!({"profile":"demo-a","account_id":ACCOUNT_A,"region":42}),
        json!({"profile":true,"account_id":ACCOUNT_A,"region":"us-east-1"}),
        json!({"profile":"demo-a","account_id":ACCOUNT_A,"region":"us-east-1","sso_session_name":[]}),
        json!({"profile":"demo-a","account_id":ACCOUNT_A,"region":"us-east-1","endpoint":"https://fixture.invalid"}),
        json!({"profile":"demo-a","account_id":ACCOUNT_A,"region":"us-east-1","request_id":"invalid id fixture"}),
    ] {
        let fixture = Fixture::new();
        let result = aws_set_account_impl(&fixture.state, params).await.unwrap();
        assert_input_gate_rejected(&result);
        assert!(fixture.state.current_ctx().is_none());
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
    for params in [
        json!([]),
        json!({"context":{"mode":"pinned","account_id":ACCOUNT_A,"region":"us-east-1"}}),
        json!({"context":{"mode":"inherit","profile":"demo-a"}}),
        json!({"context":{"mode":"unknown-fixture"}}),
        json!({"context":{"mode":"inherit"},"account_override":{"profile":"demo-a","account_id":ACCOUNT_A,"region":"us-east-1"}}),
        json!({"request_id":42}),
        json!({"profile":"ambient-profile-fixture"}),
    ] {
        let fixture = Fixture::new();
        let result = aws_list_pipelines_impl(&fixture.state, params).await.unwrap();
        assert_input_gate_rejected(&result);
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
}

#[test]
fn command_input_gate_invalid_local_mutations_do_not_create_persistence_targets() {
    let settings_cases = [
        json!([]), json!({"default_region":42}), json!({"aws_config_path":null}),
        json!({"aws_config_path":"x".repeat(4097)}),
        json!({"default_profile":"fixture\u{0}profile"}),
        json!({"unreviewed":"synthetic-private-value"}),
    ];
    for params in settings_cases {
        let fixture = Fixture::new();
        assert_input_gate_no_mutation_files(&fixture);
        assert_input_gate_rejected(&settings_set_impl(&fixture.state, params));
        assert_input_gate_no_mutation_files(&fixture);
        assert_input_gate_no_aws_or_process(&fixture);
    }
    let dashboard_cases = [
        json!({"tiles":"not-an-array"}), json!({"tiles":[{"id":"tile-fixture","w":0}]}),
        json!({"tiles":[{"id":"tile-fixture","x":0.5}]}),
        json!({"tiles":[{"id":"tile-fixture"},{"id":"tile-fixture"}]}),
        json!({"tiles":[{"id":"tile-fixture","config":{"context":{"mode":"pinned","profile":"demo-a"}}}]}),
        json!({"tiles":[{"id":"tile-fixture","config":{"inputs":{"pinned_cli_commands":[{"command":"aws sts get-caller-identity","profile":"demo-a","account_id":ACCOUNT_A}]}}}]}),
        json!({"tiles": (0..201).map(|i|json!({"id":format!("tile-fixture-{i}")})).collect::<Vec<_>>()}),
    ];
    for params in dashboard_cases {
        let fixture = Fixture::new();
        assert_input_gate_rejected(&dashboard_set_impl(&fixture.state, params));
        assert_input_gate_no_mutation_files(&fixture);
        assert_input_gate_no_aws_or_process(&fixture);
    }
    for params in [json!({"text":42}), json!({"text":"x".repeat(64*1024+1)}), json!({"text":"Version: 1","extra":true})] {
        let fixture = Fixture::new();
        assert_input_gate_rejected(&policy_set_impl(&fixture.state, params));
        assert_input_gate_no_mutation_files(&fixture);
        assert_input_gate_no_aws_or_process(&fixture);
    }
}

#[test]
fn command_input_gate_rejects_huge_or_malformed_audit_limits() {
    for limit in [json!(u64::MAX), json!(1001), json!(-1), json!("300"), json!(300.5), Value::Null] {
        let fixture = Fixture::new();
        let result = audit_tail_impl(&fixture.state, json!({"limit":limit}));
        assert_input_gate_rejected(&result);
        assert!(result.get("entries").is_none());
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
}

#[tokio::test]
async fn command_input_gate_never_echoes_invalid_private_keys_or_values_into_errors_or_audit() {
    const MARKER: &str = "CB_SYNTHETIC_PRIVATE_INPUT_MARKER_NEVER_REAL";
    let mut extra_field = input_gate_widget("cfn-stacks", json!({}));
    extra_field[MARKER] = json!(MARKER);
    for params in [
        extra_field,
        input_gate_widget(MARKER, json!({})),
        input_gate_widget("cfn-stacks", json!({"name_prefix":{"private":MARKER}})),
        json!({"request_id":format!("{MARKER} invalid"),"widget":"cfn-stacks"}),
    ] {
        let fixture = Fixture::new();
        let result = widget_fetch_impl(&fixture.state, params).await.unwrap();
        assert_input_gate_rejected(&result);
        assert!(!serde_json::to_string(&result).unwrap().contains(MARKER));
        let audit_path = fixture.state.runtime.paths.data_file("audit.log");
        let audit = std::fs::read_to_string(audit_path).expect("isolated rejection audit should exist");
        assert!(!audit.contains(MARKER));
        assert_input_gate_no_aws_or_process(&fixture);
        assert_input_gate_no_mutation_files(&fixture);
    }
    let fixture = Fixture::new();
    let result = settings_set_impl(&fixture.state, json!({MARKER:MARKER}));
    assert_input_gate_rejected(&result);
    assert!(!serde_json::to_string(&result).unwrap().contains(MARKER));
    let audit = std::fs::read_to_string(fixture.state.runtime.paths.data_file("audit.log"))
        .expect("isolated rejection audit should exist");
    assert!(!audit.contains(MARKER));
    assert_input_gate_no_mutation_files(&fixture);
    assert_input_gate_no_aws_or_process(&fixture);
}

#[test]
fn local_tauri_command_capability_and_csp_stay_narrow() {
    let capability: Value = serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
    assert_eq!(capability["windows"], json!(["main"]));
    assert!(capability.get("remote").is_none());
    assert_ne!(capability["local"], false);
    let expected = ["ping", "aws-set-account", "aws-list-profiles", "aws-list-pipelines", "aws-auth-status", "cli-availability", "widget-fetch", "widget-get-source", "settings-get", "settings-set", "dashboard-get", "dashboard-set", "audit-tail", "audit-history", "policy-get", "policy-set", "request-cancel"];
    let permissions = capability["permissions"].as_array().unwrap();
    assert_eq!(permissions.len(), expected.len());
    for command in expected { assert!(permissions.contains(&json!(format!("allow-{command}")))); }
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    assert_eq!(config["app"]["security"]["capabilities"], json!(["default"]));
    let csp = &config["app"]["security"]["csp"];
    for key in ["default-src", "script-src", "font-src"] { assert_eq!(csp[key], "'self'"); }
    for key in ["object-src", "frame-src", "frame-ancestors", "form-action", "base-uri"] { assert_eq!(csp[key], "'none'"); }
    assert_eq!(csp["connect-src"], "ipc: http://ipc.localhost");
}

#[test]
fn cancellation_and_partial_outcome_classes_are_explicit() {
    assert_eq!(crate::request::outcome(&json!({"ok":false,"error_type":"QueryCancelled"})), "cancelled");
    assert_eq!(crate::request::outcome(&json!({"ok":false,"error_type":"QueryTimeout"})), "failed");
    assert_eq!(crate::request::outcome(&json!({"render":"raw_json","data":{"error":"synthetic failure"}})), "failed");
    assert_eq!(crate::request::outcome(&json!({"render":"table","partial":true,"rows":[]})), "failed");
    assert_eq!(crate::request::outcome(&json!({"render":"table","rows":[{"error":"ordinary resource value"}]})), "succeeded");
}
