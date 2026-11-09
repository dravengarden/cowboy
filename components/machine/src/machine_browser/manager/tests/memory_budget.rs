//! The admitted memory budget: held with the instance's capacity slot for
//! its whole life, visible to every later admission, released exactly once
//! on confirmed exit (directly, through a termination retry, or with a
//! failed start's retained slot), and re-reserved at restart for every
//! survivor that may still be alive (adopted, refused, unobservable or
//! unrecorded) until its exit is confirmed.

use super::confirmation::{states, uplink};
use super::inventory::{eventually, record, unregistered};
use super::*;

const MIB: u64 = 1 << 20;
const BUDGET: u64 = 1300 * MIB;
const UNIT: &str = "cowboy-browser-inst-1";

/// Give the fixture instance's slot an admitted budget of `bytes`.
fn hold_budget(fixture: &Fixture, bytes: u64) {
    let mut slot = fixture.instance.capacity.lock();
    let held = slot
        .take()
        .unwrap()
        .with_memory(fixture.manager.ledger.force(bytes));
    *slot = Some(held);
}

fn manager_with_headroom(state: &Path, headroom: fn() -> Option<u64>) -> Arc<BrowserManager> {
    std::fs::create_dir_all(state).unwrap();
    let store = MachinePluginStore::new(
        state,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    BrowserManager::build(
        state,
        Arc::new(store),
        None,
        headroom,
        Box::leak(Box::new(Ledger::new())),
        Arc::new(fake_units::FakeUnits::default()) as Arc<dyn UnitControl>,
    )
    .unwrap()
}

#[tokio::test]
async fn a_running_budget_is_seen_by_later_starts_and_released_once_on_confirmed_exit() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    hold_budget(&fixture, BUDGET);
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    // A worker launch (or another open) sees the running instance's budget.
    let worker = 512 * MIB;
    let short = || Some(1300 * MIB + 512 * MIB + FLOOR_BYTES - 1);
    assert_eq!(
        ledger
            .reserve(worker, short, || false, Duration::ZERO, MEMORY_POLL)
            .await,
        Admission::Exhausted
    );
    // Its cgroup's own usage is credited once the unit runs: only what it
    // may still grow into stays reserved.
    fixture
        .instance
        .capacity
        .lock()
        .as_ref()
        .unwrap()
        .charge_memory_to(&fixture.cgroup);
    std::fs::write(
        fixture.cgroup.join("memory.current"),
        (640 * MIB).to_string(),
    )
    .unwrap();
    assert_eq!(ledger.reserved_bytes(), BUDGET - 640 * MIB);
    let admitted = ledger
        .reserve(worker, short, || false, Duration::ZERO, MEMORY_POLL)
        .await;
    assert!(matches!(admitted, Admission::Reserved { .. }));
    drop(admitted);
    // Still held while the stop is unconfirmed (the instance stays
    // registered); released when the termination retry confirms the exit.
    let (_uplink, mut received) = uplink(&fixture.manager);
    fixture.instance.live.lock().phase = Phase::Stopping;
    fixture.manager.unconfirmed.lock().insert(
        1,
        Unconfirmed::unit(&fixture.instance.unit, Some(&fixture.account)),
    );
    assert_eq!(ledger.reserved_bytes(), BUDGET - 640 * MIB);
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
                outcome: StopOutcome::unreported(Reason::Failed),
            },
        )
        .await;
    states(&mut received);
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), 0);
    // A second confirmation of the same instance releases nothing more.
    fixture.manager.finish_stopped(&fixture.instance);
    assert_eq!(ledger.reserved_bytes(), 0);
    let other = ledger.force(worker);
    assert_eq!(ledger.reserved_bytes(), worker);
    drop(other);
}

#[tokio::test]
async fn a_direct_confirmed_stop_releases_the_budget() {
    let fixture = fixture().await;
    hold_budget(&fixture, BUDGET);
    fixture.manager.finish_stopped(&fixture.instance);
    assert_eq!(fixture.manager.ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), 0);
}

