//! The admitted memory budget: held with the instance's capacity slot for
//! its whole life, visible to every later admission, released exactly once
//! on confirmed exit (directly, through a termination retry, or with a
//! failed start's retained slot), and re-admitted for an adopted survivor.

use super::confirmation::{states, uplink};
use super::*;

const MIB: u64 = 1 << 20;
const BUDGET: u64 = 1300 * MIB;

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

#[tokio::test]
async fn a_failed_start_keeps_its_budget_until_its_unit_exit_is_confirmed() {
    let fixture = fixture().await;
    let ledger = fixture.manager.ledger;
    let slot = fixture
        .manager
        .capacity
        .force_reserve()
        .with_memory(ledger.force(BUDGET));
    fixture.manager.unconfirmed.lock().insert(
        2,
        Unconfirmed::unit("cowboy-browser-failed", Some(&fixture.account)),
    );
    assert_eq!(ledger.reserved_bytes(), BUDGET);
    fixture
        .manager
        .release_retained(
            2,
            Retained {
                unit: "cowboy-browser-failed".into(),
                cgroup_dir: None,
                invocation_id: None,
                account: Some(fixture.account.clone()),
                instance: None,
                record: None,
                capacity: Some(slot),
                generation: None,
                outcome: StopOutcome::unreported(Reason::Failed),
            },
        )
        .await;
    assert_eq!(ledger.reserved_bytes(), 0);
}

#[tokio::test]
async fn an_adopted_survivor_is_readmitted_with_its_own_memory_max() {
    let fixture = fixture().await;
    let settings = fixture.instance.settings;
    let record = fixture.instance.record(&fixture.manager.boot_id);
    let status = unit::UnitStatus {
        load_state: "loaded".into(),
        active_state: "active".into(),
        memory_max: Some(BUDGET),
        ..unit::UnitStatus::default()
    };
    let budget = fixture
        .manager
        .readmit_budget(&record, &status, &settings)
        .await;
    assert_eq!(budget.bytes(), BUDGET);
    // Charged to the unit's cgroup from the start.
    std::fs::write(
        fixture.cgroup.join("memory.current"),
        (300 * MIB).to_string(),
    )
    .unwrap();
    assert_eq!(fixture.manager.ledger.reserved_bytes(), BUDGET - 300 * MIB);
    drop(budget);
    // Without a reported limit the configured upper bound is assumed.
    let unlimited = unit::UnitStatus {
        memory_max: None,
        ..status.clone()
    };
    let budget = fixture
        .manager
        .readmit_budget(&record, &unlimited, &settings)
        .await;
    assert_eq!(budget.bytes(), settings.memory_max);
    drop(budget);
    assert_eq!(fixture.manager.ledger.reserved_bytes(), 0);

    // No headroom: the survivor is kept and its budget reserved anyway, so
    // every later admission still sees it.
    let temp = tempfile::tempdir().unwrap();
    let tight = manager_with_headroom(&temp.path().join("state"), || Some(0));
    let budget = tight.readmit_budget(&record, &status, &settings).await;
    assert_eq!(budget.bytes(), BUDGET);
    assert_eq!(tight.ledger.reserved_bytes(), BUDGET - 300 * MIB);
    drop(budget);
    assert_eq!(tight.ledger.reserved_bytes(), 0);
}
