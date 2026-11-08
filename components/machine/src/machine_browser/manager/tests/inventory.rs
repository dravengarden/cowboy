//! Restart inventory against a synthetic systemd: the inventory-wide fence,
//! replacement invocations, recovered records, `Query` while a browser may
//! still run, attachment expiry against a stop, and the uninstall drain's
//! authority after its waits.

use super::*;

const UNIT: &str = "cowboy-browser-inst-1";

/// Forget the fixture's registered instance: these tests reconcile from
/// records and the unit listing alone.
fn unregistered(fixture: &Fixture) {
    *fixture.manager.slot(&fixture.account).instance.lock() = None;
}

/// A record of this boot for `account`'s instance `instance_id` running
/// invocation `invocation`.
fn record(
    fixture: &Fixture,
    account: &ProviderAccountId,
    instance_id: &str,
    invocation: &str,
    purpose: Purpose,
) -> InstanceRecord {
    let mut record = fixture.instance.record(&fixture.manager.boot_id);
    record.account_id = account.as_str().to_owned();
    record.instance_id = instance_id.to_owned();
    record.unit = unit::unit_name(instance_id);
    record.invocation_id = invocation.to_owned();
    record.purpose = purpose;
    record
}

fn sign_in() -> Purpose {
    Purpose::SignIn {
        login_request_id: "login-1".into(),
    }
}

fn revoke_pending(manager: &BrowserManager, account: &ProviderAccountId, operation: &str) {
    let account_dir = manager.account_dir(account).unwrap();
    let mut control = state_file::read_control(&account_dir).unwrap();
    control.pending = Some(PendingOperation {
        kind: OperationKind::Revoke,
        operation_id: operation.into(),
        next_profile_revision: control.profile_revision + 1,
        started_at_ms: unix_ms(),
    });
    state_file::write_control(&account_dir, &control).unwrap();
}

fn pending(manager: &BrowserManager, account: &ProviderAccountId) -> bool {
    state_file::read_control(&manager.account_dir(account).unwrap())
        .unwrap()
        .pending
        .is_some()
}

