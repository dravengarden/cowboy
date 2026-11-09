//! Confirmed exits, fences and admission against unresolved units, the
//! uninstall drain against in-flight starts, attachment and navigation
//! races, and snapshot publication failures.

use super::*;

/// Make the cheap periodic recheck pass against the fixture: pins on this
/// process, an empty fake cgroup tree and both sockets in the runtime
/// directory. The returned listener keeps the display socket bound.
fn recheck_passes(fixture: &Fixture) -> tokio::net::UnixListener {
    for directory in [fixture.cgroup.clone(), fixture.cgroup.join("browser")] {
        std::fs::write(directory.join("cgroup.procs"), "").unwrap();
    }
    let rfb = tokio::net::UnixListener::bind(
        fixture
            .instance
            .runtime_dir
            .join(crate::machine_browser::launcher::RFB_SOCKET),
    )
    .unwrap();
    let own = std::process::id();
    let pin = verify::Pin {
        pid: own,
        start: verify::start_time(own).unwrap(),
    };
    fixture.instance.live.lock().verified = Verified {
        supervisor: pin,
        relay: pin,
        init: pin,
        display_server: pin,
        window_manager: pin,
        browser: pin,
    };
    assert!(fixture.manager.recheck(&fixture.instance).is_ok());
    rfb
}

/// Read the Machine's next launcher command.
async fn next_command(launcher: &mut tokio::net::UnixStream) -> serde_json::Value {
    let mut reader = tokio::io::BufReader::new(launcher);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
        .await
        .expect("no launcher command")
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn reply_ok(launcher: &mut tokio::net::UnixStream, command: &serde_json::Value) {
    let reply = format!(
        "{{\"id\":{},\"ok\":true,\"bytes\":0,\"entries\":0}}\n",
        command["id"]
    );
    launcher.write_all(reply.as_bytes()).await.unwrap();
}

pub(super) fn uplink(
    manager: &Arc<BrowserManager>,
) -> (
    UplinkGuard,
    tokio::sync::mpsc::UnboundedReceiver<MachineEvent>,
) {
    let (events, received) = tokio::sync::mpsc::unbounded_channel();
    let guard = manager.attach(
        events,
        reqwest::Url::parse("https://controller.invalid/").unwrap(),
    );
    (guard, received)
}

pub(super) fn states(
    received: &mut tokio::sync::mpsc::UnboundedReceiver<MachineEvent>,
) -> Vec<StateEvent> {
    let mut states = Vec::new();
    while let Ok(event) = received.try_recv() {
        if let MachineEvent::Browser {
            browser: Event::State(state),
        } = event
        {
            states.push(state);
        }
    }
    states
}

fn open_tab(
    fixture: &Fixture,
    instance_id: &str,
) -> (ProviderAccountId, String, Purpose, Viewport) {
    (
        fixture.account.clone(),
        instance_id.to_owned(),
        Purpose::Tab,
        Viewport {
            width: 1280,
            height: 800,
            scale: 1,
        },
    )
}

fn pend_revocation(fixture: &Fixture, operation_id: &str) -> PathBuf {
    let account_dir = fixture.manager.account_dir(&fixture.account).unwrap();
    let mut record = state_file::read_control(&account_dir).unwrap();
    record.pending = Some(PendingOperation {
        kind: OperationKind::Revoke,
        operation_id: operation_id.into(),
        next_profile_revision: record.profile_revision + 1,
        started_at_ms: unix_ms(),
    });
    state_file::write_control(&account_dir, &record).unwrap();
    account_dir
}

#[tokio::test]
async fn an_unobservable_survivor_keeps_its_record_capacity_fence_and_admission() {
    let fixture = fixture().await;
    *fixture.manager.slot(&fixture.account).instance.lock() = None;
    let record = fixture.instance.record(&fixture.manager.boot_id);
    state_file::write_instance(&fixture.manager.state_dir, &record).unwrap();
    let used = fixture.manager.capacity.used();
    // `systemctl show` failed for it at restart.
    let slot = fixture
        .manager
        .survivor_slot(&record.unit, None, Some(&record.cgroup_dir));
    fixture.manager.retain_recorded(
        &record,
        Some(record.cgroup_dir.clone()),
        Reason::Failed,
        slot,
    );
    assert!(
        fixture
            .manager
            .unconfirmed
            .lock()
            .values()
            .any(|retained| retained.unit == record.unit)
    );
    assert!(fixture.manager.fence_blocked(&fixture.account));
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    // A pending revocation is not completed while the browser may live.
    let account_dir = pend_revocation(&fixture, "op-1");
    fixture.manager.reconcile_accounts().await.unwrap();
    let control = state_file::read_control(&account_dir).unwrap();
    assert!(control.pending.is_some());
    assert_eq!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .len(),
        1
    );
    // Nor is an open admitted (the revocation fence aside).
    fixture.manager.unconfirmed.lock().clear();
    state_file::write_control(&account_dir, &ControlRecord::default()).unwrap();
    fixture
        .manager
        .unconfirmed
        .lock()
        .insert(1, Unconfirmed::unit(&record.unit, Some(&fixture.account)));
    let (account, instance_id, purpose, viewport) = open_tab(&fixture, "inst-2");
    let refused = fixture
        .manager
        .open("o1", account, instance_id, purpose, viewport)
        .await;
    assert_eq!(refused.reason, Some(Reason::Busy));
    assert_eq!(fixture.manager.starts.counts(), (0, 0));
}

