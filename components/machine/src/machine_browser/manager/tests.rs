//! Manager tests against synthetic instances: a real control socket served by
//! the test as a fake launcher, a fake cgroup directory, and a Plugin store in
//! a temporary state root. Nothing here reaches systemd.

use super::*;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

mod confirmation;
mod fake_units;
mod idle_and_navigation;
mod inventory;
mod retention;

use fake_units::{FakeUnit, FakeUnits};

struct Fixture {
    _temp: tempfile::TempDir,
    manager: Arc<BrowserManager>,
    /// The synthetic systemd the manager lists, observes and stops units
    /// through; the fixture's unit is live and never confirms its exit.
    units: Arc<FakeUnits>,
    account: ProviderAccountId,
    /// The fake launcher's side of the control connection.
    launcher: tokio::net::UnixStream,
    cgroup: PathBuf,
    instance: Arc<Instance>,
}

fn plan() -> MachineBrowserRuntimePlan {
    let contract: cowboy_plugin_sdk::MachineBrowserContract = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins/machine-browser/contract.json"),
        )
        .unwrap(),
    )
    .unwrap();
    MachineBrowserRuntimePlan {
        plugin_id: "machine-browser".into(),
        plugin_version: contract.version.clone(),
        generation_digest: "sha256:aa".into(),
        command: "cowboy-browser-launcher".into(),
        runtime_root: PathBuf::from("/nonexistent/runtime"),
        launcher: PathBuf::from("/nonexistent/runtime/usr/libexec/cowboy-browser-launcher"),
        contract,
    }
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let store = MachinePluginStore::new(
        &state,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let units = Arc::new(FakeUnits::default());
    units.set(
        "cowboy-browser-inst-1",
        FakeUnit::live("invocation").unstoppable(),
    );
    let manager = BrowserManager::build(
        &state,
        Arc::new(store),
        None,
        || None,
        Arc::clone(&units) as Arc<dyn UnitControl>,
    )
    .unwrap();
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let revision = state_file::read_control(&manager.account_dir(&account).unwrap())
        .unwrap()
        .profile_revision;
    let runtime_dir = temp.path().join("run");
    std::fs::create_dir_all(&runtime_dir).unwrap();
    let listener =
        tokio::net::UnixListener::bind(runtime_dir.join(super::super::launcher::CONTROL_SOCKET))
            .unwrap();
    let (control, accepted) = tokio::join!(
        Control::connect(&runtime_dir, Duration::from_secs(5)),
        listener.accept()
    );
    let cgroup = temp.path().join("cgroup");
    std::fs::create_dir_all(cgroup.join("browser")).unwrap();
    std::fs::write(
        cgroup.join("browser/cgroup.events"),
        "populated 1\nfrozen 1\n",
    )
    .unwrap();
    let mut settings = Settings::from_config(None);
    settings.disk_low_watermark = 0;
    let instance = Arc::new(Instance::new(InstanceParts {
        account_id: account.clone(),
        instance_id: "inst-1".into(),
        incarnation: 7,
        purpose: Purpose::Tab,
        deadline_ms: unix_ms() + 600_000,
        unit: "cowboy-browser-inst-1".into(),
        runtime_dir,
        cgroup_dir: cgroup.clone(),
        invocation_id: "invocation".into(),
        scale: 1,
        profile_revision: revision,
        plan: Arc::new(plan()),
        settings,
        report: ReadyReport::default(),
        control: control.unwrap(),
        phase: Phase::Detached,
        geometry: Geometry {
            width: 1280,
            height: 800,
        },
        last_snapshot: None,
        capacity: manager.capacity.try_reserve(1),
    }));
    *manager.slot(&account).instance.lock() = Some(Arc::clone(&instance));
    Fixture {
        _temp: temp,
        manager,
        units,
        account,
        launcher: accepted.unwrap().0,
        cgroup,
        instance,
    }
}

