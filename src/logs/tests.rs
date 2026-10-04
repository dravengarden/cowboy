use super::{
    data::*,
    source::{JsonlSource, LogSource as _},
    storage::*,
};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, InstrumentationScope, any_value},
    logs::v1::LogRecord,
    resource::v1::Resource,
};
use prost::Message as _;
use std::os::unix::fs::PermissionsExt as _;

fn fixture() -> (tempfile::TempDir, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(root.path().join("logs"), true).unwrap();
    (root, store)
}
fn entry(at: i64, index: u64) -> Entry {
    let mut e = Entry::log(
        Resource {
            attributes: vec![text("service.name", "keeper"), text("host.id", "remote")],
            ..Default::default()
        },
        InstrumentationScope {
            name: "cowboy.test".into(),
            ..Default::default()
        },
        LogRecord {
            time_unix_nano: at as u64 * 1_000_000,
            observed_time_unix_nano: at as u64 * 1_000_000,
            severity_number: if index.is_multiple_of(3) { 17 } else { 9 },
            event_name: "cowboy.execution.fixture".into(),
            attributes: vec![
                text("cowboy.session.id", "s1"),
                text("cowboy.execution.environment.id", "env1"),
            ],
            body: Some(AnyValue {
                value: Some(any_value::Value::StringValue(format!("event {index}"))),
            }),
            ..Default::default()
        },
        at,
    );
    e.id = format!("{index:032x}");
    e.duration_ms = Some(index as f64);
    e
}
fn query(from: i64, to: i64) -> Query {
    Query {
        id: None,
        from_ms: from,
        to_ms: to,
        session: None,
        machine: None,
        environment: None,
        trace_id: None,
        event: None,
        service: None,
        signal: None,
        min_severity: 0,
        contains: None,
        limit: 10,
        after: None,
        include_protobuf: false,
    }
}

#[test]
fn pages_remain_stable_across_equal_timestamps_and_reject_foreign_cursors() {
    let (_root, store) = fixture();
    for n in 0..23 {
        store.append(&[entry(1000, n)], 1000).unwrap();
    }
    let mut q = query(0, 2000);
    let first = store.query(&q).unwrap();
    assert_eq!(first.items.len(), 10);
    assert_eq!(first.items[0].id, format!("{:032x}", 22));
    assert!(first.items[0].protobuf.is_empty());
    assert_eq!(first.items[0].attributes["host.id"], "remote");
    q.after = first.next_cursor.clone();
    let second = store.query(&q).unwrap();
    assert_eq!(second.items.len(), 10);
    assert_eq!(second.items[0].id, format!("{:032x}", 12));
    q.after = second.next_cursor;
    let third = store.query(&q).unwrap();
    assert_eq!(third.items.len(), 3);
    assert!(third.next_cursor.is_none());
    let (_other, other) = fixture();
    assert!(other.query(&q).is_err());
    q.session = Some("foreign".into());
    assert!(store.query(&q).is_err());
    let mut exact = query(0, 2000);
    exact.id = Some(format!("{:032x}", 7));
    assert_eq!(store.query(&exact).unwrap().items.len(), 1);
}

#[test]
fn timer_expiry_cleans_idle_records_and_rotation_retains_only_owned_files() {
    let (_root, store) = fixture();
    store
        .configure(Policy {
            retain_seconds: 120,
            rotate_seconds: 60,
            ..Default::default()
        })
        .unwrap();
    store.append(&[entry(1000, 1)], 1000).unwrap();
    store.append(&[entry(62000, 2)], 62000).unwrap();
    std::fs::write(store.directory.join("project-data"), b"untouched").unwrap();
    assert_eq!(store.status().unwrap()["segments"], 2);
    store.maintain(121001).unwrap();
    assert_eq!(store.query(&query(0, 200000)).unwrap().items.len(), 1);
    assert_eq!(store.status().unwrap()["expired_records"], 1);
    store.maintain(182001).unwrap();
    assert!(store.query(&query(0, 200000)).unwrap().items.is_empty());
    assert_eq!(store.status().unwrap()["segments"], 0);
    assert_eq!(
        std::fs::read(store.directory.join("project-data")).unwrap(),
        b"untouched"
    );
}

