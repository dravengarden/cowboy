//! Making room on a full Machine (an idle browser hibernates for another
//! account's open, a viewed one never does), the stop time an instance
//! reports, and the graceful stop ahead of the runtime cap.

use super::confirmation::{states, uplink};
use super::*;

fn other_account() -> ProviderAccountId {
    uuid::Uuid::new_v4().to_string().try_into().unwrap()
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
    let reserving = {
        let manager = Arc::clone(&fixture.manager);
        tokio::spawn(async move { manager.reserve_slot(&other_account(), 1).await })
    };
    answer_graceful_stop(&mut fixture.launcher).await;
    let slot = tokio::time::timeout(Duration::from_secs(10), reserving)
        .await
        .expect("the open never got its slot")
        .unwrap();
    assert!(slot.is_some());
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
    assert!(
        fixture
            .manager
            .reserve_slot(&other_account(), 1)
            .await
            .is_none()
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
    assert!(
        fixture
            .manager
            .reserve_slot(&fixture.account, 1)
            .await
            .is_none()
    );
    assert!(registered(&fixture));
}

#[tokio::test]
async fn a_viewer_that_attaches_while_room_is_made_keeps_its_browser() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    // Just opened (or its stream just dropped): detached, about to be viewed.
    fixture.instance.live.lock().detached_since = Some(Instant::now());
    let reserving = {
        let manager = Arc::clone(&fixture.manager);
        tokio::spawn(async move { manager.reserve_slot(&other_account(), 1).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!reserving.is_finished(), "the eviction did not wait");
    view(&fixture, "a1");
    let slot = tokio::time::timeout(Duration::from_secs(10), reserving)
        .await
        .expect("the open never answered")
        .unwrap();
    assert!(slot.is_none());
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
    // Browsers were turned off on this Device: nothing is admitted.
    assert!(
        fixture
            .manager
            .reserve_slot(&other_account(), 0)
            .await
            .is_none()
    );
    // A second slot is held by something that cannot be hibernated (a unit
    // whose exit is unconfirmed): the one idle browser would not make room.
    let retained = fixture.manager.capacity.force_reserve();
    assert!(
        fixture
            .manager
            .reserve_slot(&other_account(), 1)
            .await
            .is_none()
    );
    assert!(registered(&fixture));
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    assert!(fixture.units.terminations().is_empty());
    assert!(launcher_idle(&mut fixture.launcher).await);
    drop(retained);
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