/// Read the Machine's next command, failing if the job ends first.
async fn read_command(
    launcher: &mut tokio::net::UnixStream,
    job: &mut tokio::task::JoinHandle<Result<()>>,
) -> serde_json::Value {
    let mut reader = tokio::io::BufReader::new(launcher);
    let mut line = String::new();
    tokio::select! {
        read = reader.read_line(&mut line) => {
            read.unwrap();
        }
        ended = job => panic!("snapshot job ended before its command: {ended:?}"),
    }
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn rejected_lease_renewals_release_the_live_lock_first() {
    let fixture = fixture().await;
    let reference = fixture.instance.reference();
    let manager = Arc::clone(&fixture.manager);
    // No attachment, then a stale one: both answer instead of deadlocking.
    let answered = tokio::task::spawn_blocking(move || {
        let none = manager.lease("r1", &reference, "a1", unix_ms() + 10_000);
        manager.current(&reference).unwrap().live.lock().attachment = Some(Attachment {
            id: "a2".into(),
            lease_expires: Instant::now() + Duration::from_secs(30),
            stream: None,
        });
        let stale = manager.lease("r2", &reference, "a1", unix_ms() + 10_000);
        let renewed = manager.lease("r3", &reference, "a2", unix_ms() + 10_000);
        (none, stale, renewed)
    });
    let (none, stale, renewed) = tokio::time::timeout(Duration::from_secs(5), answered)
        .await
        .expect("lease rejection deadlocked")
        .unwrap();
    assert_eq!(none.unwrap().reason, Some(Reason::Superseded));
    assert_eq!(stale.unwrap().reason, Some(Reason::Superseded));
    assert!(renewed.is_none());
}

#[tokio::test]
async fn a_refused_copy_removes_its_staging_and_releases_the_budget() {
    let mut fixture = fixture().await;
    let mut job = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.snapshot_job(&instance).await })
    };
    let command = read_command(&mut fixture.launcher, &mut job).await;
    assert_eq!(command["command"], "snapshot");
    let generation = command["generation"].as_u64().unwrap();
    let staging = state_file::staging_root(&fixture.manager.account_dir(&fixture.account).unwrap())
        .join(state_file::generation_name(generation));
    assert!(staging.is_dir());
    assert_eq!(
        std::fs::read_to_string(fixture.cgroup.join("browser/cgroup.freeze")).unwrap(),
        "1"
    );
    let reply = format!(
        "{{\"id\":{},\"ok\":false,\"error\":\"copy failed\",\"kind\":\"internal\"}}\n",
        command["id"]
    );
    fixture.launcher.write_all(reply.as_bytes()).await.unwrap();
    assert!(job.await.unwrap().is_err());
    // Thawed, removed, released; nothing left for a stop to clean.
    assert_eq!(
        std::fs::read_to_string(fixture.cgroup.join("browser/cgroup.freeze")).unwrap(),
        "0"
    );
    assert!(!staging.exists());
    assert_eq!(fixture.manager.staging.reserved_bytes(), 0);
    assert!(fixture.instance.abandoned.lock().is_none());
    assert!(!fixture.instance.copy_running());
}

#[tokio::test]
async fn a_cancelled_copy_keeps_its_staging_reserved_until_confirmed_exit() {
    let mut fixture = fixture().await;
    let mut job = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.snapshot_job(&instance).await })
    };
    // The launcher never answers: the copier may still be writing.
    let command = read_command(&mut fixture.launcher, &mut job).await;
    let generation = command["generation"].as_u64().unwrap();
    let account_dir = fixture.manager.account_dir(&fixture.account).unwrap();
    let staging =
        state_file::staging_root(&account_dir).join(state_file::generation_name(generation));
    assert!(fixture.instance.copy_running());
    // Supervision is not blocked by the copy and a stop cancels it at once.
    fixture.instance.cancel_copy();
    assert!(fixture.instance.copy_released().await);
    assert!(job.await.unwrap().is_err());
    assert!(staging.is_dir());
    assert!(fixture.manager.staging.reserved_bytes() > 0);
    assert_eq!(
        fixture
            .instance
            .abandoned
            .lock()
            .as_ref()
            .map(|(generation, _)| *generation),
        Some(generation)
    );
    // Another snapshot of the account is refused while the staging is held.
    assert!(
        fixture
            .manager
            .admit_staging(
                &fixture.account,
                &account_dir,
                generation + 1,
                &fixture.instance.settings
            )
            .is_err()
    );
    // Confirmed exit: the staging is removed, then budget and slot released.
    assert_eq!(fixture.manager.capacity.used(), 1);
    fixture.manager.finish_stopped(&fixture.instance);
    assert!(!staging.exists());
    assert_eq!(fixture.manager.staging.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), 0);
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .is_none()
    );
}

