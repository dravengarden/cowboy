use super::*;
use crate::machine_protocol::telemetry_binding::binding_digest;
use crate::server::telemetry_binding::resolution::resolve_fixture;
use crate::telemetry_binding::resolution::{ResolutionAction, ResolutionIntent};

#[tokio::test]
async fn signed_machine_resolution_queries_once_without_dispatch_policy_adoption_or_export() {
    let f = Fixture::new(true, false).await;
    let intent = f.select();
    let step = intent.machine_step().unwrap();
    let before = f
        .store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    let before = super::super::super::advance(&f.store, &before, Progress::Dispatching)
        .await
        .unwrap();
    let observation = f
        .control
        .commit_telemetry_binding(&f.connection, &step)
        .await
        .unwrap();
    assert!(
        matches!(observation, BindingObservation::Observed { ref snapshot }
        if matches!(snapshot.receipt.as_ref().unwrap().outcome, BindingOutcome::Applied { .. }))
    );
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
    let request = ResolutionIntent::new(
        "binding-resolution-signed-wire".into(),
        intent.actor.clone(),
        &before,
        ResolutionAction::AcceptApplied {
            observation_digest: binding_digest(&serde_json::to_vec(&observation).unwrap()),
        },
        chrono::Utc::now().timestamp_millis() + 60_000,
    )
    .unwrap();
    // The already applied fact remains queryable without selecting a Plugin or
    // reviving its private policy. A future export would need separate authority.
    f.publish(vec![]);
    fs::rename(
        f.root.path().join("machine/telemetry.json"),
        f.root.path().join("machine/telemetry.paused.json"),
    )
    .unwrap();
    let local = FixtureEffects::new(&intent);
    let auth = local.confirmation().auth;
    let completed = resolve_fixture(&f.store, &request, auth, Some(f.control.clone()))
        .await
        .unwrap();
    assert_eq!(completed.progress, Progress::Completed { observation });
    assert_eq!(
        f.sends.load(Ordering::Relaxed),
        1,
        "only the original mutation was dispatched"
    );
    assert_eq!(
        f.queries.load(Ordering::Relaxed),
        1,
        "resolution queries the exact request freshly"
    );
    f.control.disconnect("machine-test");
    assert_eq!(
        resolve_fixture(&f.store, &request, auth, None)
            .await
            .unwrap(),
        completed
    );
    let saved = f
        .store
        .telemetry_binding_ledger(&intent.service_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.resolutions.len(), 1);
    assert_eq!(f.queries.load(Ordering::Relaxed), 1);
    assert!(
        !LegacyFence::recover(Some(&f.store), &intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
}
