use super::{
    bounded_incident_summary, classify_crash_detail, classify_interruption_detail,
    classify_session_error, lifecycle_incident_recovery_outcome, record_lifecycle_incidents,
    session_error_severity,
};
use crate::core::{Envelope, Event, Status};
use crate::store::{RuntimeIncidentWrite, Store};

#[test]
fn crash_details_map_to_stable_incident_classes() {
    assert_eq!(
        classify_crash_detail(Some("process exited with signal: 9")),
        "resource_exhaustion"
    );
    assert_eq!(
        classify_crash_detail(Some("runtime frame too large")),
        "protocol_failure"
    );
    assert_eq!(
        classify_crash_detail(Some("socket connection timed out")),
        "transport_failure"
    );
    assert_eq!(
        classify_crash_detail(Some("exit status 217")),
        "process_exit"
    );
    assert_eq!(
        classify_crash_detail(Some(
            "API Error: 400 This model's maximum context length is 1048576 tokens"
        )),
        "provider_context_limit"
    );
    assert_eq!(
        classify_crash_detail(Some("API Error: Stream ended without receiving any events")),
        "provider_empty_stream"
    );
    assert_eq!(classify_crash_detail(None), "runtime_failure");
    assert_eq!(
        classify_crash_detail(Some(crate::core::MODEL_REFUSAL_DETAIL)),
        "provider_refusal"
    );
    assert_eq!(
        classify_crash_detail(Some(
            "fallback after generation launch failed: worker sess-1 exited before readiness with exit status: 1"
        )),
        "worker_startup_failure"
    );
    assert_eq!(
        classify_crash_detail(Some(
            "agent did not complete ACP session/resume within 240s"
        )),
        "session_restore_timeout"
    );
    assert_eq!(
        classify_crash_detail(Some(
            "worker sess-1 exited before readiness: agent did not complete ACP session/resume within 240s"
        )),
        "session_restore_timeout"
    );
    assert_eq!(
        classify_crash_detail(Some(crate::provider_behavior::NATIVE_RESTORE_TIMEOUT_HOLD)),
        "session_restore_timeout"
    );
}

#[test]
fn only_explicit_control_plane_edges_are_expected_interruptions() {
    assert_eq!(
        classify_interruption_detail(Some("controller restart during deploy")),
        "expected_interruption"
    );
    assert_eq!(
        classify_interruption_detail(Some("force cancel watchdog fired")),
        "runtime_interruption"
    );
}

#[test]
fn session_errors_retain_provider_and_transport_classifications() {
    assert_eq!(
        classify_session_error(
            "API Error: 400 This model's maximum context length is 1048576 tokens"
        ),
        "provider_context_limit"
    );
    assert_eq!(
        classify_session_error("socket connection timed out"),
        "transport_failure"
    );
    assert_eq!(
        classify_session_error(
            "Error running remote compact task: stream disconnected before completion: Transport error: network error: error decoding response body"
        ),
        "transport_failure"
    );
    assert_eq!(
        classify_session_error("runtime rejected command"),
        "session_command_error"
    );
    assert_eq!(
        classify_session_error("worker sess-1 exited before readiness with exit status: 1"),
        "worker_startup_failure"
    );
    assert_eq!(
        classify_session_error(
            "send failed: Kept in the queue. Reload to retry restore — large conversations may still fail."
        ),
        "session_restore_timeout"
    );
    assert_eq!(session_error_severity("worker_startup_failure"), "critical");
    assert_eq!(session_error_severity("process_exit"), "critical");
    assert_eq!(session_error_severity("session_command_error"), "error");
    assert_eq!(session_error_severity("session_restore_timeout"), "error");
}

#[test]
fn incident_summaries_are_bounded_on_utf8_boundaries() {
    let (summary, truncated) = bounded_incident_summary(&"你".repeat(2_000));
    assert!(truncated);
    assert!(summary.len() <= 4 * 1024);
    assert_eq!(summary, "你".repeat(summary.chars().count()));
}

#[test]
fn busy_and_running_lifecycle_states_recover_incidents() {
    assert_eq!(
        lifecycle_incident_recovery_outcome(Status::Busy),
        Some("session_busy")
    );
    assert_eq!(
        lifecycle_incident_recovery_outcome(Status::Running),
        Some("session_running")
    );
    assert_eq!(lifecycle_incident_recovery_outcome(Status::Crashed), None);
}

#[tokio::test]
async fn authoritative_busy_snapshot_closes_a_prior_active_incident() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-incident-recovery-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}", root.join("cowboy.sqlite3").display());
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store
        .upsert_runtime_incident(&RuntimeIncidentWrite {
            id: "incident-busy-recovery".to_owned(),
            occurred_at_ms: 1_900_000_000_000,
            source: "controller".to_owned(),
            classification: "runtime_interruption".to_owned(),
            severity: "warning".to_owned(),
            state: "active".to_owned(),
            summary: "turn appeared interrupted".to_owned(),
            fingerprint: "busy-recovery".to_owned(),
            session_id: Some("session-busy-recovery".to_owned()),
            client_id: None,
            machine_id: None,
            trace_id: None,
            build: Some("test".to_owned()),
            evidence_start_ms: 1_899_999_999_000,
            evidence_end_ms: 1_900_000_001_000,
            detail: serde_json::json!({}),
        })
        .await
        .unwrap();

    record_lifecycle_incidents(
        &store,
        &[Envelope {
            session_id: "session-busy-recovery".to_owned(),
            seq: 2,
            event: Event::Lifecycle {
                status: Status::Busy,
                detail: None,
            },
            cmid: None,
        }],
    )
    .await
    .unwrap();

    let incident = store
        .runtime_incidents(10)
        .await
        .unwrap()
        .into_iter()
        .find(|incident| incident.id == "incident-busy-recovery")
        .unwrap();
    assert_eq!(incident.state, "recovered");
    assert_eq!(incident.recovery_outcome.as_deref(), Some("session_busy"));
}