#[test]
fn sqlite_capacity_rotates_under_flood_and_releases_actual_disk_space() {
    let (_root, store) = fixture();
    store
        .configure(Policy {
            segment_bytes: 1024 * 1024,
            max_bytes: 2 * 1024 * 1024,
            ..Default::default()
        })
        .unwrap();
    for n in 0..200 {
        let mut e = entry(1000, n);
        let mut request = ExportLogsServiceRequest::decode(e.protobuf.as_slice()).unwrap();
        request.resource_logs[0].scope_logs[0].log_records[0]
            .attributes
            .push(text("padding", &"x".repeat(32 * 1024)));
        e.protobuf = request.encode_to_vec();
        store.append(&[e], 1000).unwrap();
    }
    let status = store.status().unwrap();
    assert!(status["retained_bytes"].as_u64().unwrap() <= 2 * 1024 * 1024);
    assert!(status["capacity_evicted_segments"].as_u64().unwrap() > 0);
    store.maintain(8 * 86400 * 1000).unwrap();
    assert_eq!(store.status().unwrap()["retained_bytes"], 0);
}

#[test]
fn concurrent_process_style_writers_keep_all_committed_events() {
    let (_root, store) = fixture();
    let jobs: Vec<_> = (0..4)
        .map(|owner| {
            let store = store.clone();
            std::thread::spawn(move || {
                for n in 0..20 {
                    store.append(&[entry(1000, owner * 20 + n)], 1000).unwrap();
                }
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
    let metrics = store.metrics(&query(0, 2000)).unwrap();
    assert_eq!(metrics.groups.values().map(|s| s.count).sum::<u64>(), 80);
    assert_eq!(
        metrics
            .groups
            .values()
            .map(|s| s.duration_count)
            .sum::<u64>(),
        80
    );
    assert_eq!(metrics.groups.values().map(|s| s.errors).sum::<u64>(), 27);
}

#[test]
fn symlinks_unknown_schemas_and_public_paths_fail_without_modifying_targets() {
    let (root, store) = fixture();
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"private").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        store
            .directory
            .join("otel-0000000000001000-0000000000000000.sqlite"),
    )
    .unwrap();
    assert!(store.maintain(200000).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"private");
    std::fs::remove_file(
        store
            .directory
            .join("otel-0000000000001000-0000000000000000.sqlite"),
    )
    .unwrap();
    let path = store
        .directory
        .join("otel-0000000000001000-0000000000000000.sqlite");
    std::fs::write(&path, b"not a database").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.maintain(200000).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"not a database");
    std::fs::set_permissions(&store.directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SqliteStore::open(store.directory.clone(), false).is_err());
}

#[test]
fn legacy_jsonl_and_standard_otlp_use_the_same_query_projection() {
    let (root, store) = fixture();
    let dir = root.path().join("legacy");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let e = entry(1000, 3);
    let r = ExportLogsServiceRequest::decode(e.protobuf.as_slice()).unwrap();
    let r = &r.resource_logs[0];
    let s = &r.scope_logs[0];
    let row = serde_json::json!({"signal":"logs","resource":r.resource,"scope":s.scope,"record":s.log_records[0]});
    let records = format!("{row}\n");
    store
        .append(&[Entry::legacy(&row, 1000).unwrap()], 1000)
        .unwrap();
    let path = dir.join("telemetry.jsonl");
    std::fs::write(&path, records).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let legacy = JsonlSource { directory: dir };
    let q = query(0, 2000);
    assert_eq!(
        legacy.query(&q).unwrap().items[0].event,
        store.query(&q).unwrap().items[0].event
    );
    assert_eq!(
        legacy
            .metrics(&q)
            .unwrap()
            .groups
            .values()
            .next()
            .unwrap()
            .errors,
        1
    );
    let mut q = q;
    q.include_protobuf = true;
    let page = store.query(&q).unwrap();
    let exported = ExportLogsServiceRequest::decode(page.items[0].protobuf.as_slice()).unwrap();
    assert_eq!(
        exported.resource_logs[0].scope_logs[0].log_records[0],
        s.log_records[0]
    );
}

#[test]
fn missing_evidence_is_not_health_and_metrics_are_not_limited_to_a_log_page() {
    let (_root, store) = fixture();
    assert_eq!(
        super::analysis::analyze(store.metrics(&query(0, 2000)).unwrap(), None)["status"],
        "no_evidence"
    );
    for n in 0..25 {
        store.append(&[entry(1000, n)], 1000).unwrap();
    }
    let mut q = query(1000, 2000);
    q.limit = 1;
    let metrics = store.metrics(&q).unwrap();
    assert_eq!(metrics.groups.values().next().unwrap().count, 25);
    assert!(!metrics.coverage.truncated);
    let analysis = super::analysis::analyze(metrics, None);
    assert!(!analysis["findings"].as_array().unwrap().is_empty());
}

#[test]
fn bad_filters_and_recursive_remote_requests_fail_before_execution() {
    let (_root, store) = fixture();
    let mut q = query(2, 1);
    assert!(store.query(&q).is_err());
    q = query(0, 1000);
    q.limit = 1001;
    assert!(store.query(&q).is_err());
    let source = super::source::Source {
        name: "host".into(),
        directory: store.directory,
        backend: super::source::Backend::Sqlite,
        ssh: Some("hawk".into()),
        ssh_config: None,
        sudo_user: None,
        command: "cowboy".into(),
    };
    assert!(super::source::execute_local(&source, super::source::Operation::Status).is_err());
    let mut source = source;
    source.ssh = Some("-oProxyCommand=bad".into());
    assert!(source.validate().is_err());
    assert_eq!(
        super::source::quote("/tmp/it's $(not-run)"),
        "'/tmp/it'\\''s $(not-run)'"
    );
}

#[test]
fn tracing_evidence_survives_disabled_stderr_filter_and_excludes_bodies() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logs::tests::capture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COWBOY_TEST_LOG_DIRECTORY", root.path().join("logs"))
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let store = SqliteStore::open(root.path().join("logs"), false).unwrap();
    let mut q = query(now_ms() - 60000, now_ms() + 1000);
    q.event = Some("cowboy.execution.fixture_failure".into());
    q.include_protobuf = true;
    let page = store.query(&q).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].session, "session-private");
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains("secret-body"));
    assert!(!serialized.contains("secret-token"));
    assert_eq!(page.items[0].attributes["error.type"], "cursor_expired");
    let status = store.status().unwrap();
    assert_eq!(status["writers"][0]["stopped"], true);
    assert_eq!(status["writers"][0]["failures"], 0);
}

