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

#[test]
fn steer_reservation_blocks_timer_and_concurrent_edits_and_fails_closed_on_mismatch() {
    let hub = session_with_queue("s");
    hub.clear_queue("s");
    hub.set_paused("s", true);
    let past_ms = chrono::Utc::now().timestamp_millis() - 5_000;
    hub.schedule_draft(
        "s",
        None,
        Some("d-steer".into()),
        "original text".into(),
        vec![],
        past_ms,
        Delivery::Back,
        &crate::prompt_origin::RequestCaller::Person,
    );
    let draft_id = hub.queue_and_drafts("s", |_| true).unwrap().1[0].id.clone();
    let source = SteerSource {
        kind: SteerSourceKind::Draft,
        id: draft_id.clone(),
    };

    // Stale text must fail closed without touching the draft.
    assert!(
        hub.reserve_steer_source("s", "op-stale", &source, "different text", &[])
            .is_err()
    );
    assert_eq!(hub.queue_and_drafts("s", |_| true).unwrap().1.len(), 1);

    // Matching reservation defers elapsed timers and blocks concurrent edits.
    assert!(
        hub.reserve_steer_source("s", "op-1", &source, "original text", &[])
            .is_ok()
    );
    hub.fire_scheduled_draft("s", &draft_id);
    assert!(hub.queue_and_drafts("s", |_| true).unwrap().0.is_empty());
    assert!(!hub.edit_draft(
        "s",
        &draft_id,
        "clobber".into(),
        vec![],
        &crate::prompt_origin::RequestCaller::Person,
    ));

    // Releasing after not_delivered fires the overdue deferred draft.
    hub.release_steer_reservation("s", "op-1");
    let (q_after_release, d_after_release) = hub.queue_and_drafts("s", |_| true).unwrap();
    assert_eq!(q_after_release.len(), 1);
    assert_eq!(q_after_release[0].text, "original text");
    assert!(d_after_release.is_empty());

    // Committing a queued steer reservation atomically consumes the queued row.
    let q_id = q_after_release[0].id.clone();
    let q_source = SteerSource {
        kind: SteerSourceKind::Queued,
        id: q_id,
    };
    assert!(
        hub.reserve_steer_source("s", "op-2", &q_source, "original text", &[])
            .is_ok()
    );
    hub.commit_steer_reservation("s", "op-2");
    assert!(hub.queue_and_drafts("s", |_| true).unwrap().0.is_empty());
}

