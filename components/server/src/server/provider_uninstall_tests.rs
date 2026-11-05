use super::provider_session_has_active_turn;
use crate::agent_model::Status;

#[test]
fn only_an_in_flight_turn_requires_active_uninstall_confirmation() {
    assert!(provider_session_has_active_turn(Status::Busy));
    for idle_or_terminal in [
        Status::Starting,
        Status::Running,
        Status::Exited,
        Status::Crashed,
        Status::Interrupted,
    ] {
        assert!(!provider_session_has_active_turn(idle_or_terminal));
    }
}