#[test]
#[ignore = "isolated tracing singleton fixture; called by the parent test"]
fn capture_child() {
    let dir = std::env::var_os("COWBOY_TEST_LOG_DIRECTORY").expect("parent-owned fixture");
    let mut context = super::Context::new("cowboy-fixture");
    context.session = "session-private".into();
    let guard = super::init(dir.into(), context).unwrap();
    #[cfg(any(feature = "full", feature = "machine-host"))]
    assert!(!super::forward_runtime());
    tracing::error!(target:"cowboy::fixture",event_name="cowboy.execution.fixture_failure",reason="cursor_expired",token="secret-token","secret-body");
    match std::env::var("COWBOY_TEST_LOG_FAULT").as_deref() {
        Ok("initialization") => {
            let dir =
                std::path::PathBuf::from(std::env::var_os("COWBOY_TEST_LOG_DIRECTORY").unwrap());
            std::fs::write(dir.join("initialized"), b"ready").unwrap();
        }
        Ok("returned_error") => {
            assert!(
                guard
                    .track_outcome()
                    .finish(Err::<(), _>(anyhow::anyhow!("secret-error-body")))
                    .is_err()
            );
            return;
        }
        Ok("early_return") => {
            drop(guard.track_outcome());
            return;
        }
        Ok("queue") => {
            let dir =
                std::path::PathBuf::from(std::env::var_os("COWBOY_TEST_LOG_DIRECTORY").unwrap());
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(dir.join(".logs.lock"))
                .unwrap();
            fs2::FileExt::lock_exclusive(&lock).unwrap();
            for _ in 0..5000 {
                tracing::info!(target:"cowboy::fixture",event_name="cowboy.fixture.noise",bytes=4096);
            }
            tracing::error!(target:"cowboy::fixture",event_name="cowboy.fixture.critical",reason="critical_after_flood");
            fs2::FileExt::unlock(&lock).unwrap();
        }
        Ok("storage") => {
            let dir =
                std::path::PathBuf::from(std::env::var_os("COWBOY_TEST_LOG_DIRECTORY").unwrap());
            std::fs::rename(dir.join("store.json"), dir.join("saved-state.json")).unwrap();
            tracing::error!(target:"cowboy::fixture",event_name="cowboy.fixture.storage_failure");
        }
        _ => {}
    }
    drop(guard);
}