#[tokio::test]
async fn an_unconfirmed_unit_blocks_its_account_fence() {
    let fixture = fixture().await;
    assert!(!fixture.manager.fence_blocked(&fixture.account));
    fixture.manager.unconfirmed.lock().insert(
        1,
        Unconfirmed::unit("cowboy-browser-inst-1", Some(&fixture.account)),
    );
    assert!(fixture.manager.fence_blocked(&fixture.account));
    let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert!(!fixture.manager.fence_blocked(&other));
    // An unrecorded unit may belong to any account.
    fixture
        .manager
        .unconfirmed
        .lock()
        .insert(2, Unconfirmed::unit("cowboy-browser-x", None));
    assert!(fixture.manager.fence_blocked(&other));
}

#[test]
fn completing_a_fence_deletes_the_profile_and_lifts_the_writer_version() {
    for kind in [OperationKind::Reset, OperationKind::Revoke] {
        let temp = tempfile::tempdir().unwrap();
        let account = temp.path();
        let mut record = ControlRecord {
            browser_version: Some("144.0.1.1".into()),
            generation_digest: Some("sha256:bb".into()),
            current_generation: Some(3),
            current_generation_at_ms: Some(1),
            pending: Some(PendingOperation {
                kind,
                operation_id: "op".into(),
                next_profile_revision: 5,
                started_at_ms: 1,
            }),
            ..ControlRecord::default()
        };
        std::fs::create_dir_all(state_file::snapshots_root(account).join("gen-3")).unwrap();
        complete_fenced(account, &mut record).unwrap();
        assert!(!state_file::browser_root(account).exists());
        let stored = state_file::read_control(account).unwrap();
        assert_eq!(stored.profile_revision, 5);
        assert_eq!(stored.browser_version, None);
        assert_eq!(stored.generation_digest, None);
        assert!(stored.pending.is_none());
        // An older runtime opens the reset profile instead of refusing it.
        assert_eq!(
            compare_versions(stored.browser_version.as_deref(), "143.0.7499.40"),
            VersionOrder::Unrecorded
        );
    }
}

fn login(account: &ProviderAccountId, expires_at_ms: i64) -> LoginSession {
    let (cancel, _) = tokio::sync::watch::channel(false);
    let (input, _) = tokio::sync::mpsc::channel(1);
    LoginSession {
        cancel,
        input,
        candidate: None,
        account: Some(account.clone()),
        provider: "claude-code".into(),
        expires_at_ms,
        verification_url: None,
        browser: None,
    }
}

