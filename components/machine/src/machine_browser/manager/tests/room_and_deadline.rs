//! Making room on a full Machine (an idle browser hibernates for another
//! account's open, a viewed one never does), the stop time an instance
//! reports, and the graceful stop ahead of the runtime cap.

use super::confirmation::{states, uplink};
use super::*;

fn other_account() -> ProviderAccountId {
    uuid::Uuid::new_v4().to_string().try_into().unwrap()
}

/// Reserve a slot for an open of `account` with the fixture's runtime.
async fn reserve(
    manager: &Arc<BrowserManager>,
    account: &ProviderAccountId,
    max_instances: u32,
) -> Result<CapacityGuard, Reason> {
    manager.reserve_slot(account, &plan(), max_instances).await
}

fn spawn_reserve(
    fixture: &Fixture,
    account: ProviderAccountId,
    max_instances: u32,
) -> tokio::task::JoinHandle<Result<CapacityGuard, Reason>> {
    let manager = Arc::clone(&fixture.manager);
    tokio::spawn(async move { reserve(&manager, &account, max_instances).await })
}

/// The fixture's instance has been detached for a minute.
fn idle_for_a_minute(fixture: &Fixture) {
    fixture.instance.live.lock().detached_since =
        Some(Instant::now().checked_sub(Duration::from_secs(60)).unwrap());
}

fn view(fixture: &Fixture, attachment: &str) {
    let mut live = fixture.instance.live.lock();
    live.attachment = Some(Attachment {
        id: attachment.into(),
        lease_expires: Instant::now() + Duration::from_secs(30),
        stream: None,
    });
    live.phase = Phase::Attached;
    live.detached_since = None;
}

/// Read the launcher's next command, which must be a stop asking for a
/// stop-time copy, and answer it as a flushed, copied stop.
async fn answer_graceful_stop(launcher: &mut tokio::net::UnixStream) {
    let mut reader = tokio::io::BufReader::new(&mut *launcher);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
        .await
        .expect("no stop reached the launcher")
        .unwrap();
    let command: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(command["command"], "stop");
    assert!(
        command["snapshot_generation"].is_u64(),
        "the stop asks for no stop-time copy: {command}"
    );
    let reply = format!(
        "{{\"id\":{},\"ok\":true,\"graceful\":true,\"snapshot\":{{\"bytes\":0,\"entries\":0}}}}\n",
        command["id"]
    );
    launcher.write_all(reply.as_bytes()).await.unwrap();
}

/// Whether anything reaches the launcher within a moment.
async fn launcher_idle(launcher: &mut tokio::net::UnixStream) -> bool {
    let mut probe = [0_u8; 1];
    !matches!(
        tokio::time::timeout(
            Duration::from_millis(200),
            tokio::io::AsyncReadExt::read(launcher, &mut probe),
        )
        .await,
        Ok(Ok(1))
    )
}

fn registered(fixture: &Fixture) -> bool {
    fixture
        .manager
        .slot(&fixture.account)
        .instance
        .lock()
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, &fixture.instance))
}

#[tokio::test]
async fn an_open_on_a_full_machine_hibernates_an_idle_browser_with_its_snapshot() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let (_uplink, mut received) = uplink(&fixture.manager);
    idle_for_a_minute(&fixture);
    // The only slot is the idle browser's.
    assert!(fixture.manager.capacity.try_reserve(1).is_none());
    let reserving = spawn_reserve(&fixture, other_account(), 1);
    answer_graceful_stop(&mut fixture.launcher).await;
    let slot = tokio::time::timeout(Duration::from_secs(10), reserving)
        .await
        .expect("the open never got its slot")
        .unwrap();
    assert!(slot.is_ok());
    // The slot moved to the open; the idle browser is gone and reported.
    assert_eq!(fixture.manager.capacity.used(), 1);
    assert!(!registered(&fixture));
    assert_eq!(
        fixture.units.terminations(),
        vec![(
            "cowboy-browser-inst-1".to_owned(),
            Some("invocation".to_owned())
        )]
    );
    let reported = states(&mut received);
    let terminal = reported
        .iter()
        .find(|state| state.state == State::Hibernated)
        .unwrap_or_else(|| panic!("no terminal report: {reported:?}"));
    assert_eq!(terminal.instance, fixture.instance.reference());
    assert_eq!(terminal.reason, Some(Reason::Reclaim));
    assert!(!terminal.unsaved_state_lost);
    assert!(terminal.snapshot_generation.is_some());
}