/// Whether a 512 MiB worker launch is refused beside `held` reserved bytes
/// when the headroom would cover it without them.
async fn worker_refused(ledger: &'static Ledger, held: u64) -> bool {
    let headroom = move || Some(held + 512 * MIB + FLOOR_BYTES - 1);
    ledger
        .reserve(512 * MIB, headroom, || false, Duration::ZERO, MEMORY_POLL)
        .await
        == Admission::Exhausted
}

#[tokio::test]
async fn a_failed_start_keeps_its_budget_until_its_unit_exit_is_confirmed() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    let used = fixture.manager.capacity.used();
    let failed = |unit: &str, slot| Retained {
        unit: unit.into(),
        cgroup_dir: None,
        invocation_id: Some("f1".into()),
        account: Some(fixture.account.clone()),
        instance: None,
        record: None,
        capacity: Some(slot),
        generation: None,
        outcome: StopOutcome::unreported(Reason::ConfinementUnverified),
    };
    // Verification failed after the budget was admitted: the stop is not
    // confirmed, so the slot and the whole budget (no cgroup known) stay.
    fixture
        .units
        .set("cowboy-browser-failed", FakeUnit::live("f1").unstoppable());
    let slot = fixture
        .manager
        .capacity
        .force_reserve()
        .with_memory(ledger.force(BUDGET));
    assert!(
        !fixture
            .manager
            .terminate_or_retain(failed("cowboy-browser-failed", slot))
            .await
    );
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert!(worker_refused(ledger, BUDGET).await);
    fixture.units.make_stoppable("cowboy-browser-failed");
    eventually("the failed start's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
    // A confirmed stop releases it at once.
    fixture
        .units
        .set("cowboy-browser-failed", FakeUnit::live("f1"));
    let slot = fixture
        .manager
        .capacity
        .force_reserve()
        .with_memory(ledger.force(BUDGET));
    assert!(
        fixture
            .manager
            .terminate_or_retain(failed("cowboy-browser-failed", slot))
            .await
    );
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
}

#[tokio::test]
async fn a_survivor_slot_holds_its_own_memory_max_credited_only_with_a_known_cgroup() {
    let fixture = fixture().await;
    let settings = fixture.instance.settings;
    let ledger = fixture.manager.ledger;
    let used = fixture.manager.capacity.used();
    std::fs::write(
        fixture.cgroup.join("memory.current"),
        (300 * MIB).to_string(),
    )
    .unwrap();
    let slot = fixture
        .manager
        .survivor_slot(UNIT, Some(BUDGET), Some(&fixture.cgroup));
    assert_eq!(slot.memory_bytes(), Some(BUDGET));
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert_eq!(ledger.reserved_bytes(), BUDGET - 300 * MIB);
    drop(slot);
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
    // Its own limit unknown, the configured upper bound is assumed; its
    // cgroup unknown, nothing is credited.
    let slot = fixture.manager.survivor_slot(UNIT, None, None);
    assert_eq!(slot.memory_bytes(), Some(settings.memory_max));
    assert_eq!(ledger.reserved_bytes(), settings.memory_max);
    drop(slot);
    assert_eq!(ledger.reserved_bytes(), 0);

    // No headroom: the survivor's budget is reserved anyway, so every later
    // admission still sees it.
    let temp = tempfile::tempdir().unwrap();
    let tight = manager_with_headroom(&temp.path().join("state"), || Some(0));
    let slot = tight.survivor_slot(UNIT, Some(BUDGET), Some(&fixture.cgroup));
    assert_eq!(tight.ledger.reserved_bytes(), BUDGET - 300 * MIB);
    drop(slot);
    assert_eq!(tight.ledger.reserved_bytes(), 0);
}