#[tokio::test]
async fn a_sign_in_open_binds_the_running_browser_or_refuses() {
    let fixture = fixture().await;
    let logins: LoginSessions = Arc::default();
    let (events, _received) = tokio::sync::mpsc::unbounded_channel();
    let _uplink = fixture.manager.attach(
        events,
        reqwest::Url::parse("https://controller.invalid/").unwrap(),
        &logins,
    );
    let expiry = unix_ms() + 60_000;
    logins
        .lock()
        .insert("login-1".into(), login(&fixture.account, expiry));
    let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    logins
        .lock()
        .insert("login-2".into(), login(&other, expiry));
    let open = |login: &str| Command::Open {
        account_id: fixture.account.clone(),
        instance_id: "inst-1".into(),
        purpose: Purpose::SignIn {
            login_request_id: login.into(),
        },
        viewport: Viewport {
            width: 1280,
            height: 800,
            scale: 1,
        },
    };
    let Command::Open {
        account_id,
        instance_id,
        purpose,
        viewport,
    } = open("login-2")
    else {
        unreachable!()
    };
    let refused = fixture
        .manager
        .open("o1", account_id, instance_id, purpose, viewport)
        .await;
    assert_eq!(refused.reason, Some(Reason::LoginMismatch));
    assert_eq!(fixture.instance.purpose(), Purpose::Tab);
    assert!(logins.lock()["login-2"].browser.is_none());

    let Command::Open {
        account_id,
        instance_id,
        purpose,
        viewport,
    } = open("login-1")
    else {
        unreachable!()
    };
    let bound = fixture
        .manager
        .open("o2", account_id, instance_id, purpose, viewport)
        .await;
    assert_eq!(bound.reason, None);
    assert_eq!(bound.deadline_ms, Some(expiry));
    assert_eq!(fixture.instance.login_request().as_deref(), Some("login-1"));
    assert_eq!(
        logins.lock()["login-1"].browser,
        Some(("inst-1".into(), fixture.instance.incarnation))
    );
    assert!(
        fixture
            .manager
            .login_binding_broken(&fixture.instance)
            .is_none()
    );
    // The durable record now says sign-in, so a restart stops it.
    let records = state_file::read_instances(&fixture.manager.state_dir).unwrap();
    assert!(matches!(
        records.first().map(|record| &record.purpose),
        Some(Purpose::SignIn { .. })
    ));
}

#[tokio::test]
async fn an_uninstall_drain_holds_opens_and_refuses_unconfirmed_exits() {
    let fixture = fixture().await;
    // The fixture's instance is not a real unit; forget it for this test.
    *fixture.manager.slot(&fixture.account).instance.lock() = None;
    let draining = || fixture.manager.starts.counts().0;
    let hold = fixture.manager.drain_instances(&|| Ok(())).await.unwrap();
    assert_eq!(draining(), 1);
    assert!(fixture.manager.starts.enter().is_none());
    drop(hold);
    assert_eq!(draining(), 0);
    fixture
        .manager
        .unconfirmed
        .lock()
        .insert(1, Unconfirmed::unit("cowboy-browser-x", None));
    assert!(fixture.manager.drain_instances(&|| Ok(())).await.is_err());
    assert_eq!(draining(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_account_s_lifecycle_commands_run_in_arrival_order() {
    let fixture = fixture().await;
    *fixture.manager.slot(&fixture.account).instance.lock() = None;
    let logins: LoginSessions = Arc::default();
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let _uplink = fixture.manager.attach(
        events,
        reqwest::Url::parse("https://controller.invalid/").unwrap(),
        &logins,
    );
    for index in 0..24 {
        let command = if index % 3 == 0 {
            Command::Query {
                account_id: fixture.account.clone(),
            }
        } else {
            Command::Revoke {
                account_id: fixture.account.clone(),
                operation_id: format!("op-{index}"),
            }
        };
        fixture.manager.dispatch(format!("r{index:02}"), command);
    }
    let mut order = Vec::new();
    while order.len() < 24 {
        let event = tokio::time::timeout(Duration::from_secs(10), received.recv())
            .await
            .unwrap()
            .unwrap();
        if let MachineEvent::Browser {
            browser: Event::State(state),
        } = event
            && let Some(request) = state.request_id
        {
            order.push(request);
        }
    }
    let expected: Vec<String> = (0..24).map(|index| format!("r{index:02}")).collect();
    assert_eq!(order, expected);
    let record =
        state_file::read_control(&fixture.manager.account_dir(&fixture.account).unwrap()).unwrap();
    assert_eq!(record.last_operation.unwrap().operation_id, "op-23");
}