#[tokio::test]
async fn an_incomplete_unit_inventory_holds_every_account() {
    let fixture = fixture().await;
    let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert!(!fixture.manager.fence_blocked(&other));
    fixture.manager.retry_inventory();
    assert!(fixture.manager.fence_blocked(&fixture.account));
    assert!(fixture.manager.fence_blocked(&other));
}

#[tokio::test]
async fn an_open_is_refused_while_a_unit_of_the_same_name_is_unconfirmed() {
    let fixture = fixture().await;
    *fixture.manager.slot(&fixture.account).instance.lock() = None;
    let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    fixture.manager.unconfirmed.lock().insert(
        1,
        Unconfirmed::unit(&unit::unit_name("inst-9"), Some(&other)),
    );
    assert!(!fixture.manager.fence_blocked(&fixture.account));
    let (account, instance_id, purpose, viewport) = open_tab(&fixture, "inst-9");
    let refused = fixture
        .manager
        .open("o1", account, instance_id, purpose, viewport)
        .await;
    assert_eq!(refused.reason, Some(Reason::Busy));
}

#[tokio::test]
async fn a_confirmed_retry_reports_the_terminal_state_and_the_fence_completion() {
    let fixture = fixture().await;
    let (_uplink, mut received) = uplink(&fixture.manager);
    states(&mut received);
    fixture.instance.live.lock().phase = Phase::Stopping;
    let account_dir = pend_revocation(&fixture, "op-7");
    fixture.manager.unconfirmed.lock().insert(
        1,
        Unconfirmed::unit(&fixture.instance.unit, Some(&fixture.account)),
    );
    fixture
        .manager
        .release_retained(
            1,
            Retained {
                unit: fixture.instance.unit.clone(),
                cgroup_dir: Some(fixture.cgroup.clone()),
                invocation_id: Some(fixture.instance.invocation_id.clone()),
                account: Some(fixture.account.clone()),
                instance: Some(Arc::clone(&fixture.instance)),
                record: None,
                capacity: None,
                generation: Some(fixture.instance.plan.generation_digest.clone()),
                outcome: StopOutcome {
                    reason: Reason::Revoked,
                    unsaved: true,
                    reference: None,
                },
            },
        )
        .await;
    assert!(fixture.manager.unconfirmed.lock().is_empty());
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .is_none()
    );
    assert_eq!(fixture.manager.capacity.used(), 0);
    let reported = states(&mut received);
    assert_eq!(reported.len(), 2, "{reported:?}");
    let terminal = &reported[0];
    assert_eq!(terminal.state, State::Hibernated);
    assert_eq!(terminal.instance, fixture.instance.reference());
    assert_eq!(terminal.reason, Some(Reason::Revoked));
    assert!(terminal.unsaved_state_lost);
    assert_eq!(terminal.request_id, None);
    let completion = &reported[1];
    let control = state_file::read_control(&account_dir).unwrap();
    assert!(control.pending.is_none());
    assert_eq!(completion.state, State::Hibernated);
    assert_eq!(completion.instance.account_id, fixture.account);
    assert_eq!(completion.instance.incarnation, 0);
    assert_eq!(completion.reason, Some(Reason::Revoked));
    assert_eq!(completion.profile_revision, Some(control.profile_revision));
}