async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let waited = Instant::now();
    while !condition() {
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn other_account() -> ProviderAccountId {
    uuid::Uuid::new_v4().to_string().try_into().unwrap()
}

#[tokio::test]
async fn a_replacement_of_a_recorded_invocation_is_stopped_as_unrecorded() {
    let fixture = fixture().await;
    unregistered(&fixture);
    let stale = record(&fixture, &fixture.account, "inst-1", "old", Purpose::Tab);
    state_file::write_instance(&fixture.manager.state_dir, &stale).unwrap();
    // The recorded invocation ended; another one runs under the name.
    fixture
        .units
        .set(UNIT, FakeUnit::live("replacement").unstoppable());
    fixture.units.list(Some(&[UNIT]));
    let used = fixture.manager.capacity.used();
    fixture.manager.reconcile_units().await.unwrap();
    assert!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .units
            .terminations()
            .contains(&(UNIT.to_owned(), Some("replacement".to_owned())))
    );
    // Its exit is unconfirmed: it is retained, bound to its own invocation,
    // counted, and holds every account's fence and opens.
    {
        let unconfirmed = fixture.manager.unconfirmed.lock();
        let retained: Vec<_> = unconfirmed.values().collect();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].unit, UNIT);
        assert_eq!(retained[0].invocation_id.as_deref(), Some("replacement"));
        assert_eq!(retained[0].account, None);
    }
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert!(!fixture.manager.inventory_unresolved());
    assert!(fixture.manager.fence_blocked(&fixture.account));
    assert!(fixture.manager.fence_blocked(&other_account()));
    let query = fixture.manager.query("q1", &fixture.account);
    assert_eq!(query.state, State::Stopping);
    assert_eq!(query.reason, Some(Reason::Busy));
    assert_eq!(query.instance.incarnation, 0);
    // Its own confirmed exit releases everything.
    fixture.units.make_stoppable(UNIT);
    eventually("the replacement's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert_eq!(fixture.manager.capacity.used(), used);
    assert!(!fixture.manager.fence_blocked(&fixture.account));
    let query = fixture.manager.query("q2", &fixture.account);
    assert_eq!(query.state, State::Hibernated);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_inventory_fence_holds_every_account_until_the_whole_inventory_is_accounted_for() {
    let fixture = fixture().await;
    unregistered(&fixture);
    let other = other_account();
    revoke_pending(&fixture.manager, &other, "op-b");
    let survivor = record(&fixture, &other, "inst-b", "ib", sign_in());
    state_file::write_instance(&fixture.manager.state_dir, &survivor).unwrap();
    fixture.units.set(&survivor.unit, FakeUnit::live("ib"));
    fixture.units.list(Some(&[survivor.unit.as_str()]));
    let gate = fixture.units.gate(&survivor.unit);
    let startup = {
        let manager = Arc::clone(&fixture.manager);
        tokio::spawn(async move { manager.reconcile_at_start().await })
    };
    tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .expect("inventory never reached the survivor");
    // The other account's browser is not classified yet: a termination
    // retry finishing now (it resumes fenced operations) completes nothing,
    // no account admits an open, and no query reports absence.
    assert!(fixture.manager.inventory_unresolved());
    fixture.manager.reconcile_accounts().await.unwrap();
    assert!(pending(&fixture.manager, &other));
    assert!(fixture.manager.fence_blocked(&fixture.account));
    let (account, instance_id, purpose, viewport) = (
        fixture.account.clone(),
        "inst-3".to_owned(),
        Purpose::Tab,
        Viewport {
            width: 1280,
            height: 800,
            scale: 1,
        },
    );
    let refused = fixture
        .manager
        .open("o1", account, instance_id, purpose, viewport)
        .await;
    assert_eq!(refused.reason, Some(Reason::Busy));
    let query = fixture.manager.query("q1", &other);
    assert_eq!(query.state, State::Stopping);
    assert_eq!(query.reason, Some(Reason::Busy));
    gate.release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), startup)
        .await
        .expect("startup reconciliation did not finish")
        .unwrap();
    // The sign-in survivor was stopped (confirmed), then the fence lowered
    // and the revocation completed.
    assert!(!fixture.units.exists(&survivor.unit));
    assert!(!fixture.manager.inventory_unresolved());
    assert!(!pending(&fixture.manager, &other));
    assert!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn records_recovered_by_a_retried_inventory_are_reconciled_not_exempted() {
    let fixture = fixture().await;
    unregistered(&fixture);
    // Neither the records nor the unit listing are readable.
    let records_root = state_file::instances_root(&fixture.manager.state_dir);
    let _ = std::fs::remove_dir_all(&records_root);
    std::fs::write(&records_root, "not a directory").unwrap();
    fixture.units.list(None);
    assert!(fixture.manager.reconcile_units().await.is_err());
    assert!(fixture.manager.inventory_unresolved());
    let other = other_account();
    assert!(fixture.manager.fence_blocked(&other));
    // An unresolved inventory never reports absence.
    let query = fixture.manager.query("q1", &fixture.account);
    assert_eq!(query.state, State::Stopping);
    assert_eq!(query.reason, Some(Reason::Busy));
    assert_eq!(query.instance.instance_id, "");
    // Both recover: a sign-in survivor whose exit cannot be confirmed.
    std::fs::remove_file(&records_root).unwrap();
    let survivor = record(
        &fixture,
        &fixture.account,
        "inst-1",
        "invocation",
        sign_in(),
    );
    state_file::write_instance(&fixture.manager.state_dir, &survivor).unwrap();
    fixture.units.list(Some(&[UNIT]));
    eventually("a retried inventory completes", || {
        !fixture.manager.inventory_unresolved()
    })
    .await;
    // It was reconciled (stopped, bound to its recorded invocation), not
    // exempted; it is retained with its record, slot and account fence.
    assert!(
        fixture
            .units
            .terminations()
            .contains(&(UNIT.to_owned(), Some("invocation".to_owned())))
    );
    assert_eq!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .len(),
        1
    );
    assert!(fixture.manager.fence_blocked(&fixture.account));
    assert!(!fixture.manager.fence_blocked(&other));
    // `Query` reports the retained incarnation as stopping.
    let reference = InstanceRef {
        account_id: fixture.account.clone(),
        instance_id: "inst-1".into(),
        incarnation: survivor.incarnation,
    };
    let query = fixture.manager.query("q2", &fixture.account);
    assert_eq!(query.state, State::Stopping);
    assert_eq!(query.instance, reference);
    assert_eq!(query.reason, Some(Reason::LoginEnded));
    assert_eq!(query.request_id.as_deref(), Some("q2"));
    // So does a command naming it.
    let hibernate = fixture.manager.hibernate("h1", &reference).await;
    assert_eq!(hibernate.state, State::Stopping);
    assert_eq!(hibernate.instance, reference);
    // A retried listing never re-exempts it: the next pass skips only the
    // retained record, and its confirmed exit reports the terminal state.
    let (_uplink, mut received, _logins) = super::confirmation::uplink(&fixture.manager);
    super::confirmation::states(&mut received);
    fixture.units.make_stoppable(UNIT);
    eventually("the survivor's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .is_empty()
    );
    let reported = super::confirmation::states(&mut received);
    assert!(
        reported
            .iter()
            .any(|state| state.instance == reference && state.state == State::Hibernated),
        "{reported:?}"
    );
    assert_eq!(
        fixture.manager.query("q3", &fixture.account).state,
        State::Hibernated
    );
}

