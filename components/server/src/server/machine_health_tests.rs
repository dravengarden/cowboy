use super::{
    MACHINE_COMPONENT_RECONCILIATION_GRACE, MACHINE_RUNTIME_STARTUP_GRACE, project_machine_health,
};
use crate::machine_protocol::{MachineHealthReason, MachineHealthState};

#[test]
fn controller_projects_runtime_health_instead_of_process_presence() {
    let ready = project_machine_health(
        "online",
        true,
        Some(std::time::Duration::from_secs(90)),
        false,
        200,
        Some(150),
    );
    assert_eq!(ready.state, MachineHealthState::Ready);
    assert_eq!(ready.reason, MachineHealthReason::RuntimeConnected);
    assert_eq!(ready.observed_at_ms, 200);
    assert_eq!(ready.last_seen_at_ms, Some(150));

    let starting = project_machine_health(
        "online",
        false,
        Some(MACHINE_RUNTIME_STARTUP_GRACE - std::time::Duration::from_millis(1)),
        false,
        200,
        None,
    );
    assert_eq!(starting.state, MachineHealthState::Starting);
    assert_eq!(starting.reason, MachineHealthReason::RuntimeStarting);

    let unresponsive = project_machine_health(
        "online",
        false,
        Some(MACHINE_RUNTIME_STARTUP_GRACE),
        false,
        200,
        Some(150),
    );
    assert_eq!(unresponsive.state, MachineHealthState::Degraded);
    assert_eq!(unresponsive.reason, MachineHealthReason::RuntimeUnavailable);

    let stale_database_presence =
        project_machine_health("online", false, None, false, 200, Some(150));
    assert_eq!(stale_database_presence.state, MachineHealthState::Degraded);
    assert_eq!(
        stale_database_presence.reason,
        MachineHealthReason::TransportUnavailable
    );

    let updating = project_machine_health(
        "online",
        false,
        Some(MACHINE_COMPONENT_RECONCILIATION_GRACE - std::time::Duration::from_millis(1)),
        true,
        200,
        Some(150),
    );
    assert_eq!(updating.state, MachineHealthState::Updating);
    assert_eq!(updating.reason, MachineHealthReason::MachineUpdating);

    let stuck_update = project_machine_health(
        "online",
        false,
        Some(MACHINE_COMPONENT_RECONCILIATION_GRACE),
        true,
        200,
        Some(150),
    );
    assert_eq!(stuck_update.state, MachineHealthState::Degraded);
    assert_eq!(stuck_update.reason, MachineHealthReason::RuntimeUnavailable);
}

#[test]
fn controller_preserves_explicit_machine_lifecycle_states() {
    for (status, expected_state, expected_reason) in [
        (
            "reconnecting",
            MachineHealthState::Reconnecting,
            MachineHealthReason::TransportReconnecting,
        ),
        (
            "updating",
            MachineHealthState::Updating,
            MachineHealthReason::MachineUpdating,
        ),
        (
            "degraded",
            MachineHealthState::Degraded,
            MachineHealthReason::MachineReportedDegraded,
        ),
        (
            "offline",
            MachineHealthState::Offline,
            MachineHealthReason::MachineOffline,
        ),
    ] {
        let health = project_machine_health(status, false, None, false, 200, None);
        assert_eq!(health.state, expected_state);
        assert_eq!(health.reason, expected_reason);
    }
}

#[test]
fn machine_health_serializes_as_a_stable_client_contract() {
    let health = project_machine_health("online", true, None, false, 200, Some(150));
    assert_eq!(
        serde_json::to_value(health).expect("serialize Machine health"),
        serde_json::json!({
            "state": "ready",
            "reason": "runtime_connected",
            "observed_at_ms": 200,
            "last_seen_at_ms": 150,
        })
    );
}
