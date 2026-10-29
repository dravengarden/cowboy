//! A Machine restarted mid-staging must not leave the Provider fenced until an
//! Operator acts, while every other uncertain outcome stays fenced.

use super::super::interrupted::{InterruptedStagingMachine, RECOVERY_ACCOUNT, recover_slot};
use super::*;
use crate::machine_protocol::plugin_install::{
    InstallPhase as MachinePhase, InstallTarget, InstallUncertainty,
};
use crate::plugin_operation::installation::machine_fixture as fixture;

struct FakeMachine {
    outcome: Option<InstallOutcome>,
    target: Option<InstallTarget>,
    current: bool,
    queries: AtomicUsize,
}

impl FakeMachine {
    fn new(outcome: InstallOutcome) -> Self {
        Self {
            outcome: Some(outcome),
            target: Some(InstallTarget::Vacant {}),
            current: true,
            queries: AtomicUsize::new(0),
        }
    }
}

impl InterruptedStagingMachine for FakeMachine {
    fn current(&self) -> bool {
        self.current
    }
    async fn receipt(
        &self,
        step: &InstallStep,
    ) -> Option<crate::machine_protocol::plugin_install::InstallReceipt> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        self.outcome
            .clone()
            .map(|outcome| machine_receipt(step, outcome))
    }
    async fn target(&self, _: &InstallTargetQuery) -> Option<InstallTarget> {
        self.target.clone()
    }
}

fn interrupted() -> InstallOutcome {
    InstallOutcome::Unknown {
        phase: MachinePhase::Staging,
        reason: InstallUncertainty::Interrupted,
    }
}

/// The incident: the observer saved `Pending/Staging` before the Machine
/// process died, leaving the slot `NeedsReconcile`.
async fn interrupted_install(id: &str) -> (tempfile::TempDir, Store, InstallIntent) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture(id);
    store.begin_plugin_install(&intent).await.unwrap();
    store
        .advance_plugin_install(
            &intent,
            InstallPhase::Prepared,
            InstallPhase::Installing,
            None,
        )
        .await
        .unwrap();
    let step = intent.machine_step().unwrap();
    store
        .record_plugin_install_receipt(
            &intent,
            &machine_receipt(
                &step,
                InstallOutcome::Pending {
                    phase: MachinePhase::Staging,
                },
            ),
        )
        .await
        .unwrap();
    (root, store, intent)
}

async fn recover(
    store: &Store,
    fences: &PluginLifecycleFences,
    intent: &InstallIntent,
    machine: &FakeMachine,
) -> Option<crate::plugin_operation::installation::InstallStagingResolutionReceipt> {
    recover_slot(
        store,
        fences,
        &intent.service_id,
        &intent.machine_id,
        &intent.plugin_id,
        machine,
    )
    .await
    .unwrap()
}

fn slot_of(intent: &InstallIntent) -> (String, String) {
    (intent.machine_id.clone(), intent.plugin_id.clone())
}

fn needs_reconcile(intent: &InstallIntent) -> PluginLifecycleFences {
    Arc::new(parking_lot::RwLock::new(
        [(slot_of(intent), PluginFenceState::NeedsReconcile)]
            .into_iter()
            .collect(),
    ))
}

#[tokio::test]
async fn machine_proven_interrupted_staging_releases_the_slot_without_an_operator() {
    let (_root, store, intent) = interrupted_install("interrupted-staging").await;
    let fences = needs_reconcile(&intent);
    let machine = FakeMachine::new(interrupted());

    let resolution = recover(&store, &fences, &intent, &machine)
        .await
        .expect("interrupted staging is resolved");

    assert_eq!(
        resolution.intent.actor,
        crate::plugin_operation::Actor::Admin {
            account: RECOVERY_ACCOUNT.into()
        }
    );
    assert_eq!(resolution.intent.observed_target, InstallTarget::Vacant {});
    assert_eq!(fences.read().get(&slot_of(&intent)), None);
    let saved = store
        .plugin_install_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.phase, InstallPhase::NeedsAttention);
    assert_eq!(
        saved.machine_receipt.map(|receipt| receipt.outcome),
        Some(interrupted()),
        "the original operation keeps the exact Machine evidence"
    );
    assert_eq!(
        store
            .plugin_install_staging_resolution(&intent.operation_id)
            .await
            .unwrap(),
        Some(resolution)
    );
    assert!(
        store
            .recover_plugin_installs(&intent.service_id)
            .await
            .unwrap()
            .is_empty(),
        "a restarted Controller does not re-fence the resolved slot"
    );
    store
        .begin_plugin_install(&fixture("fresh-after-interruption"))
        .await
        .expect("a fresh operation identity may install again");

    // Idempotent: nothing is left to resolve on the next reconnect.
    fences
        .write()
        .insert(slot_of(&intent), PluginFenceState::NeedsReconcile);
    assert!(recover(&store, &fences, &intent, &machine).await.is_none());
}

