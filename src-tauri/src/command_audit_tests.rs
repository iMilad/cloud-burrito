#[test]
fn activity_history_actions_and_cursor_are_narrow_and_fail_before_io() {
    for params in [
        json!({}),
        json!({"action":"delete"}),
        json!({"action":"preserve","path":"synthetic"}),
        json!({"action":true}),
    ] {
        let fixture = Fixture::new();
        assert_eq!(
            audit_history_impl(&fixture.state, params)["error_type"],
            "InvalidRequest"
        );
        assert!(!fixture
            .state
            .runtime
            .paths
            .data_file("settings.json")
            .exists());
        assert!(!fixture.state.runtime.paths.data_file("audit.1").exists());
        fixture.aws.assert_no_resolution();
        fixture.no_process();
    }
    for cursor in [
        json!("x".repeat(257)),
        json!("synthetic-غ"),
        json!(12),
        json!({"offset":2}),
    ] {
        let fixture = Fixture::new();
        let result = audit_tail_impl(&fixture.state, json!({"cursor":cursor}));
        assert_eq!(result["error_type"], "InvalidRequest");
        assert!(result.get("entries").is_none());
        fixture.aws.assert_no_resolution();
        fixture.no_process();
    }
}

#[tokio::test]
async fn activity_tail_async_has_bounded_cursor_and_reports_only_appended_rows() {
    let fixture = Fixture::new();
    for index in 0..400 {
        fixture
            .state
            .runtime
            .audit(json!({"event":"succeeded","request_id":format!("synthetic-audit-{index}")}));
    }
    let first = audit_tail_async(&fixture.state, json!({})).await;
    assert_eq!(first["ok"], true);
    assert_eq!(first["entries"].as_array().unwrap().len(), 300);
    assert_eq!(first["entries"][0]["request_id"], "synthetic-audit-100");
    assert_eq!(first["entries"][299]["request_id"], "synthetic-audit-399");
    assert_eq!(first["limited"], true);
    assert!(first["bytes_read"].as_u64().unwrap() <= 2 * 1024 * 1024);
    fixture
        .state
        .runtime
        .audit(json!({"event":"succeeded","request_id":"synthetic-appended"}));
    let next = audit_tail_async(&fixture.state, json!({"cursor":first["cursor"],"limit":10})).await;
    assert_eq!(next["entries"].as_array().unwrap().len(), 1);
    assert_eq!(next["entries"][0]["request_id"], "synthetic-appended");
    assert_eq!(next["reset"], false);
    assert_eq!(next["has_more"], false);
    assert!(serde_json::to_vec(&next).unwrap().len() <= 512 * 1024);
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[test]
fn settings_retention_defaults_preserve_and_old_forms_cannot_reset_selected_mode() {
    let fixture = Fixture::new();
    let initial = settings_get_impl(&fixture.state);
    assert_eq!(initial["audit_retention"], "preserve");
    assert_eq!(initial["_audit_retention"]["effective_mode"], "preserve");
    assert!(!fixture
        .state
        .runtime
        .paths
        .data_file("settings.json")
        .exists());
    let mut params = Value::Object(settings::defaults());
    params["audit_retention"] = json!("bounded");
    let saved = settings_set_impl(&fixture.state, params.clone());
    assert_eq!(saved["audit_retention"], "bounded");
    assert_eq!(
        fixture.state.runtime.audit_retention_mode(),
        crate::audit::RetentionMode::Bounded
    );
    params.as_object_mut().unwrap().remove("audit_retention");
    params["theme"] = json!("light");
    let old_form = settings_set_impl(&fixture.state, params.clone());
    assert_eq!(old_form["audit_retention"], "bounded");
    assert_eq!(old_form["theme"], "light");
    let persisted = std::fs::read(fixture.state.runtime.paths.data_file("settings.json")).unwrap();
    for invalid in [json!("delete"), json!(""), json!(true), json!(null)] {
        params["audit_retention"] = invalid;
        assert_eq!(
            settings_set_impl(&fixture.state, params.clone())["error_type"],
            "InvalidRequest"
        );
        assert_eq!(
            std::fs::read(fixture.state.runtime.paths.data_file("settings.json")).unwrap(),
            persisted
        );
        assert_eq!(
            fixture.state.runtime.audit_retention_mode(),
            crate::audit::RetentionMode::Bounded
        );
    }
    let mut bad_saved = Value::Object(settings::defaults());
    bad_saved["audit_retention"] = json!("unrecognized-mode");
    fixture
        .state
        .runtime
        .storage
        .write_json("settings.json", &bad_saved)
        .unwrap();
    assert_eq!(
        settings_get_impl(&fixture.state)["error_type"],
        "StorageInvalid"
    );
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn oversized_history_requires_explicit_preservation_before_bounded_retention() {
    let fixture = Fixture::new();
    let path = fixture.state.runtime.paths.data_file("audit.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(10 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let before = std::fs::metadata(&path).unwrap().len();
    let status = audit_history_async(&fixture.state, json!({"action":"status"})).await;
    assert_eq!(status["mode"], "preserve");
    assert_eq!(status["preserve_required"], true);
    assert_eq!(status["total_bytes"], before);
    let mut params = Value::Object(settings::defaults());
    params["audit_retention"] = json!("bounded");
    assert_eq!(
        settings_set_impl(&fixture.state, params.clone())["error_type"],
        "AuditPreserveRequired"
    );
    assert!(!fixture
        .state
        .runtime
        .paths
        .data_file("settings.json")
        .exists());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);

    // A legacy saved preference cannot activate pruning merely by loading it.
    fixture
        .state
        .runtime
        .storage
        .write_json("settings.json", &params)
        .unwrap();
    let loaded = settings_get_impl(&fixture.state);
    assert_eq!(loaded["audit_retention"], "bounded");
    assert_eq!(loaded["_audit_retention"]["effective_mode"], "preserve");
    assert_eq!(loaded["_audit_retention"]["preserve_required"], true);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    let preserved = audit_history_async(&fixture.state, json!({"action":"preserve"})).await;
    assert_eq!(preserved["ok"], true);
    assert_eq!(preserved["preserved_files"], 1);
    let location = std::path::Path::new(preserved["preserved_location"].as_str().unwrap());
    assert!(location.starts_with(fixture._dir.path()));
    assert_eq!(
        std::fs::metadata(location.join("audit.log")).unwrap().len(),
        before
    );
    assert!(!path.exists());
    assert_eq!(
        settings_set_impl(&fixture.state, params)["audit_retention"],
        "bounded"
    );
    assert_eq!(
        fixture.state.runtime.audit_retention_mode(),
        crate::audit::RetentionMode::Bounded
    );
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}

#[tokio::test]
async fn activity_preservation_refuses_unconfirmed_writes_and_keeps_history() {
    let fixture = Fixture::new();
    let path = fixture.state.runtime.paths.data_file("audit.log");
    std::fs::create_dir_all(&path).unwrap();
    fixture
        .state
        .runtime
        .audit(json!({"event":"synthetic-write-failure"}));
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"synthetic existing history\n").unwrap();
    let result = audit_history_async(&fixture.state, json!({"action":"preserve"})).await;
    assert_eq!(result["error_type"], "AuditFlushFailed");
    assert_eq!(result["_diagnostics"]["audit_write_failed"], true);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"synthetic existing history\n"
    );
    assert!(result.get("preserved_location").is_none());
}