#[tokio::test]
async fn a_browser_being_viewed_is_never_taken_for_another_open() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    view(&fixture, "a1");
    assert_eq!(
        reserve(&fixture.manager, &other_account(), 1).await.err(),
        Some(Reason::Capacity)
    );
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Attached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
    // Nor is the opening account's own (an open of a running one never
    // reaches the reservation; a slot it holds is not its own to take).
    idle_for_a_minute(&fixture);
    fixture.instance.live.lock().phase = Phase::Detached;
    fixture.instance.live.lock().attachment = None;
    assert_eq!(
        reserve(&fixture.manager, &fixture.account, 1).await.err(),
        Some(Reason::Capacity)
    );
    assert!(registered(&fixture));
    // Nor one a snapshot copy keeps busy: its stop would wait for the copy.
    let (cancel, _) = tokio::sync::watch::channel(false);
    let (_done, done) = tokio::sync::watch::channel(false);
    *fixture.instance.copy.lock() = Some(CopyJob::for_test(cancel, done));
    assert_eq!(
        reserve(&fixture.manager, &other_account(), 1).await.err(),
        Some(Reason::Capacity)
    );
    assert!(registered(&fixture));
    assert!(fixture.units.terminations().is_empty());
}

#[tokio::test]
async fn a_viewer_that_attaches_while_room_is_made_keeps_its_browser() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    // Just opened (or its stream just dropped): detached, about to be viewed.
    fixture.instance.live.lock().detached_since = Some(Instant::now());
    let reserving = spawn_reserve(&fixture, other_account(), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!reserving.is_finished(), "the eviction did not wait");
    view(&fixture, "a1");
    let slot = tokio::time::timeout(Duration::from_secs(10), reserving)
        .await
        .expect("the open never answered")
        .unwrap();
    assert_eq!(slot.err(), Some(Reason::Capacity));
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Attached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn idle_browsers_stay_when_hibernating_them_could_not_free_a_slot() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    idle_for_a_minute(&fixture);
    // Browsers were turned off on this Device: nothing is admitted, and
    // that is not a slot some other browser could free.
    assert_eq!(
        reserve(&fixture.manager, &other_account(), 0).await.err(),
        Some(Reason::NotAvailable)
    );
    // A second slot is held by something that cannot be hibernated (a unit
    // whose exit is unconfirmed): the one idle browser would not make room.
    let retained = fixture.manager.capacity.force_reserve();
    assert_eq!(
        reserve(&fixture.manager, &other_account(), 1).await.err(),
        Some(Reason::Capacity)
    );
    drop(retained);
    // The opening account's own record would refuse its start: a pending
    // revocation, or a profile a newer browser wrote.
    let fenced = other_account();
    let fenced_dir = fixture.manager.account_dir(&fenced).unwrap();
    let mut record = state_file::read_control(&fenced_dir).unwrap();
    record.pending = Some(PendingOperation {
        kind: OperationKind::Revoke,
        operation_id: "op-1".into(),
        next_profile_revision: record.profile_revision + 1,
        started_at_ms: unix_ms(),
    });
    state_file::write_control(&fenced_dir, &record).unwrap();
    assert_eq!(
        reserve(&fixture.manager, &fenced, 1).await.err(),
        Some(Reason::Revoked)
    );
    let newer = other_account();
    let newer_dir = fixture.manager.account_dir(&newer).unwrap();
    let mut record = state_file::read_control(&newer_dir).unwrap();
    record.browser_version = Some("9999.0.0.0".into());
    state_file::write_control(&newer_dir, &record).unwrap();
    assert_eq!(
        reserve(&fixture.manager, &newer, 1).await.err(),
        Some(Reason::ProfileNewerThanRuntime)
    );
    // A Plugin uninstall holds the start gate: its drain stops what runs.
    let draining = fixture.manager.starts.hold();
    assert_eq!(
        reserve(&fixture.manager, &other_account(), 1).await.err(),
        Some(Reason::NotAvailable)
    );
    drop(draining);
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn a_slot_freed_while_room_is_made_needs_nobody_hibernated() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    // Two slots, both taken: the idle fixture and another unit just exiting.
    fixture.instance.live.lock().detached_since = Some(Instant::now());
    let exiting = fixture.manager.capacity.force_reserve();
    let reserving = spawn_reserve(&fixture, other_account(), 2);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!reserving.is_finished());
    drop(exiting);
    let slot = tokio::time::timeout(Duration::from_secs(10), reserving)
        .await
        .expect("the open never answered")
        .unwrap();
    assert!(slot.is_ok());
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn an_open_waits_only_so_long_for_room_and_the_stop_still_finishes() {
    let fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    idle_for_a_minute(&fixture);
    // The launcher reads the stop and never answers it.
    let began = Instant::now();
    let slot = tokio::time::timeout(
        Duration::from_secs(15),
        reserve(&fixture.manager, &other_account(), 1),
    )
    .await
    .expect("the open outlived its wait for room");
    assert_eq!(slot.err(), Some(Reason::Capacity));
    assert!(began.elapsed() < Duration::from_secs(10));
    // The idle browser's stop goes on by itself and ends with its unit.
    assert_eq!(fixture.instance.live.lock().phase, Phase::Stopping);
    drop(fixture.launcher);
    super::inventory::eventually("the stop that made room finishes", || {
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .is_none()
    })
    .await;
    assert_eq!(fixture.manager.capacity.used(), 0);
}