#[test]
fn queue_overflow_keeps_the_reserved_failure_lane_and_reports_lost_evidence() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logs::tests::capture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COWBOY_TEST_LOG_DIRECTORY", root.path().join("logs"))
        .env("COWBOY_TEST_LOG_FAULT", "queue")
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("dropped_records="));
    let store = SqliteStore::open(root.path().join("logs"), false).unwrap();
    let mut q = query(now_ms() - 60000, now_ms() + 1000);
    q.event = Some("cowboy.fixture.critical".into());
    let page = store.query(&q).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(
        page.coverage
            .issues
            .contains(&"writer_reported_queue_loss".into())
    );
    assert!(
        store.status().unwrap()["writers"][0]["dropped"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn storage_failure_is_visible_after_recovery_without_claiming_durability() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("logs");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logs::tests::capture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COWBOY_TEST_LOG_DIRECTORY", &dir)
        .env("COWBOY_TEST_LOG_FAULT", "storage")
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("write/maintenance failed"));
    std::fs::rename(dir.join("saved-state.json"), dir.join("store.json")).unwrap();
    let store = SqliteStore::open(dir, false).unwrap();
    let q = query(now_ms() - 60000, now_ms() + 1000);
    assert!(
        store
            .query(&q)
            .unwrap()
            .coverage
            .issues
            .contains(&"writer_reported_storage_failures".into())
    );
    assert!(
        store.status().unwrap()["writers"][0]["failures"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn standard_metric_points_and_scalar_legacy_attributes_remain_queryable() {
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    let (_root, store) = fixture();
    let e=Entry::legacy(&serde_json::json!({"kind":"metric","name":"queue_size","value":12.5,"occurred_at_ms":1000,"dimensions":{"active":true,"capacity":64}}),1000).unwrap();
    let exported = ExportMetricsServiceRequest::decode(e.protobuf.as_slice()).unwrap();
    assert_eq!(
        exported.resource_metrics[0].scope_metrics[0].metrics[0].name,
        "queue_size"
    );
    store.append(&[e], 1000).unwrap();
    let page = store.query(&query(0, 2000)).unwrap();
    assert!(page.items[0].metric.is_some());
    assert!(
        page.items[0]
            .metric
            .as_ref()
            .unwrap()
            .to_string()
            .contains("12.5")
    );
    let schema = super::cli::command_schema();
    let query = schema["cli"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "query")
        .unwrap();
    assert!(
        query["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["long"] == "session")
    );
}

#[test]
fn interrupted_segment_initialization_is_cleaned_without_touching_unowned_files() {
    let (_root, store) = fixture();
    let partial = store
        .directory
        .join("otel-0000000000001000-0000000000000001.creating");
    std::fs::write(&partial, b"partial").unwrap();
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600)).unwrap();
    let unrelated = store.directory.join("user.creating");
    std::fs::write(&unrelated, b"preserved").unwrap();
    store.maintain(1001).unwrap();
    assert!(!partial.exists());
    assert_eq!(std::fs::read(unrelated).unwrap(), b"preserved");
    store.append(&[entry(1001, 1)], 1001).unwrap();
}

#[test]
fn established_store_startup_tolerates_a_diagnostic_reader_lock() {
    let (_root, store) = fixture();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.directory.join(".logs.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logs::tests::capture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COWBOY_TEST_LOG_DIRECTORY", &store.directory)
        .env("COWBOY_TEST_LOG_FAULT", "initialization")
        .env("RUST_LOG", "off")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !store.directory.join("initialized").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let started_while_locked = store.directory.join("initialized").exists();
    fs2::FileExt::unlock(&lock).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(started_while_locked);
    let mut q = query(now_ms() - 60000, now_ms() + 1000);
    q.event = Some("cowboy.execution.fixture_failure".into());
    assert_eq!(store.query(&q).unwrap().items.len(), 1);
    assert_eq!(store.status().unwrap()["writers"][0]["failures"], 0);
}

#[test]
fn fatal_return_and_unobserved_outcome_leave_safe_failure_evidence() {
    for (mode, event, level) in [
        ("returned_error", "cowboy.process.failed", 17),
        ("early_return", "cowboy.process.outcome_missing", 13),
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logs::tests::capture_child",
                "--ignored",
                "--nocapture",
            ])
            .env("COWBOY_TEST_LOG_DIRECTORY", root.path().join("logs"))
            .env("COWBOY_TEST_LOG_FAULT", mode)
            .env("RUST_LOG", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let store = SqliteStore::open(root.path().join("logs"), false).unwrap();
        let mut q = query(now_ms() - 60000, now_ms() + 1000);
        q.event = Some(event.into());
        let page = store.query(&q).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].severity, level);
        assert!(
            !serde_json::to_string(&page)
                .unwrap()
                .contains("secret-error-body")
        );
    }
}