#[tokio::test]
async fn every_other_uncertain_outcome_stays_fenced() {
    let cases = [
        InstallOutcome::Pending {
            phase: MachinePhase::Staging,
        },
        InstallOutcome::Unknown {
            phase: MachinePhase::Staging,
            reason: InstallUncertainty::EffectFailure,
        },
        InstallOutcome::Unknown {
            phase: MachinePhase::Staging,
            reason: InstallUncertainty::Expired,
        },
        InstallOutcome::Unknown {
            phase: MachinePhase::Activating,
            reason: InstallUncertainty::Interrupted,
        },
    ];
    for outcome in cases {
        let (_root, store, intent) = interrupted_install("other-uncertain").await;
        let before = store
            .plugin_install_operation(&intent.operation_id)
            .await
            .unwrap();
        let fences = needs_reconcile(&intent);
        let machine = FakeMachine::new(outcome.clone());
        assert!(
            recover(&store, &fences, &intent, &machine).await.is_none(),
            "{outcome:?}"
        );
        assert_eq!(
            fences.read().get(&slot_of(&intent)),
            Some(&PluginFenceState::NeedsReconcile),
            "{outcome:?}"
        );
        assert_eq!(
            store
                .plugin_install_operation(&intent.operation_id)
                .await
                .unwrap(),
            before,
            "{outcome:?} leaves the operation untouched"
        );
    }
}

#[tokio::test]
async fn changed_target_offline_machine_or_live_fence_stays_fenced() {
    // The Machine slot no longer matches the original target.
    let (_root, store, intent) = interrupted_install("changed-target").await;
    let fences = needs_reconcile(&intent);
    let mut machine = FakeMachine::new(interrupted());
    machine.target = Some(InstallTarget::Removed {
        revision: format!("installation-{}", "e".repeat(64))
            .try_into()
            .unwrap(),
    });
    assert!(recover(&store, &fences, &intent, &machine).await.is_none());
    assert_eq!(
        fences.read().get(&slot_of(&intent)),
        Some(&PluginFenceState::NeedsReconcile)
    );
    assert!(
        store
            .plugin_install_staging_resolution(&intent.operation_id)
            .await
            .unwrap()
            .is_none()
    );

    // Installation admission disabled or target unobservable.
    machine.target = None;
    assert!(recover(&store, &fences, &intent, &machine).await.is_none());
    assert_eq!(
        fences.read().get(&slot_of(&intent)),
        Some(&PluginFenceState::NeedsReconcile)
    );

    // The connection that produced the evidence was replaced.
    let (_root, store, intent) = interrupted_install("replaced-connection").await;
    let fences = needs_reconcile(&intent);
    let mut machine = FakeMachine::new(interrupted());
    machine.current = false;
    assert!(recover(&store, &fences, &intent, &machine).await.is_none());
    assert_eq!(
        fences.read().get(&slot_of(&intent)),
        Some(&PluginFenceState::NeedsReconcile)
    );

    // A live install or Operator reconciliation owns the slot: no Machine query.
    let (_root, store, intent) = interrupted_install("live-fence").await;
    let fences: PluginLifecycleFences = Arc::new(parking_lot::RwLock::new(
        [(slot_of(&intent), PluginFenceState::Installing)]
            .into_iter()
            .collect(),
    ));
    let machine = FakeMachine::new(interrupted());
    assert!(recover(&store, &fences, &intent, &machine).await.is_none());
    assert_eq!(machine.queries.load(Ordering::SeqCst), 0);
    assert_eq!(
        fences.read().get(&slot_of(&intent)),
        Some(&PluginFenceState::Installing)
    );
}