#[tokio::test]
async fn an_instance_reports_when_it_stops_if_nothing_changes() {
    let fixture = fixture().await;
    let cap =
        fixture.instance.deadline_ms - i64::try_from(DEADLINE_STOP_WINDOW.as_millis()).unwrap();
    let grace = i64::try_from(fixture.instance.settings.hibernate_after.as_millis()).unwrap();
    // Detached: the end of its grace, far ahead of the cap.
    fixture.instance.live.lock().detached_since = Some(Instant::now());
    let detached = fixture.instance.state(None);
    assert_eq!(detached.state, State::Detached);
    let reported = detached.deadline_ms.unwrap();
    let expected = unix_ms() + grace;
    assert!(
        (expected - 2_000..=expected).contains(&reported),
        "{reported} is not the end of the grace ({expected})"
    );
    assert!(reported < cap);
    // Half the grace later, the same instant is reported, not a new grace.
    fixture.instance.live.lock().detached_since =
        Some(Instant::now().checked_sub(Duration::from_secs(60)).unwrap());
    let later = fixture.instance.state(None).deadline_ms.unwrap();
    let expected = unix_ms() + grace - 60_000;
    assert!((expected - 2_000..=expected).contains(&later), "{later}");
    // Viewed: the graceful stop ahead of the runtime cap.
    view(&fixture, "a1");
    assert_eq!(fixture.instance.state(None).deadline_ms, Some(cap));
    // A grace that would outlast the cap is bounded by it.
    let short = fixture_with(None, Duration::from_secs(60)).await;
    short.instance.live.lock().detached_since = Some(Instant::now());
    assert_eq!(
        short.instance.state(None).deadline_ms,
        Some(short.instance.deadline_ms - i64::try_from(DEADLINE_STOP_WINDOW.as_millis()).unwrap())
    );
}

#[tokio::test]
async fn the_runtime_cap_is_preceded_by_a_graceful_stop_that_saves_the_profile() {
    // Ten seconds of runtime left: inside the graceful window, ahead of the cap.
    let mut fixture = fixture_with(None, Duration::from_secs(10)).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let (_uplink, mut received) = uplink(&fixture.manager);
    // The owner is watching: the cap is not idleness.
    view(&fixture, "a1");
    let supervised = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.supervise(&instance).await })
    };
    answer_graceful_stop(&mut fixture.launcher).await;
    let gone = tokio::time::timeout(Duration::from_secs(10), supervised)
        .await
        .expect("supervision did not finish the stop")
        .unwrap();
    assert!(gone);
    assert!(!registered(&fixture));
    let reported = states(&mut received);
    let terminal = reported
        .iter()
        .find(|state| state.state == State::Hibernated)
        .unwrap_or_else(|| panic!("no terminal report: {reported:?}"));
    assert_eq!(terminal.reason, Some(Reason::Deadline));
    assert!(!terminal.unsaved_state_lost);
    assert!(terminal.snapshot_generation.is_some());
}

