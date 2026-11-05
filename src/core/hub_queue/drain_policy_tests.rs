//! The drain policy as a table: each hold reason, its order, and which
//! holds a manual send overrides.

use super::DrainDecision;
use super::*;

fn session_with_queue(id: &str) -> Hub {
    let hub = Hub::new();
    hub.create_local_session(
        id.to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "t".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_status(id, Status::Running, None);
    // Without a dispatcher the drain never pops, so the head stays queued.
    hub.submit(id, "first".into(), vec![], Some("cmid-1".into()));
    hub
}

#[test]
fn an_idle_running_session_dispatches_its_head() {
    let hub = session_with_queue("s");
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Dispatch)
    );
}

#[test]
fn an_empty_queue_has_nothing_to_dispatch() {
    let hub = session_with_queue("s");
    hub.clear_queue("s");
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Empty)
    );
}

#[test]
fn a_busy_session_is_not_ready_even_for_a_manual_send() {
    let hub = session_with_queue("s");
    hub.set_status("s", Status::Busy, None);
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::NotReady)
    );
    assert_eq!(
        hub.drain_decision_for("s", true, true),
        Some(DrainDecision::NotReady)
    );
}

#[test]
fn only_a_revive_may_dispatch_into_an_exited_session() {
    let hub = session_with_queue("s");
    hub.set_status("s", Status::Exited, None);
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::NotReady)
    );
    assert_eq!(
        hub.drain_decision_for("s", true, false),
        Some(DrainDecision::Dispatch)
    );
}

#[test]
fn pause_holds_the_auto_drain_but_not_a_manual_send() {
    let hub = session_with_queue("s");
    hub.set_paused("s", true);
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Paused)
    );
    assert_eq!(
        hub.drain_decision_for("s", false, true),
        Some(DrainDecision::Dispatch)
    );
}

#[test]
fn editing_holds_the_auto_drain_but_not_a_manual_send() {
    let hub = session_with_queue("s");
    let head = hub.queue_and_drafts("s", |_| true).unwrap().0[0].id.clone();
    hub.set_queue_editing("s", Some(head));
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Editing)
    );
    assert_eq!(
        hub.drain_decision_for("s", false, true),
        Some(DrainDecision::Dispatch)
    );
}

#[test]
fn a_pending_context_edit_holds_even_a_manual_send() {
    let hub = session_with_queue("s");
    let _guard = hub.begin_context_edit("s", true).unwrap();
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::ContextHold)
    );
    assert_eq!(
        hub.drain_decision_for("s", false, true),
        Some(DrainDecision::ContextHold)
    );
}

#[test]
fn pause_is_reported_before_editing() {
    let hub = session_with_queue("s");
    let head = hub.queue_and_drafts("s", |_| true).unwrap().0[0].id.clone();
    hub.set_queue_editing("s", Some(head));
    hub.set_paused("s", true);
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Paused)
    );
}