#[test]
fn failed_settings_write_keeps_the_previous_retention_mode() {
    let fixture = Fixture::new();
    let path = fixture.state.runtime.paths.data_file("settings.json");
    std::fs::create_dir_all(&path).unwrap();
    let mut params = Value::Object(settings::defaults());
    params["audit_retention"] = json!("bounded");
    let result = settings_set_impl(&fixture.state, params);
    assert_eq!(result["error_type"], "StorageWriteFailed");
    assert_eq!(
        fixture.state.runtime.audit_retention_mode(),
        crate::audit::RetentionMode::Preserve
    );
    assert!(path.is_dir());
}

#[test]
fn concurrent_settings_saves_and_preservation_leave_a_consistent_selected_mode() {
    let fixture = Fixture::new();
    fixture
        .state
        .runtime
        .audit(json!({"event":"succeeded","request_id":"synthetic-preserve-race"}));
    std::thread::scope(|scope| {
        let barrier = Arc::new(std::sync::Barrier::new(3));
        for mode in ["bounded", "preserve"] {
            let barrier = barrier.clone();
            let state = &fixture.state;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..16 {
                    let mut params = Value::Object(settings::defaults());
                    params["audit_retention"] = json!(mode);
                    assert_eq!(settings_set_impl(state, params)["audit_retention"], mode);
                }
            });
        }
        let state = &fixture.state;
        scope.spawn(move || {
            barrier.wait();
            let result = audit_history_impl(state, json!({"action":"preserve"}));
            assert_eq!(result["ok"], true);
            assert_eq!(result["preserved_files"], 1);
        });
    });
    let saved = settings::load(&fixture.state.runtime.storage).unwrap();
    assert_eq!(
        saved["audit_retention"],
        fixture.state.runtime.audit_retention_mode().as_str()
    );
    fixture.aws.assert_no_resolution();
    fixture.no_process();
}
