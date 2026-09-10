use super::*;
use crate::plugin_operation::fixture;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn recovery_assessment_preserves_service_fences_and_detects_changes_during_query() {
    use crate::machine_protocol::plugin_recovery::{InstallationEvidence, RecoverySnapshot};
    use crate::machine_protocol::{MachineCommand, MachineEvent};
    for race in [false, true] {
        let (_root, store, intent) = setup("recovery-inspection").await;
        let before = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        let control = MachineControl::default();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install(intent.machine_id.clone(), "epoch".into(), false, 12, tx);
        let query = inspect_recovery(&store, before.clone(), &control, &connection);
        let reply = async {
            let MachineCommand::QueryPluginUninstallRecovery { request_id, step } =
                commands.recv().await.unwrap()
            else {
                panic!("observation may only send the read command")
            };
            assert_eq!(*step, intent.machine_step().unwrap());
            if race {
                store
                    .advance_plugin_uninstall(
                        &intent.operation_id,
                        Phase::Prepared,
                        Phase::StoppingSessions,
                        None,
                    )
                    .await
                    .unwrap();
            }
            control.record_remote(
                &connection,
                MachineEvent::PluginUninstallRecovery {
                    request_id,
                    observation: Box::new(RecoveryObservation::Observed {
                        snapshot: Box::new(RecoverySnapshot {
                            request_digest: step.request_digest().unwrap(),
                            receipt: None,
                            installation: InstallationEvidence::Untracked {},
                            slot_fenced: false,
                        }),
                    }),
                },
            );
        };
        let (assessment, ()) = tokio::join!(query, reply);
        let after = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        if race {
            assert!(
                assessment.is_err(),
                "changed Service evidence invalidates the assessment"
            );
            assert_eq!(after.phase, Phase::StoppingSessions);
        } else {
            assert_eq!(after, before);
            let body = serde_json::to_value(assessment.unwrap()).unwrap();
            assert_eq!(body["recovery_execution_available"], false);
            assert_eq!(body["reconciliation_performed"], false);
            assert_eq!(body["not_verified"].as_array().unwrap().len(), 4);
            assert!(!body.to_string().contains("user-test"));
            assert!(!body.to_string().contains("session_ids"));
        }
        assert!(
            commands.try_recv().is_err(),
            "no hidden recovery or worker command"
        );
    }
}

#[test]
fn durable_step_result_never_infers_success_from_missing_or_unknown_evidence() {
    use crate::machine_protocol::plugin_step::{
        StepReceipt, StepRejection, StepUnavailable, StepUncertainty,
    };
    let step = fixture("receipt-kind").machine_step().unwrap();
    for outcome in [
        StepOutcome::Applied {},
        StepOutcome::Rejected {
            reason: StepRejection::TargetChanged,
        },
        StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted,
        },
    ] {
        let result = require_applied_step(StepLookup::Found {
            receipt: Box::new(StepReceipt {
                step: step.clone(),
                request_digest: step.request_digest().unwrap(),
                outcome: outcome.clone(),
            }),
        });
        match outcome {
            StepOutcome::Applied {} => assert!(result.is_ok()),
            StepOutcome::Rejected { .. } => {
                assert_eq!(result.unwrap_err().certainty, CommandFailure::Rejected)
            }
            StepOutcome::Unknown { .. } => {
                assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown)
            }
        }
    }
    for result in [
        StepLookup::NotFound {},
        StepLookup::Unavailable {
            reason: StepUnavailable::Storage,
        },
    ] {
        assert_eq!(
            require_applied_step(result).unwrap_err().certainty,
            CommandFailure::Unknown
        );
    }
}