#[test]
fn steer_reservations_persist_across_restore_and_block_draft_and_queue_transitions() {
    let hub = session_with_queue("s");
    hub.create_local_session(
        "other".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "o".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_paused("s", true);
    let caller = crate::prompt_origin::RequestCaller::Person;
    hub.add_draft(
        "s",
        "reserved draft".into(),
        vec![],
        Some("d-res".into()),
        &caller,
    );
    hub.add_draft(
        "s",
        "free draft".into(),
        vec![],
        Some("d-free".into()),
        &caller,
    );
    let (q0, d0) = hub.queue_and_drafts("s", |_| true).unwrap();
    let q_id = q0[0].id.clone();
    let d_res_id = d0[0].id.clone();

    assert!(
        hub.reserve_steer_source(
            "s",
            "op-d",
            &SteerSource {
                kind: SteerSourceKind::Draft,
                id: d_res_id.clone(),
            },
            "reserved draft",
            &[],
        )
        .is_ok()
    );
    assert!(
        hub.reserve_steer_source(
            "s",
            "op-q",
            &SteerSource {
                kind: SteerSourceKind::Queued,
                id: q_id.clone(),
            },
            "first",
            &[],
        )
        .is_ok()
    );
    assert!(hub.is_steer_reserved("s", &d_res_id));
    assert!(hub.is_steer_reserved("s", &q_id));

    // Simulate a Controller restart by restoring settings + session rows into a fresh Hub.
    let setting_key = format!("{}s", settings_keys::SESSION_STEER_RESERVATIONS_PREFIX);
    let persisted = hub
        .setting(&setting_key)
        .expect("steer reservations persisted");
    let (q_snap, d_snap) = hub.queue_and_drafts("s", |_| true).unwrap();
    let restored = Hub::new();
    restored.load_settings(vec![(setting_key.clone(), persisted)]);
    let mut restored_session = super::super::runtime_reconciliation_tests::restored_busy("s");
    restored_session.meta.status = Status::Running;
    restored_session.queue = q_snap;
    restored_session.drafts = d_snap;
    restored.restore_impl(vec![restored_session], &[], false);
    restored.set_paused("s", true);
    restored.create_local_session(
        "other".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "o".to_owned(),
        SessionOrigin::Web,
        false,
    );
    assert!(restored.is_steer_reserved("s", &d_res_id));
    assert!(restored.is_steer_reserved("s", &q_id));

    // Reserved rows refuse move, activate, queued_to_draft, promote, and bulk clear;
    // unreserved drafts still activate normally.
    restored.move_draft("s", &d_res_id, "other", &caller);
    restored.activate_draft_with_front("s", &d_res_id, None, true, &caller);
    restored.activate_all_drafts("s", &caller);
    restored.clear_drafts("s");
    restored.queued_to_draft("s", &q_id, &caller);
    assert!(!restored.promote_queued("s", &q_id, &caller));
    restored.clear_queue("s");

    let (q_after, d_after) = restored.queue_and_drafts("s", |_| true).unwrap();
    assert_eq!(d_after.len(), 1);
    assert_eq!(d_after[0].id, d_res_id);
    // Reserved queued row remained untouched; unreserved draft was activated then cleared by clear_queue.
    assert_eq!(q_after.len(), 1);
    assert_eq!(q_after[0].id, q_id);
    assert!(
        restored
            .queue_and_drafts("other", |_| true)
            .unwrap()
            .1
            .is_empty()
    );

    // Committing both clears the persisted setting.
    restored.commit_steer_reservation("s", "op-d");
    restored.commit_steer_reservation("s", "op-q");
    assert!(!restored.is_steer_reserved("s", &d_res_id));
    assert!(!restored.is_steer_reserved("s", &q_id));
    assert!(
        restored
            .setting(&setting_key)
            .is_none_or(|value| value.is_null())
    );
}

#[test]
fn editing_token_is_emitted_in_queue_resync_and_cleared_on_release() {
    let hub = session_with_queue("s");
    let head = hub.queue_and_drafts("s", |_| true).unwrap().0[0].id.clone();
    let epoch = hub.set_queue_editing_with_token("s", Some(head.clone()), Some("tok-1".into()));
    let Outbound::SyncPatch { value, .. } = hub.queue_resync("s").expect("resync while held")
    else {
        panic!("expected SyncPatch");
    };
    assert_eq!(value["editing"], serde_json::json!(head));
    assert_eq!(value["editing_token"], serde_json::json!("tok-1"));

    assert!(hub.release_queue_editing_if_epoch("s", &head, epoch));
    let Outbound::SyncPatch { value, .. } = hub.queue_resync("s").expect("resync after release")
    else {
        panic!("expected SyncPatch");
    };
    assert!(value["editing"].is_null());
    assert!(value["editing_token"].is_null());
}

#[tokio::test]
async fn hibernation_exit_and_unpausing_hibernated_session_revive_queued_work() {
    let hub = Hub::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    hub.set_dispatch_tx(tx);
    hub.create_local_session(
        "hib-unpaused".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "t".to_owned(),
        SessionOrigin::Web,
        false,
    );
    // Busy turn holds queued prompt; hibernation completes -> revives queued prompt.
    hub.set_status("hib-unpaused", Status::Busy, None);
    hub.submit(
        "hib-unpaused",
        "queued during hibernate".into(),
        vec![],
        Some("cmid-hib-1".into()),
        &crate::prompt_origin::RequestCaller::Person,
    );
    assert!(rx.try_recv().is_err());
    hub.set_status(
        "hib-unpaused",
        Status::Exited,
        Some("hibernated".to_owned()),
    );
    let dispatched = rx
        .try_recv()
        .expect("hibernation exit revives unpaused queue");
    assert_eq!(dispatched.session_id, "hib-unpaused");
    assert_eq!(dispatched.text, "queued during hibernate");

    // Paused queue held across hibernation -> unpausing revives the hibernated session.
    hub.create_local_session(
        "hib-paused".to_owned(),
        "claude-code".to_owned(),
        "/tmp".to_owned(),
        "t".to_owned(),
        SessionOrigin::Web,
        false,
    );
    hub.set_status("hib-paused", Status::Running, None);
    hub.set_paused("hib-paused", true);
    hub.submit(
        "hib-paused",
        "paused during hibernate".into(),
        vec![],
        Some("cmid-hib-2".into()),
        &crate::prompt_origin::RequestCaller::Person,
    );
    hub.set_status("hib-paused", Status::Exited, Some("hibernated".to_owned()));
    assert!(
        rx.try_recv().is_err(),
        "paused queue stays held across hibernation"
    );
    hub.set_paused("hib-paused", false);
    let revived = rx
        .try_recv()
        .expect("unpausing hibernated session revives queue");
    assert_eq!(revived.session_id, "hib-paused");
    assert_eq!(revived.text, "paused during hibernate");
}