#[tokio::test]
async fn an_unobservable_unrecorded_unit_is_stopped_and_a_claimed_one_keeps_the_fence() {
    let fixture = fixture().await;
    unregistered(&fixture);
    let stray = unit::unit_name("stray");
    fixture
        .units
        .set(&stray, FakeUnit::live("s1").unobservable());
    fixture.units.list(Some(&[stray.as_str()]));
    // Unclaimed and unobservable: retained unbound, whatever runs under it.
    fixture.manager.reconcile_units().await.unwrap();
    assert!(
        fixture
            .units
            .terminations()
            .contains(&(stray.clone(), None))
    );
    assert!(fixture.manager.unit_claimed(&stray));
    // Now claimed: a pass that cannot observe it stays incomplete.
    assert!(fixture.manager.inventory_pass().await.is_err());
}

#[tokio::test]
async fn an_expired_lease_never_detaches_a_stopping_or_renewed_attachment() {
    let fixture = fixture().await;
    let now = Instant::now();
    let expired = |id: &str| Attachment {
        id: id.into(),
        lease_expires: now - Duration::from_secs(1),
        stream: None,
    };
    // A stop took the attachment after the tick saw it expired.
    fixture.instance.live.lock().phase = Phase::Stopping;
    assert!(!supervision::expire_attachment(
        &fixture.instance,
        "a1",
        now
    ));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Stopping);
    // Even one still holding it is never moved back to detached.
    fixture.instance.live.lock().attachment = Some(expired("a1"));
    assert!(!supervision::expire_attachment(
        &fixture.instance,
        "a1",
        now
    ));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Stopping);
    // Renewed, or taken over, meanwhile: left attached.
    {
        let mut live = fixture.instance.live.lock();
        live.phase = Phase::Attached;
        live.attachment = Some(Attachment {
            id: "a1".into(),
            lease_expires: now + Duration::from_secs(30),
            stream: None,
        });
    }
    assert!(!supervision::expire_attachment(
        &fixture.instance,
        "a1",
        now
    ));
    fixture.instance.live.lock().attachment = Some(expired("a2"));
    assert!(!supervision::expire_attachment(
        &fixture.instance,
        "a1",
        now
    ));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Attached);
    // Still the same, still expired: detached.
    assert!(supervision::expire_attachment(&fixture.instance, "a2", now));
    let live = fixture.instance.live.lock();
    assert_eq!(live.phase, Phase::Detached);
    assert!(live.attachment.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_whose_authority_ends_while_it_waits_stops_nothing() {
    let fixture = fixture().await;
    let authority = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let slot = fixture.manager.slot(&fixture.account);
    let lifecycle = slot.lifecycle.lock().await;
    let drain = {
        let manager = Arc::clone(&fixture.manager);
        let authority = Arc::clone(&authority);
        tokio::spawn(async move {
            manager
                .drain_instances(&|| {
                    ensure!(
                        authority.load(std::sync::atomic::Ordering::SeqCst),
                        "lease ended"
                    );
                    Ok(())
                })
                .await
                .map(|_| ())
        })
    };
    // The drain checked its authority and now waits for the lifecycle lock.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!drain.is_finished());
    authority.store(false, std::sync::atomic::Ordering::SeqCst);
    drop(lifecycle);
    let ended = tokio::time::timeout(Duration::from_secs(10), drain)
        .await
        .expect("drain did not return")
        .unwrap();
    assert!(ended.is_err());
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(fixture.units.terminations().is_empty());
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .is_some()
    );
}
