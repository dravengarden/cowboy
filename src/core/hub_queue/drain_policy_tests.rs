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
    hub.submit(
        id,
        "first".into(),
        vec![],
        Some("cmid-1".into()),
        &crate::prompt_origin::RequestCaller::Person,
    );
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

#[test]
fn editing_state_is_included_in_queue_resync_and_clears_on_release() {
    let hub = session_with_queue("s");
    let head = hub.queue_and_drafts("s", |_| true).unwrap().0[0].id.clone();
    hub.set_queue_editing("s", Some(head.clone()));
    let held_patch = hub.queue_resync("s").expect("resync while held");
    let Outbound::SyncPatch { value, .. } = held_patch else {
        panic!("expected SyncPatch");
    };
    assert_eq!(value["editing"], serde_json::json!(head));

    hub.set_queue_editing("s", None);
    let cleared_patch = hub.queue_resync("s").expect("resync after release");
    let Outbound::SyncPatch { value, .. } = cleared_patch else {
        panic!("expected SyncPatch");
    };
    assert!(value["editing"].is_null());
}

#[test]
fn scheduled_draft_edit_defers_fire_without_blocking_queue_and_fires_updated_text_on_release() {
    let hub = session_with_queue("s");
    let past_ms = chrono::Utc::now().timestamp_millis() - 5_000;
    hub.schedule_draft(
        "s",
        None,
        Some("sched-1".into()),
        "stale scheduled text".into(),
        vec![],
        past_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();

    // Holding a scheduled draft must NOT freeze the unrelated queue.
    hub.set_queue_editing("s", Some(draft_id.clone()));
    assert_eq!(
        hub.drain_decision_for("s", false, false),
        Some(DrainDecision::Dispatch)
    );

    // Timer firing while editing defers instead of firing stale text.
    hub.fire_scheduled_draft("s", &draft_id);
    let (q_during, d_during) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert_eq!(q_during.len(), 1);
    assert_eq!(d_during.len(), 1);

    // User saves updated text, then releases edit → updated text enqueues.
    assert!(hub.edit_draft(
        "s",
        &draft_id,
        "updated scheduled text".into(),
        vec![],
        &crate::prompt_origin::RequestCaller::Person,
    ));
    hub.set_queue_editing("s", None);
    let (q_after, d_after) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert!(d_after.is_empty());
    assert_eq!(q_after.len(), 2);
    assert_eq!(q_after[1].text, "updated scheduled text");
}

#[test]
fn scheduled_draft_unscheduled_during_edit_does_not_fire_on_release() {
    let hub = session_with_queue("s");
    let past_ms = chrono::Utc::now().timestamp_millis() - 5_000;
    hub.schedule_draft(
        "s",
        None,
        Some("sched-2".into()),
        "cancel me".into(),
        vec![],
        past_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();

    hub.set_queue_editing("s", Some(draft_id.clone()));
    hub.fire_scheduled_draft("s", &draft_id);
    hub.unschedule_draft("s", &draft_id);
    hub.set_queue_editing("s", None);

    let (q_after, d_after) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert_eq!(q_after.len(), 1);
    assert_eq!(d_after.len(), 1);
    assert!(d_after[0].schedule.is_none());
}

#[test]
fn stale_timer_callback_never_fires_unscheduled_or_rescheduled_draft() {
    let hub = session_with_queue("s");
    hub.clear_queue("s");
    hub.set_paused("s", true);
    let past_ms = chrono::Utc::now().timestamp_millis() - 5_000;
    let future_ms = chrono::Utc::now().timestamp_millis() + 600_000;
    hub.schedule_draft(
        "s",
        None,
        Some("c-resched".into()),
        "move later".into(),
        vec![],
        past_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();

    // Reschedule into the future, then simulate an old timer firing with past_ms.
    hub.schedule_draft(
        "s",
        Some(draft_id.clone()),
        None,
        String::new(),
        vec![],
        future_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    hub.fire_scheduled_draft_at("s", &draft_id, Some(past_ms));
    hub.fire_scheduled_draft("s", &draft_id);

    let (q, d) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert!(q.is_empty());
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].schedule.as_ref().unwrap().fire_at_ms, future_ms);
}

#[test]
fn clear_queue_preserves_scheduled_draft_hold_and_displaced_hold_fires_deferred_draft() {
    let hub = session_with_queue("s");
    hub.set_paused("s", true);
    let past_ms = chrono::Utc::now().timestamp_millis() - 5_000;
    hub.schedule_draft(
        "s",
        None,
        Some("d1".into()),
        "due draft".into(),
        vec![],
        past_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();
    let epoch = hub.set_queue_editing("s", Some(draft_id.clone()));
    hub.fire_scheduled_draft("s", &draft_id);

    // Clearing the queue must NOT erase the scheduled-draft edit hold.
    hub.clear_queue("s");
    assert!(hub.release_queue_editing_if_epoch("s", &draft_id, epoch));
    let (q, d) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].text, "due draft");
    assert!(d.is_empty());

    // Acquiring a replacement hold on another row must service a displaced
    // scheduled draft's deferred fire immediately.
    hub.schedule_draft(
        "s",
        None,
        Some("d2".into()),
        "second due".into(),
        vec![],
        past_ms,
        Delivery::Front,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let (q_before, d_before) = hub.queue_and_drafts("s", |_| true).unwrap();
    let q_id = q_before[0].id.clone();
    let d2_id = d_before[0].id.clone();
    hub.set_queue_editing("s", Some(d2_id.clone()));
    hub.fire_scheduled_draft("s", &d2_id);
    hub.set_queue_editing("s", Some(q_id));

    let (q_final, d_final) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert!(d_final.is_empty());
    assert_eq!(q_final.len(), 2);
    assert_eq!(q_final[0].text, "second due");
}

#[test]
fn activate_draft_with_front_atomically_removes_scheduled_draft_and_prepends_queue() {
    let hub = session_with_queue("s");
    hub.clear_queue("s");
    hub.set_paused("s", true);
    hub.submit(
        "s",
        "existing".into(),
        vec![],
        Some("q1".into()),
        &crate::prompt_origin::RequestCaller::Person,
    );
    let future_ms = chrono::Utc::now().timestamp_millis() + 600_000;
    hub.schedule_draft(
        "s",
        None,
        Some("d-front".into()),
        "jump ahead".into(),
        vec![],
        future_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();
    hub.set_queue_editing("s", Some(draft_id.clone()));
    hub.activate_draft_with_front(
        "s",
        &draft_id,
        Some("op-front".into()),
        true,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let (q, d) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert!(d.is_empty());
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].text, "jump ahead");
    assert_eq!(q[1].text, "existing");
}