#[tokio::test]
async fn the_runtime_cap_itself_still_forces_the_stop() {
    let mut fixture = fixture_with(None, Duration::ZERO).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let (_uplink, mut received) = uplink(&fixture.manager);
    view(&fixture, "a1");
    assert!(fixture.manager.supervise(&fixture.instance).await);
    assert!(!registered(&fixture));
    let reported = states(&mut received);
    let terminal = reported
        .iter()
        .find(|state| state.state == State::Hibernated)
        .unwrap_or_else(|| panic!("no terminal report: {reported:?}"));
    assert_eq!(terminal.reason, Some(Reason::Deadline));
    assert!(terminal.unsaved_state_lost);
    // No stop-time copy was asked of the launcher.
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn a_graceful_stop_never_holds_the_instance_past_the_cap() {
    // Inside the graceful window with two seconds left to the cap.
    let fixture = fixture_with(None, Duration::from_secs(2)).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let (_uplink, mut received) = uplink(&fixture.manager);
    view(&fixture, "a1");
    // The launcher reads the stop and never answers: left alone, the stop
    // would wait out the unit's whole stop timeout.
    let began = Instant::now();
    let gone = tokio::time::timeout(
        Duration::from_secs(15),
        fixture.manager.supervise(&fixture.instance),
    )
    .await
    .expect("the graceful stop ran past the cap");
    assert!(gone);
    assert!(
        began.elapsed() < Duration::from_secs(8),
        "stopped {:?} after it began",
        began.elapsed()
    );
    assert!(!registered(&fixture));
    let reported = states(&mut received);
    let terminal = reported
        .iter()
        .find(|state| state.state == State::Hibernated)
        .unwrap_or_else(|| panic!("no terminal report: {reported:?}"));
    assert_eq!(terminal.reason, Some(Reason::Deadline));
    // The copy the cap cut short was not published.
    assert!(terminal.unsaved_state_lost);
    assert_eq!(terminal.snapshot_generation, None);
}

#[tokio::test]
async fn no_stop_time_copy_is_asked_for_once_the_cap_has_passed() {
    let mut fixture = fixture_with(None, Duration::ZERO).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    // The owner's hibernate arrives at the cap, ahead of supervision.
    let event = fixture
        .manager
        .hibernate("h1", &fixture.instance.reference())
        .await;
    assert_eq!(event.state, State::Hibernated);
    assert_eq!(event.reason, Some(Reason::OwnerRequest));
    assert!(event.unsaved_state_lost);
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn the_graceful_stop_waits_supervised_while_a_copy_runs() {
    let mut fixture = fixture_with(None, Duration::from_secs(10)).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    view(&fixture, "a1");
    // A periodic copy is in flight when the graceful window opens.
    let mut job = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.snapshot_job(&instance).await })
    };
    let copy = read_command(&mut fixture.launcher, &mut job).await;
    assert_eq!(copy["command"], "snapshot");
    // The viewer's lease runs out meanwhile.
    fixture
        .instance
        .live
        .lock()
        .attachment
        .as_mut()
        .unwrap()
        .lease_expires = Instant::now();
    // Supervision neither blocks on the copy nor starts the stop: it keeps
    // its other duties, here the expired lease.
    let gone = tokio::time::timeout(
        Duration::from_secs(5),
        fixture.manager.supervise(&fixture.instance),
    )
    .await
    .expect("supervision blocked on the running copy");
    assert!(!gone);
    {
        let live = fixture.instance.live.lock();
        assert_eq!(live.phase, Phase::Detached);
        assert!(live.attachment.is_none());
    }
    assert!(launcher_idle(&mut fixture.launcher).await);
    // The copy finishes; the next tick stops gracefully.
    let reply = format!(
        "{{\"id\":{},\"ok\":true,\"bytes\":0,\"entries\":0}}\n",
        copy["id"]
    );
    fixture.launcher.write_all(reply.as_bytes()).await.unwrap();
    job.await.unwrap().unwrap();
    let supervised = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.supervise(&instance).await })
    };
    answer_graceful_stop(&mut fixture.launcher).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(10), supervised)
            .await
            .expect("supervision did not finish the stop")
            .unwrap()
    );
    assert!(!registered(&fixture));
}

#[tokio::test]
async fn an_open_that_stopped_waiting_hibernates_nobody_afterwards() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    idle_for_a_minute(&fixture);
    // The idle browser's lifecycle lock is held (a snapshot being published,
    // a reset) for longer than the open waits for room.
    let slot = fixture.manager.slot(&fixture.account);
    let held = slot.lifecycle.lock().await;
    let refused = tokio::time::timeout(
        Duration::from_secs(15),
        reserve(&fixture.manager, &other_account(), 1),
    )
    .await
    .expect("the open outlived its wait for room");
    assert_eq!(refused.err(), Some(Reason::Capacity));
    // Its stop gets the lock only now, and no longer has anyone to make
    // room for: the browser keeps running.
    drop(held);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
}

