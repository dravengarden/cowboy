use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingRejection, BindingSnapshot, binding_digest,
};
use crate::machine_protocol::telemetry_recovery::RecoveryObservation;
use crate::server::telemetry_binding::{
    recovery::{recover_fixture, request_fixture},
    resolution::resolve_fixture,
};
use crate::telemetry_binding::resolution::{ResolutionAction, ResolutionIntent};

async fn signed_interruption(lose_ack: bool) {
    let f = Fixture::setup(true, lose_ack, true).await;
    let intent = f.pending.as_ref().unwrap();
    let before = f
        .store
        .change_telemetry_binding(&Change::Begin(intent), &|| true)
        .await
        .unwrap()
        .operation;
    let before = super::super::super::advance(&f.store, &before, Progress::Dispatching)
        .await
        .unwrap();
    let before = super::super::super::advance(
        &f.store,
        &before,
        Progress::NeedsAttention {
            reason: crate::telemetry_binding::Attention::Uncertain,
            observation: None,
        },
    )
    .await
    .unwrap();
    let original = f
        .store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    let request = request_fixture(&before, &intent.actor);
    // Resolving this bookkeeping attempt needs neither an installation nor its
    // private routing policy; both remain unchanged, with no emission command.
    f.publish(vec![]);
    fs::rename(
        f.root.path().join("machine/telemetry.json"),
        f.root.path().join("machine/telemetry.paused.json"),
    )
    .unwrap();
    let local = FixtureEffects::new(intent);
    let auth = local.confirmation().auth;
    let observation = recover_fixture(&f.store, &request, auth, f.control.clone())
        .await
        .unwrap();
    let RecoveryObservation::Observed { snapshot } = observation else {
        panic!()
    };
    assert!(snapshot.receipt.unwrap().matches(&request));
    let binding = snapshot.binding;
    assert!(matches!(&binding, BindingObservation::Observed { snapshot }
        if snapshot.current == Some(BindingSnapshot::initial()) && !snapshot.unresolved
        && snapshot.receipt.as_ref().unwrap().outcome == BindingOutcome::Rejected { reason: BindingRejection::AuthorizationEnded }));
    assert_eq!(
        f.store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap(),
        original,
        "Machine recovery does not resolve the Service or change its selection"
    );
    assert!(
        !LegacyFence::recover(Some(&f.store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    assert_eq!(
        f.sends.load(Ordering::Relaxed),
        1,
        "never resend a lost recovery ACK"
    );
    assert_eq!(
        f.queries.load(Ordering::Relaxed),
        if lose_ack { 2 } else { 1 }
    );
    let service_resolution = ResolutionIntent::new(
        "service-after-machine-recovery".into(),
        intent.actor.clone(),
        &before,
        ResolutionAction::RecordRejected {
            observation_digest: binding_digest(&serde_json::to_vec(&binding).unwrap()),
        },
        chrono::Utc::now().timestamp_millis() + 60_000,
    )
    .unwrap();
    let completed = resolve_fixture(&f.store, &service_resolution, auth, Some(f.control.clone()))
        .await
        .unwrap();
    assert_eq!(
        completed.progress,
        Progress::Rejected {
            observation: binding
        }
    );
    let saved = f
        .store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.resolutions.len(), 1);
    assert_eq!(f.sends.load(Ordering::Relaxed), 1);
    assert!(
        !LegacyFence::recover(Some(&f.store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
}

#[tokio::test]
async fn signed_prepared_reopen_uses_separate_machine_and_service_confirmations() {
    signed_interruption(false).await;
}

#[tokio::test]
async fn signed_machine_recovery_lost_ack_queries_once_without_replay() {
    signed_interruption(true).await;
}