#[test]
fn historical_metric_time_and_correlation_survive_legacy_projection() {
    use opentelemetry_proto::tonic::metrics::v1::{
        HistogramDataPoint, NumberDataPoint, number_data_point,
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let attrs = vec![
        text("cowboy.machine.id", "ovh"),
        text("cowboy.session.id", "s1"),
    ];
    let sum = NumberDataPoint {
        time_unix_nano: 1_000_000_000,
        attributes: attrs.clone(),
        value: Some(number_data_point::Value::AsInt(3)),
        ..Default::default()
    };
    let histogram = HistogramDataPoint {
        time_unix_nano: 1_000_000_000,
        attributes: attrs,
        count: 1,
        bucket_counts: vec![1],
        ..Default::default()
    };
    let resource = Resource {
        attributes: vec![text("service.name", "old"), text("host.id", "")],
        ..Default::default()
    };
    let mut records = String::new();
    for point in [
        serde_json::json!({"sum":sum}),
        serde_json::json!({"histogram":histogram}),
    ] {
        let row = serde_json::json!({"signal":"metrics","name":"old.metric","resource":resource,"point":point});
        records.push_str(&format!("{row}\n"));
    }
    let path = root.path().join("telemetry.jsonl");
    std::fs::write(&path, records).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut q = query(0, 2000);
    q.machine = Some("ovh".into());
    q.session = Some("s1".into());
    let page = JsonlSource {
        directory: root.path().into(),
    }
    .query(&q)
    .unwrap();
    assert_eq!(page.items.len(), 2);
    for entry in page.items {
        assert_eq!(entry.timestamp_ms, 1000);
        assert_eq!(entry.observed_ms, 1000);
        assert_eq!(entry.attributes["cowboy.machine.id"], "ovh");
    }
}

#[test]
fn analysis_links_to_failure_records_after_a_run_of_successful_events() {
    let (_root, store) = fixture();
    for n in 0..6 {
        let mut e = entry(1000, n);
        e.severity = if n == 0 { 17 } else { 9 };
        store.append(&[e], 1000).unwrap();
    }
    let analysis = super::analysis::analyze(store.metrics(&query(0, 2000)).unwrap(), None);
    assert_eq!(
        analysis["findings"][0]["evidence_ids"],
        serde_json::json!([format!("{:032x}", 0)])
    );
}

#[test]
fn stale_writer_overflow_remains_queryable_and_compacts_with_loss_evidence() {
    let (_root, store) = fixture();
    let now = now_ms();
    store.append(&[entry(now, 1)], now).unwrap();
    for n in 0..4097 {
        let health = serde_json::json!({"schema":1,"instance":format!("{n:032x}"),"service":"fixture","pid":0,"updated_ms":now-60000,"stopped":false,"admitted":1,"written":0,"dropped":0,"failures":0,"pending_bytes":2048});
        let path = store.directory.join(format!("writer-{n:032x}.json"));
        std::fs::write(&path, serde_json::to_vec(&health).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let q = query(now - 120000, now + 1000);
    let before = store.query(&q).unwrap();
    assert_eq!(before.items.len(), 1);
    assert!(
        before
            .coverage
            .issues
            .contains(&"writer_health_inventory_truncated".into())
    );
    store.maintain(now).unwrap();
    let status = store.status().unwrap();
    assert_eq!(status["writers"].as_array().unwrap().len(), 4096);
    assert_eq!(status["writer_history"]["compacted_records"], 1);
    let after = store.query(&q).unwrap();
    assert!(
        after
            .coverage
            .issues
            .contains(&"writer_history_compacted".into())
    );
    assert!(
        after
            .coverage
            .issues
            .contains(&"writer_health_stale".into())
    );
}

#[test]
fn incomplete_shutdown_flush_is_explicit_in_coverage() {
    let (_root, store) = fixture();
    let now = now_ms();
    let health = serde_json::json!({"schema":1,"instance":"00000000000000000000000000000001","service":"fixture","pid":0,"updated_ms":now,"stopped":true,"admitted":1,"written":0,"dropped":0,"failures":0,"pending_bytes":2048});
    atomic_json(
        &store
            .directory
            .join("writer-00000000000000000000000000000001.json"),
        &health,
    )
    .unwrap();
    assert!(
        store
            .query(&query(now - 1000, now + 1000))
            .unwrap()
            .coverage
            .issues
            .contains(&"writer_shutdown_flush_incomplete".into())
    );
}