#[tokio::test]
async fn an_unobservable_recorded_survivor_holds_its_budget_until_its_confirmed_exit() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    let memory_max = fixture.instance.settings.memory_max;
    unregistered(&fixture);
    let used = fixture.manager.capacity.used();
    let survivor = record(
        &fixture,
        &fixture.account,
        "inst-1",
        "invocation",
        Purpose::Tab,
    );
    state_file::write_instance(&fixture.manager.state_dir, &survivor).unwrap();
    fixture
        .units
        .set(UNIT, FakeUnit::live("invocation").unobservable());
    fixture.units.list(Some(&[]));
    fixture.manager.reconcile_units().await.unwrap();
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert_eq!(ledger.reserved_bytes(), memory_max);
    assert!(worker_refused(ledger, memory_max).await);
    // Observable and stoppable again: the retry confirms the exit and
    // releases slot and budget once.
    fixture.units.set(UNIT, FakeUnit::live("invocation"));
    eventually("the survivor's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
}

#[tokio::test]
async fn an_unrecorded_survivor_holds_its_memory_max_until_its_confirmed_exit() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    unregistered(&fixture);
    let used = fixture.manager.capacity.used();
    let stray = unit::unit_name("stray");
    fixture.units.set(
        &stray,
        FakeUnit::live("s1").memory_max(BUDGET).unstoppable(),
    );
    fixture.units.list(Some(&[stray.as_str()]));
    fixture.manager.reconcile_units().await.unwrap();
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    assert!(worker_refused(ledger, BUDGET).await);
    fixture.units.make_stoppable(&stray);
    eventually("the stray's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
}

#[tokio::test]
async fn a_refused_adoption_hands_its_budget_to_the_stop_until_the_exit_is_confirmed() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    unregistered(&fixture);
    let used = fixture.manager.capacity.used();
    let survivor = record(
        &fixture,
        &fixture.account,
        "inst-1",
        "invocation",
        Purpose::Tab,
    );
    // `adopt` refuses (no runtime is installed) and hands back the very
    // slot and budget it was given.
    let status = unit::UnitStatus {
        load_state: "loaded".into(),
        active_state: "active".into(),
        invocation_id: "invocation".into(),
        memory_max: Some(BUDGET),
        ..unit::UnitStatus::default()
    };
    let slot = fixture
        .manager
        .survivor_slot(UNIT, Some(BUDGET), Some(&fixture.cgroup));
    let Err((_, slot)) = fixture.manager.adopt(&survivor, &status, slot).await else {
        panic!("adopted without a runtime");
    };
    assert_eq!(slot.memory_bytes(), Some(BUDGET));
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    drop(slot);
    assert_eq!(ledger.reserved_bytes(), 0);

    // Through restart reconciliation: the refused survivor's stop is
    // unconfirmed, so its slot and budget (credited with its recorded
    // cgroup's usage) stay with the retained termination.
    state_file::write_instance(&fixture.manager.state_dir, &survivor).unwrap();
    fixture.units.set(
        UNIT,
        FakeUnit::live("invocation")
            .memory_max(BUDGET)
            .unstoppable(),
    );
    fixture.units.list(Some(&[UNIT]));
    fixture.manager.reconcile_units().await.unwrap();
    assert!(
        fixture
            .units
            .terminations()
            .contains(&(UNIT.to_owned(), Some("invocation".to_owned())))
    );
    assert_eq!(fixture.manager.capacity.used(), used + 1);
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    std::fs::write(
        fixture.cgroup.join("memory.current"),
        (300 * MIB).to_string(),
    )
    .unwrap();
    assert_eq!(ledger.reserved_bytes(), BUDGET - 300 * MIB);
    assert!(worker_refused(ledger, BUDGET - 300 * MIB).await);
    fixture.units.make_stoppable(UNIT);
    eventually("the refused survivor's exit is confirmed", || {
        fixture.manager.unconfirmed.lock().is_empty()
    })
    .await;
    assert_eq!(ledger.reserved_bytes(), 0);
    assert_eq!(fixture.manager.capacity.used(), used);
    assert!(
        state_file::read_instances(&fixture.manager.state_dir)
            .unwrap()
            .is_empty()
    );
}