#[tokio::test]
async fn a_query_after_a_stop_says_how_the_browser_stopped() {
    // The cap kills: the profile since the last snapshot is lost.
    let fixture = fixture_with(None, Duration::ZERO).await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    assert!(fixture.manager.supervise(&fixture.instance).await);
    let answer = fixture.manager.query("q1", &fixture.account);
    assert_eq!(answer.state, State::Hibernated);
    assert_eq!(answer.instance.incarnation, 0);
    assert_eq!(answer.reason, Some(Reason::Deadline));
    assert!(answer.unsaved_state_lost);
    // Another account's query knows nothing of it.
    let other = fixture.manager.query("q2", &other_account());
    assert_eq!(other.reason, None);
    assert!(!other.unsaved_state_lost);
    // A pending fence is the account's state before any earlier stop.
    let account_dir = fixture.manager.account_dir(&fixture.account).unwrap();
    let mut record = state_file::read_control(&account_dir).unwrap();
    record.pending = Some(PendingOperation {
        kind: OperationKind::Reset,
        operation_id: "op-1".into(),
        next_profile_revision: record.profile_revision + 1,
        started_at_ms: unix_ms(),
    });
    state_file::write_control(&account_dir, &record).unwrap();
    let fenced = fixture.manager.query("q3", &fixture.account);
    assert_eq!(fenced.reason, Some(Reason::Busy));
    assert!(!fenced.unsaved_state_lost);
    // The reset completed: the stop was of a profile that is gone.
    record.pending = None;
    record.profile_revision += 1;
    state_file::write_control(&account_dir, &record).unwrap();
    let reset = fixture.manager.query("q4", &fixture.account);
    assert_eq!(reset.reason, None);
    assert!(!reset.unsaved_state_lost);
}

#[tokio::test]
async fn a_query_after_a_saved_stop_reports_nothing_lost() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let hibernating = {
        let manager = Arc::clone(&fixture.manager);
        let reference = fixture.instance.reference();
        tokio::spawn(async move { manager.hibernate("h1", &reference).await })
    };
    answer_graceful_stop(&mut fixture.launcher).await;
    let stopped = hibernating.await.unwrap();
    assert_eq!(stopped.reason, Some(Reason::OwnerRequest));
    let answer = fixture.manager.query("q1", &fixture.account);
    assert_eq!(answer.state, State::Hibernated);
    assert_eq!(answer.reason, Some(Reason::OwnerRequest));
    assert!(!answer.unsaved_state_lost);
    assert_eq!(answer.snapshot_generation, stopped.snapshot_generation);
}

#[tokio::test]
async fn the_grace_stop_is_put_off_while_a_copy_runs() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    // The grace ran out while a copy (started while attached) still runs.
    fixture.instance.live.lock().detached_since = Some(
        Instant::now()
            .checked_sub(fixture.instance.settings.hibernate_after + Duration::from_secs(1))
            .unwrap(),
    );
    let mut job = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.snapshot_job(&instance).await })
    };
    let copy = read_command(&mut fixture.launcher, &mut job).await;
    assert_eq!(copy["command"], "snapshot");
    // Supervision is not blocked by it and stops nothing yet.
    let gone = tokio::time::timeout(
        Duration::from_secs(5),
        fixture.manager.supervise(&fixture.instance),
    )
    .await
    .expect("supervision blocked on the running copy");
    assert!(!gone);
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(launcher_idle(&mut fixture.launcher).await);
    // The copy finishes; the next tick hibernates for the grace.
    let reply = format!(
        "{{\"id\":{},\"ok\":true,\"bytes\":0,\"entries\":0}}\n",
        copy["id"]
    );
    fixture.launcher.write_all(reply.as_bytes()).await.unwrap();
    job.await.unwrap().unwrap();
    let (_uplink, mut received) = uplink(&fixture.manager);
    let supervised = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.supervise(&instance).await })
    };
    answer_graceful_stop(&mut fixture.launcher).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(10), supervised)
            .await
            .expect("supervision did not finish the stop")
            .unwrap()
    );
    let reported = states(&mut received);
    assert!(
        reported
            .iter()
            .any(|state| state.state == State::Hibernated && state.reason == Some(Reason::Grace)),
        "{reported:?}"
    );
}