struct MockEffects {
    store: Store,
    outcome: Option<CommandFailure>,
    restoration_fails: bool,
    uninstalls: AtomicUsize,
    restorations: AtomicUsize,
    stops: AtomicUsize,
    reloads: AtomicUsize,
    checks: AtomicUsize,
    deny_at: Option<Phase>,
    deny_after_checks: Option<usize>,
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl MockEffects {
    fn new(store: Store, outcome: Option<CommandFailure>) -> Self {
        Self {
            store,
            outcome,
            restoration_fails: false,
            uninstalls: AtomicUsize::new(0),
            restorations: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            reloads: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
            deny_at: None,
            deny_after_checks: None,
            gate: None,
        }
    }
}

impl Effects for MockEffects {
    async fn authorized(&self, intent: &UninstallIntent) -> bool {
        let count = self.checks.fetch_add(1, Ordering::SeqCst);
        let phase = self
            .store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .phase;
        self.deny_at != Some(phase) && self.deny_after_checks.is_none_or(|limit| count < limit)
    }

    fn stop(&self, _: &str) -> bool {
        self.stops.fetch_add(1, Ordering::SeqCst);
        false
    }
    fn reload(&self, _: &str) -> Result<(), String> {
        self.reloads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn uninstall(&self, intent: &UninstallIntent) -> Result<(), CommandRequestError> {
        let saved = self
            .store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&saved.intent, intent);
        assert_eq!(
            saved.phase,
            Phase::Uninstalling,
            "durable intent must precede the remote mutation"
        );
        self.uninstalls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        match &self.outcome {
            None => Ok(()),
            Some(certainty) => Err(CommandRequestError {
                certainty: match certainty {
                    CommandFailure::NotSent => CommandFailure::NotSent,
                    CommandFailure::Rejected => CommandFailure::Rejected,
                    CommandFailure::Unknown => CommandFailure::Unknown,
                },
                detail: "private remote error must not be persisted".to_owned(),
            }),
        }
    }
    async fn reactivate(&self, intent: &UninstallIntent) -> Result<(), CommandRequestError> {
        assert_eq!(
            self.store
                .plugin_uninstall_operation(&intent.operation_id)
                .await
                .unwrap()
                .unwrap()
                .phase,
            Phase::RestoringMachine
        );
        self.restorations.fetch_add(1, Ordering::SeqCst);
        if self.restoration_fails {
            Err(CommandRequestError {
                certainty: CommandFailure::Unknown,
                detail: "private restoration failure".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn current_authority_is_required_at_each_durable_effect_boundary() {
    for denied in [
        Phase::Prepared,
        Phase::StoppingSessions,
        Phase::Uninstalling,
        Phase::MachineUninstalled,
    ] {
        let (_root, store, mut intent) = setup("before-impact").await;
        store
            .advance_plugin_uninstall(&intent.operation_id, Phase::Prepared, Phase::Aborted, None)
            .await
            .unwrap();
        intent.operation_id = fixture("with-impact").operation_id;
        intent.session_ids = vec!["session-a".into(), "session-b".into()];
        store.begin_plugin_uninstall(&intent).await.unwrap();
        let mut effects = MockEffects::new(store.clone(), None);
        effects.deny_at = Some(denied);
        let phase = execute(&store, &intent, &effects).await.unwrap();
        assert_eq!(
            phase,
            if denied == Phase::Prepared {
                Phase::Aborted
            } else {
                Phase::NeedsAttention
            }
        );
        assert_eq!(
            effects.uninstalls.load(Ordering::SeqCst),
            usize::from(denied == Phase::MachineUninstalled)
        );
        assert_eq!(effects.restorations.load(Ordering::SeqCst), 0);
        let saved = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.problem, Some(Problem::PreconditionsChanged));
        if phase == Phase::NeedsAttention {
            assert_eq!(saved.attention_from, Some(denied));
        }
    }
}

#[tokio::test]
async fn authority_loss_between_worker_stops_preserves_the_partial_operation() {
    let (_root, store, mut intent) = setup("before-stop").await;
    store
        .advance_plugin_uninstall(&intent.operation_id, Phase::Prepared, Phase::Aborted, None)
        .await
        .unwrap();
    intent.operation_id = fixture("partial-stop").operation_id;
    intent.session_ids = vec!["session-a".into(), "session-b".into()];
    store.begin_plugin_uninstall(&intent).await.unwrap();
    let mut effects = MockEffects::new(store.clone(), None);
    effects.deny_after_checks = Some(2); // initial admission, then the first stop
    assert_eq!(
        execute(&store, &intent, &effects).await.unwrap(),
        Phase::NeedsAttention
    );
    assert_eq!(effects.stops.load(Ordering::SeqCst), 1);
    assert_eq!(effects.uninstalls.load(Ordering::SeqCst), 0);
    assert_eq!(effects.restorations.load(Ordering::SeqCst), 0);
    assert!(
        !recover_fences(Some(&store), "service-test")
            .await
            .unwrap()
            .read()
            .is_empty()
    );
}

#[tokio::test]
async fn compensation_and_worker_reload_cannot_use_revoked_forward_approval() {
    for denied in [Phase::RestoringMachine, Phase::RestoringSessions] {
        let (_root, store, mut intent) = setup("before-recovery").await;
        store
            .advance_plugin_uninstall(&intent.operation_id, Phase::Prepared, Phase::Aborted, None)
            .await
            .unwrap();
        intent.operation_id = fixture("recover-workers").operation_id;
        intent.session_ids = vec!["session-a".into()];
        intent.live_session_ids = intent.session_ids.clone();
        store.begin_plugin_uninstall(&intent).await.unwrap();
        for (from, to) in [
            (Phase::Prepared, Phase::StoppingSessions),
            (Phase::StoppingSessions, Phase::Uninstalling),
        ] {
            store
                .advance_plugin_uninstall(&intent.operation_id, from, to, None)
                .await
                .unwrap();
        }
        let mut effects = MockEffects::new(store.clone(), Some(CommandFailure::Rejected));
        effects.deny_at = Some(denied);
        assert_eq!(
            compensate(
                &store,
                &intent,
                &effects,
                Phase::Uninstalling,
                Problem::MachineRejected
            )
            .await
            .unwrap(),
            Phase::NeedsAttention
        );
        assert_eq!(
            effects.restorations.load(Ordering::SeqCst),
            usize::from(denied == Phase::RestoringSessions)
        );
        assert_eq!(effects.reloads.load(Ordering::SeqCst), 0);
        let saved = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.problem, Some(Problem::CompensationFailed));
        assert_eq!(saved.cause, Some(Problem::MachineRejected));
    }
}

async fn setup(id: &str) -> (tempfile::TempDir, Store, UninstallIntent) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture(id);
    store.begin_plugin_uninstall(&intent).await.unwrap();
    (root, store, intent)
}

#[tokio::test]
async fn successful_uninstall_persists_completion_and_releases_only_its_slot() {
    let (_root, store, intent) = setup("success").await;
    let effects = MockEffects::new(store.clone(), None);
    assert_eq!(
        execute(&store, &intent, &effects).await.unwrap(),
        Phase::Completed
    );
    assert_eq!(effects.uninstalls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.restorations.load(Ordering::SeqCst), 0);
    assert!(
        recover_fences(Some(&store), "service-test")
            .await
            .unwrap()
            .read()
            .is_empty()
    );
}

#[tokio::test]
async fn ambiguous_or_unsent_remote_result_never_replays_or_reactivates() {
    for certainty in [CommandFailure::Unknown, CommandFailure::NotSent] {
        let (_root, store, intent) = setup("unknown").await;
        let effects = MockEffects::new(store.clone(), Some(certainty));
        assert_eq!(
            execute(&store, &intent, &effects).await.unwrap(),
            Phase::NeedsAttention
        );
        assert_eq!(effects.restorations.load(Ordering::SeqCst), 0);
        let fences = recover_fences(Some(&store), "service-test").await.unwrap();
        assert_eq!(
            fences
                .read()
                .get(&("hawk".to_owned(), "victoria".to_owned())),
            Some(&PluginFenceState::NeedsReconcile)
        );
        assert!(
            OperationFence::acquire(&fences, ("hawk".to_owned(), "victoria".to_owned())).is_err()
        );
        let independent =
            OperationFence::acquire(&fences, ("falcon".to_owned(), "victoria".to_owned())).unwrap();
        drop(independent);
        assert_eq!(fences.read().len(), 1);
        let saved = serde_json::to_string(
            &store
                .plugin_uninstall_operation(&intent.operation_id)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!saved.contains("private"));
        assert!(execute(&store, &intent, &effects).await.is_err());
        assert_eq!(effects.uninstalls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn explicit_failure_records_compensation_and_keeps_failed_recovery_visible() {
    for fails in [false, true] {
        let (_root, store, intent) = setup("rejected").await;
        let mut effects = MockEffects::new(store.clone(), Some(CommandFailure::Rejected));
        effects.restoration_fails = fails;
        assert_eq!(
            execute(&store, &intent, &effects).await.unwrap(),
            if fails {
                Phase::NeedsAttention
            } else {
                Phase::Compensated
            }
        );
        assert_eq!(effects.restorations.load(Ordering::SeqCst), 1);
        let saved = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.problem,
            Some(if fails {
                Problem::CompensationFailed
            } else {
                Problem::MachineRejected
            })
        );
        assert_eq!(
            saved.cause,
            Some(Problem::MachineRejected),
            "primary failure survives failed compensation"
        );
    }
}

#[tokio::test]
async fn local_commit_failure_compensates_but_committed_delete_never_does() {
    let (_root, store, mut intent) = setup("unused").await;
    store
        .advance_plugin_uninstall(&intent.operation_id, Phase::Prepared, Phase::Aborted, None)
        .await
        .unwrap();
    intent.operation_id = fixture("missing-session").operation_id;
    intent.session_ids.push("sess-404".to_owned());
    store.begin_plugin_uninstall(&intent).await.unwrap();
    let effects = MockEffects::new(store.clone(), None);
    assert_eq!(
        execute(&store, &intent, &effects).await.unwrap(),
        Phase::Compensated
    );
    assert_eq!(effects.restorations.load(Ordering::SeqCst), 1);
    let (_root2, store2, committed) = setup("committed").await;
    let effects2 = MockEffects::new(store2.clone(), None);
    execute(&store2, &committed, &effects2).await.unwrap();
    assert_eq!(
        compensate(
            &store2,
            &committed,
            &effects2,
            Phase::MachineUninstalled,
            Problem::StorageFailure
        )
        .await
        .unwrap(),
        Phase::Completed
    );
    assert_eq!(
        effects2.restorations.load(Ordering::SeqCst),
        0,
        "lost local COMMIT acknowledgement cannot undo committed deletion"
    );
}

#[tokio::test]
async fn observer_drop_does_not_cancel_admitted_execution() {
    let (_root, store, intent) = setup("observer").await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut effects = MockEffects::new(store.clone(), None);
    effects.gate = Some(Arc::clone(&gate));
    let effects = Arc::new(effects);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn({
        let store = store.clone();
        let intent = intent.clone();
        let effects = Arc::clone(&effects);
        async move {
            let result = execute(&store, &intent, effects.as_ref()).await;
            let _ = done_tx.send(result);
        }
    });
    drop(handle);
    gate.add_permits(1);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), done_rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Phase::Completed
    );
    assert_eq!(effects.uninstalls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn forced_task_interruption_recovers_the_persisted_remote_window() {
    let (_root, store, intent) = setup("abort").await;
    let mut effects = MockEffects::new(store.clone(), None);
    effects.gate = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let effects = Arc::new(effects);
    let handle = tokio::spawn({
        let store = store.clone();
        let intent = intent.clone();
        let effects = Arc::clone(&effects);
        async move { execute(&store, &intent, effects.as_ref()).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while effects.uninstalls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    handle.abort();
    assert!(handle.await.unwrap_err().is_cancelled());
    recover_fences(Some(&store), "service-test").await.unwrap();
    let saved = store
        .plugin_uninstall_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.phase, Phase::NeedsAttention);
    assert_eq!(saved.attention_from, Some(Phase::Uninstalling));
    assert_eq!(effects.restorations.load(Ordering::SeqCst), 0);
}

#[test]
fn canceled_admitted_scope_and_alternate_worker_entrypoints_keep_the_fence() {
    let fences = Arc::new(parking_lot::RwLock::new(HashMap::new()));
    let key = ("hawk".to_owned(), "codex".to_owned());
    let mut fence = OperationFence::acquire(&fences, key.clone()).unwrap();
    fence.keep = true;
    drop(fence);
    assert_eq!(
        fences.read().get(&key),
        Some(&PluginFenceState::NeedsReconcile)
    );
    for command in [
        Inbound::OpenSession {
            session_id: "sess-1".to_owned(),
        },
        Inbound::ResetSession {
            session_id: "sess-1".to_owned(),
        },
        Inbound::RetryTurn {
            session_id: "sess-1".to_owned(),
        },
    ] {
        assert!(requires_runtime_fence(&command));
    }
    assert!(!requires_runtime_fence(&Inbound::Cancel {
        session_id: "sess-1".to_owned()
    }));
}

#[tokio::test]
async fn accepted_worker_reload_is_not_a_verified_restoration() {
    let (_root, store, mut intent) = setup("old").await;
    store
        .advance_plugin_uninstall(&intent.operation_id, Phase::Prepared, Phase::Aborted, None)
        .await
        .unwrap();
    intent.operation_id = fixture("worker-reload").operation_id;
    intent.session_ids = vec!["sess-1".to_owned()];
    intent.live_session_ids = intent.session_ids.clone();
    store.begin_plugin_uninstall(&intent).await.unwrap();
    for (a, b) in [
        (Phase::Prepared, Phase::StoppingSessions),
        (Phase::StoppingSessions, Phase::Uninstalling),
    ] {
        store
            .advance_plugin_uninstall(&intent.operation_id, a, b, None)
            .await
            .unwrap();
    }
    let effects = MockEffects::new(store.clone(), None);
    assert_eq!(
        compensate(
            &store,
            &intent,
            &effects,
            Phase::Uninstalling,
            Problem::MachineRejected
        )
        .await
        .unwrap(),
        Phase::NeedsAttention
    );
    assert_eq!(
        store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .problem,
        Some(Problem::WorkerRecoveryUnverified)
    );
}

#[test]
fn foreign_or_incomplete_confirmation_does_not_consume_the_owner_preview() {
    let intent = fixture("preview");
    let plan = PluginUninstallPlan {
        actor: intent.actor.clone(),
        machine_id: intent.machine_id.clone(),
        plugin_id: intent.plugin_id.clone(),
        plugin_version: intent.plugin_version,
        generation_digest: intent.generation_digest,
        installation_revision: intent.installation_revision,
        contract_fingerprint: intent.contract_fingerprint,
        session_ids: vec!["sess-1".to_owned()],
        active_session_ids: vec!["sess-1".to_owned()],
        purge_after_ms: intent.purge_after_ms,
        expires_at_ms: intent.expires_at_ms,
    };
    let mut plans = HashMap::from([(intent.operation_id.clone(), plan)]);
    let mut request = PluginUninstallRequest {
        plan_id: intent.operation_id.clone(),
        confirm_active_sessions: true,
    };
    let other = Actor::Admin {
        account: "unrelated".to_owned(),
    };
    assert!(consume_preview(&mut plans, &other, "hawk", "victoria", &request, now_ms()).is_err());
    assert!(
        consume_preview(
            &mut plans,
            &intent.actor,
            "falcon",
            "victoria",
            &request,
            now_ms()
        )
        .is_err()
    );
    assert!(
        consume_preview(
            &mut plans,
            &intent.actor,
            "hawk",
            "victoria",
            &request,
            intent.expires_at_ms + 1
        )
        .is_err()
    );
    request.confirm_active_sessions = false;
    assert!(
        consume_preview(
            &mut plans,
            &intent.actor,
            "hawk",
            "victoria",
            &request,
            now_ms()
        )
        .is_err()
    );
    assert_eq!(plans.len(), 1);
    request.confirm_active_sessions = true;
    consume_preview(
        &mut plans,
        &intent.actor,
        "hawk",
        "victoria",
        &request,
        now_ms(),
    )
    .unwrap();
    assert!(plans.is_empty());
}