#[tokio::test]
async fn a_drain_waits_for_admitted_starts_and_stops_when_its_authority_ends() {
    let fixture = fixture().await;
    let start = fixture.manager.starts.enter().unwrap();
    let drain = {
        let manager = Arc::clone(&fixture.manager);
        *manager.slot(&fixture.account).instance.lock() = None;
        tokio::spawn(async move { manager.drain_instances(&|| Ok(())).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    // No new start is admitted, and the drain waits for the admitted one.
    assert!(fixture.manager.starts.enter().is_none());
    assert!(fixture.manager.starts.draining());
    assert!(!drain.is_finished());
    drop(start);
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .expect("drain did not settle")
        .unwrap()
        .unwrap();
    assert_eq!(fixture.manager.starts.counts(), (0, 0));

    // An ended lease stops nothing.
    *fixture.manager.slot(&fixture.account).instance.lock() = Some(Arc::clone(&fixture.instance));
    let ended = fixture
        .manager
        .drain_instances(&|| bail!("lease ended"))
        .await;
    assert!(ended.is_err());
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert_eq!(fixture.manager.starts.counts(), (0, 0));
}

#[tokio::test]
async fn an_attach_that_raced_a_stop_does_not_revive_the_instance() {
    let mut fixture = fixture().await;
    let _rfb = recheck_passes(&fixture);
    let (_uplink, _received) = uplink(&fixture.manager);
    let attach = {
        let manager = Arc::clone(&fixture.manager);
        let reference = fixture.instance.reference();
        tokio::spawn(async move {
            manager
                .attach_viewer(
                    "a1",
                    reference,
                    "attachment-1".into(),
                    "token".into(),
                    Viewport {
                        width: 1024,
                        height: 768,
                        scale: 1,
                    },
                )
                .await
        })
    };
    let resize = next_command(&mut fixture.launcher).await;
    assert_eq!(resize["command"], "resize");
    // A reset began while the resize was outstanding.
    fixture.instance.live.lock().phase = Phase::Stopping;
    reply_ok(&mut fixture.launcher, &resize).await;
    let answer = attach.await.unwrap();
    assert_eq!(answer.state, State::Stopping);
    assert_eq!(answer.reason, Some(Reason::Busy));
    let live = fixture.instance.live.lock();
    assert_eq!(live.phase, Phase::Stopping);
    assert!(live.attachment.is_none());
}

#[tokio::test]
async fn a_navigation_waits_for_a_running_copy_and_holds_new_ones() {
    let mut fixture = fixture().await;
    let job = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.snapshot_job(&instance).await })
    };
    let snapshot = next_command(&mut fixture.launcher).await;
    assert_eq!(snapshot["command"], "snapshot");
    let quiesced = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move {
            let _hold = manager.quiesce_copies(&instance).await;
            // While the hold lives no copy is admitted.
            manager
                .snapshot_job(&instance)
                .await
                .unwrap_err()
                .to_string()
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The navigation waits for the running copy rather than verifying
    // beside its publish mount.
    assert!(!quiesced.is_finished());
    reply_ok(&mut fixture.launcher, &snapshot).await;
    job.await.unwrap().unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), quiesced)
        .await
        .expect("navigation did not resume")
        .unwrap();
    assert!(refused.contains("navigation"), "{refused}");
    // Released: copies are admitted again.
    assert!(!fixture.instance.copies_held());
}

#[tokio::test]
async fn a_publication_failed_after_its_rename_is_reconciled_before_release() {
    use std::os::unix::fs::PermissionsExt as _;
    if rustix::process::geteuid().is_root() {
        return; // Permissions cannot make the record unwritable.
    }
    let fixture = fixture().await;
    let account_dir = fixture.manager.account_dir(&fixture.account).unwrap();
    let mut record = state_file::read_control(&account_dir).unwrap();
    let generation = record.allocate_generation();
    state_file::write_control(&account_dir, &record).unwrap();
    let reservation = fixture
        .manager
        .admit_staging(
            &fixture.account,
            &account_dir,
            generation,
            &fixture.instance.settings,
        )
        .unwrap();
    snapshot::create_staging(&account_dir, generation).unwrap();
    std::fs::set_permissions(&account_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let published =
        fixture
            .manager
            .publish_snapshot(&fixture.instance, &account_dir, generation, reservation);
    std::fs::set_permissions(&account_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(published.is_err());
    // The renamed generation is not named by the durable record: it is
    // removed before its budget is released, and nothing is left over.
    let name = state_file::generation_name(generation);
    assert!(
        !state_file::snapshots_root(&account_dir)
            .join(&name)
            .exists()
    );
    assert!(!state_file::staging_root(&account_dir).join(&name).exists());
    assert_eq!(fixture.manager.staging.reserved_bytes(), 0);
    assert!(fixture.manager.leftover_staging.lock().is_empty());
    assert!(fixture.instance.live.lock().last_snapshot.is_none());
}

#[tokio::test]
async fn a_resize_queued_behind_the_control_connection_rechecks_its_attachment() {
    let fixture = fixture().await;
    let _rfb = recheck_passes(&fixture);
    let viewer = |id: &str| Attachment {
        id: id.to_owned(),
        lease_expires: Instant::now() + Duration::from_secs(60),
        stream: None,
    };
    for takeover in [true, false] {
        {
            let mut live = fixture.instance.live.lock();
            live.phase = Phase::Attached;
            live.attachment = Some(viewer("a1"));
        }
        // A snapshot or navigation holds the control connection.
        let held = fixture.instance.control.lock().await;
        let resize = {
            let manager = Arc::clone(&fixture.manager);
            let reference = fixture.instance.reference();
            tokio::spawn(async move { manager.resize("r1", &reference, "a1", 1024, 768).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!resize.is_finished(), "the resize did not queue");
        {
            let mut live = fixture.instance.live.lock();
            if takeover {
                // Another viewer took over meanwhile.
                live.attachment = Some(viewer("a2"));
            } else if let Some(attachment) = live.attachment.as_mut() {
                // The lease expired meanwhile.
                attachment.lease_expires = Instant::now();
            }
        }
        drop(held);
        let answer = tokio::time::timeout(Duration::from_secs(5), resize)
            .await
            .expect("a superseded resize must not reach the launcher")
            .unwrap();
        assert_eq!(answer.reason, Some(Reason::Superseded));
        let geometry = fixture.instance.live.lock().geometry;
        assert_eq!((geometry.width, geometry.height), (1280, 800));
    }
}
