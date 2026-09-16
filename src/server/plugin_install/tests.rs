use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

mod journal;

// Preserve the original continuation/transport tests while exercising the
// actual durable coordinator, not a parallel in-memory implementation.
async fn coordinate(effects: &impl Effects, fence: &mut InstallationFence) -> Outcome {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = crate::plugin_operation::installation::machine_fixture("continuation");
    store.begin_plugin_install(&intent).await.unwrap();
    fence.disposition = Disposition::Uncertain;
    super::coordinate(effects, fence, &mut Progress::new(&store, &intent))
        .await
        .unwrap()
}

fn slot() -> (String, String) {
    ("machine-test".into(), "victoria".into())
}

fn fences(previous: Option<PluginFenceState>) -> PluginLifecycleFences {
    Arc::new(parking_lot::RwLock::new(
        previous.map(|p| (slot(), p)).into_iter().collect(),
    ))
}

struct MockEffects {
    checks: AtomicUsize,
    syncs: AtomicUsize,
    installs: AtomicUsize,
    deny_at: usize,
    sync: bool,
    sync_fails_at: usize,
    failure: Option<CommandFailure>,
    started: tokio::sync::Notify,
    gate: Option<Arc<tokio::sync::Semaphore>>,
    panic: bool,
    outcome: Option<InstallOutcome>,
    unavailable: bool,
    changed_receipt: bool,
}

impl Default for MockEffects {
    fn default() -> Self {
        Self {
            checks: AtomicUsize::new(0),
            syncs: AtomicUsize::new(0),
            installs: AtomicUsize::new(0),
            deny_at: usize::MAX,
            sync: true,
            sync_fails_at: usize::MAX,
            failure: None,
            started: tokio::sync::Notify::new(),
            gate: None,
            panic: false,
            outcome: None,
            unavailable: false,
            changed_receipt: false,
        }
    }
}

impl Effects for MockEffects {
    async fn authorized(&self) -> Result<(), Precondition> {
        // The fake denies by call index; which precondition it blames is not
        // what these tests are about, so they all use one.
        if self.checks.fetch_add(1, Ordering::SeqCst) == self.deny_at {
            return Err(Precondition::OperatorApproval);
        }
        Ok(())
    }
    async fn needs_auth_sync(&self, _: bool) -> bool {
        self.sync
    }
    async fn sync_auth(&self) -> bool {
        self.syncs.fetch_add(1, Ordering::SeqCst) != self.sync_fails_at
    }
    async fn install(&self, step: &InstallStep) -> Result<InstallObservation, CommandRequestError> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        assert!(!self.panic, "injected coordinator interruption");
        if let Some(failure) = &self.failure {
            return Err(CommandRequestError {
                certainty: match failure {
                    CommandFailure::NotSent => CommandFailure::NotSent,
                    CommandFailure::Rejected => CommandFailure::Rejected,
                    CommandFailure::Unknown => CommandFailure::Unknown,
                },
                detail: "private Machine error must never enter HTTP output".into(),
            });
        }
        let mut receipt = machine_receipt(step, self.outcome.clone().unwrap_or_else(applied));
        if self.changed_receipt {
            receipt.step.plan_digest = format!("sha256:{}", "f".repeat(64));
        }
        Ok(InstallObservation {
            admission_enabled: false,
            result: if self.unavailable {
                InstallLookup::Unavailable {
                    reason: crate::machine_protocol::plugin_install::InstallUnavailable::Storage,
                }
            } else {
                InstallLookup::Found {
                    receipt: Box::new(receipt),
                }
            },
        })
    }
}

fn applied() -> InstallOutcome {
    InstallOutcome::Applied {
        revision: format!("installation-{}", "d".repeat(64))
            .try_into()
            .unwrap(),
    }
}

fn machine_receipt(
    step: &InstallStep,
    outcome: InstallOutcome,
) -> crate::machine_protocol::plugin_install::InstallReceipt {
    crate::machine_protocol::plugin_install::InstallReceipt {
        step: step.clone(),
        request_digest: step.request_digest().unwrap(),
        outcome,
    }
}

#[test]
fn exact_release_is_required_and_unrecognized_install_fields_are_rejected() {
    for body in [
        "{}",
        r#"{"version":"1.0.0"}"#,
        r#"{"digest":"sha256:abc"}"#,
        r#"{"version":null,"digest":"sha256:abc"}"#,
        r#"{"version":"1.0.0","digest":"sha256:abc","url":"https://untrusted.invalid"}"#,
    ] {
        assert!(serde_json::from_str::<PluginInstallRequest>(body).is_err());
    }
    assert!(
        serde_json::from_str::<PluginInstallRequest>(
            r#"{"operation_id":"installation-fixture","version":"1.0.0","digest":"sha256:abc"}"#
        )
        .is_ok()
    );
}

/// A refusal has to say what to fix. Before this, every one of these answered
/// with the same sentence — "confirmation, compatibility, connection or
/// authentication preconditions changed" — which is true of all of them and
/// actionable for none, so an operator converging a fleet could not tell a
/// Machine that needs updating from one that needs reconnecting.
#[test]
fn each_refused_precondition_says_what_to_fix() {
    let details: Vec<&str> = [
        Precondition::OperatorApproval,
        Precondition::MachineConnection,
        Precondition::MachineAdmission,
        Precondition::MachineTarget,
        Precondition::CatalogRelease,
        Precondition::MachineCapability,
        Precondition::Storage,
    ]
    .into_iter()
    .map(Precondition::detail)
    .collect();
    for detail in &details {
        assert!(
            detail.starts_with("Plugin installation was not sent: "),
            "{detail}"
        );
        assert_eq!(details.iter().filter(|other| *other == detail).count(), 1);
    }
    // The two Machine-side refusals need opposite actions, so they must not
    // read alike: admission disabled is "update Cowboy Machine there", a lost
    // connection is "reconnect it".
    assert!(
        Precondition::MachineAdmission
            .detail()
            .contains("admission disabled")
    );
    assert!(
        Precondition::MachineAdmission
            .detail()
            .contains("Update Cowboy Machine")
    );
    assert!(
        Precondition::MachineConnection
            .detail()
            .contains("control connection")
    );
    assert!(
        Precondition::MachineConnection
            .detail()
            .contains("same operation ID")
    );
    // A named refusal is still a 409 carrying the same durable problem.
    let response = Outcome::NotDispatched(Precondition::MachineAdmission).response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[test]
fn reservation_preserves_existing_lifecycle_fences_and_other_slots() {
    for previous in [
        PluginFenceState::Installing,
        PluginFenceState::Uninstalling,
        PluginFenceState::NeedsReconcile,
    ] {
        let fences = fences(Some(previous));
        assert!(InstallationFence::acquire(&fences, slot()).is_err());
        assert_eq!(fences.read().get(&slot()), Some(&previous));
    }
    let fences = fences(None);
    let owner = InstallationFence::acquire(&fences, slot()).unwrap();
    assert!(InstallationFence::acquire(&fences, slot()).is_err());
    let other = ("other-machine".into(), "victoria".into());
    let other_owner = InstallationFence::acquire(&fences, other.clone()).unwrap();
    drop(owner);
    assert!(!fences.read().contains_key(&slot()));
    assert_eq!(
        fences.read().get(&other),
        Some(&PluginFenceState::Installing)
    );
    drop(other_owner);
    assert!(fences.read().is_empty());
}

#[tokio::test]
async fn every_effect_checks_authority_and_stops_after_revocation() {
    for previous in [None, Some(PluginFenceState::Uninstalled)] {
        for deny_at in 0..4 {
            let fences = fences(previous);
            let effects = MockEffects {
                deny_at,
                ..MockEffects::default()
            };
            let mut owner = InstallationFence::acquire(&fences, slot()).unwrap();
            let outcome = coordinate(&effects, &mut owner).await;
            drop(owner);
            let installed = deny_at == 3;
            assert_eq!(
                outcome,
                if installed {
                    Outcome::AuthenticationPending
                } else {
                    Outcome::NotDispatched(Precondition::OperatorApproval)
                }
            );
            assert_eq!(
                effects.installs.load(Ordering::SeqCst),
                usize::from(installed)
            );
            assert_eq!(
                effects.syncs.load(Ordering::SeqCst),
                usize::from(deny_at >= 2)
            );
            assert_eq!(
                fences.read().get(&slot()).copied(),
                if installed { None } else { previous }
            );
        }
    }
}

#[tokio::test]
async fn authentication_failure_distinguishes_before_and_after_installation() {
    for sync_fails_at in [0, 1] {
        let fences = fences(Some(PluginFenceState::Uninstalled));
        let effects = MockEffects {
            sync_fails_at,
            ..MockEffects::default()
        };
        let mut owner = InstallationFence::acquire(&fences, slot()).unwrap();
        let outcome = coordinate(&effects, &mut owner).await;
        drop(owner);
        assert_eq!(
            outcome,
            if sync_fails_at == 0 {
                Outcome::NotDispatched(Precondition::OperatorApproval)
            } else {
                Outcome::AuthenticationPending
            }
        );
        assert_eq!(effects.installs.load(Ordering::SeqCst), sync_fails_at);
        assert_eq!(
            fences.read().get(&slot()).copied(),
            if sync_fails_at == 0 {
                Some(PluginFenceState::Uninstalled)
            } else {
                None
            }
        );
    }
}

#[tokio::test]
async fn only_not_sent_restores_previous_fence_and_acknowledged_install_never_replays() {
    for previous in [None, Some(PluginFenceState::Uninstalled)] {
        for failure in [
            None,
            Some(CommandFailure::NotSent),
            Some(CommandFailure::Rejected),
            Some(CommandFailure::Unknown),
        ] {
            let (expected, disposition) = match failure {
                None => (Outcome::Installed, None),
                Some(CommandFailure::NotSent) => (
                    Outcome::NotDispatched(Precondition::MachineConnection),
                    previous,
                ),
                _ => (
                    Outcome::NeedsReconcile,
                    Some(PluginFenceState::NeedsReconcile),
                ),
            };
            let fences = fences(previous);
            let effects = MockEffects {
                failure,
                sync: false,
                ..MockEffects::default()
            };
            let mut owner = InstallationFence::acquire(&fences, slot()).unwrap();
            assert_eq!(coordinate(&effects, &mut owner).await, expected);
            drop(owner);
            assert_eq!(fences.read().get(&slot()).copied(), disposition);
            assert_eq!(effects.installs.load(Ordering::SeqCst), 1);
            assert_eq!(effects.syncs.load(Ordering::SeqCst), 0);
            let body = axum::body::to_bytes(expected.response().into_body(), 4096)
                .await
                .unwrap();
            assert!(!String::from_utf8_lossy(&body).contains("private Machine error"));
        }
    }
}

#[tokio::test]
async fn dropping_http_observer_keeps_one_admitted_attempt_and_its_slot_owned() {
    let fences = fences(None);
    let owner = InstallationFence::acquire(&fences, slot()).unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let effects = Arc::new(MockEffects {
        gate: Some(Arc::clone(&gate)),
        ..MockEffects::default()
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    let running = Arc::clone(&effects);
    let observer = tokio::spawn(observe(async move {
        let mut owner = owner;
        let result = coordinate(running.as_ref(), &mut owner).await;
        drop(owner);
        tx.send(result).unwrap();
        result.response()
    }));
    effects.started.notified().await;
    observer.abort();
    assert!(observer.await.unwrap_err().is_cancelled());
    assert_eq!(
        fences.read().get(&slot()),
        Some(&PluginFenceState::Installing)
    );
    assert!(InstallationFence::acquire(&fences, slot()).is_err());
    gate.add_permits(1);
    assert_eq!(rx.await.unwrap(), Outcome::Installed);
    assert!(fences.read().is_empty());
    assert_eq!(effects.installs.load(Ordering::SeqCst), 1);
    assert_eq!(effects.syncs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn panic_after_dispatch_keeps_uncertainty_instead_of_reopening_installation() {
    let fences = fences(None);
    let owner = InstallationFence::acquire(&fences, slot()).unwrap();
    let response = observe(async move {
        let mut owner = owner;
        let effects = MockEffects {
            panic: true,
            ..MockEffects::default()
        };
        coordinate(&effects, &mut owner).await.response()
    })
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        fences.read().get(&slot()),
        Some(&PluginFenceState::NeedsReconcile)
    );
}

#[cfg(feature = "machine-host")]
#[tokio::test]
async fn transport_never_moves_installation_to_a_replacement_connection_or_retries_a_lost_ack() {
    use crate::machine_protocol::{MachineCommand, MachineEvent};
    let root = tempfile::tempdir().unwrap();
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(root.path()).unwrap();
    let desired = crate::machine_plugins::telemetry_release_for_test(&publisher, "1.1.0");
    let mut step = crate::machine_protocol::plugin_install::fixture();
    step.plugin_version
        .clone_from(&desired.release.plugin_version);
    step.generation_digest
        .clone_from(&desired.release.artifact_digest);
    step.contract_fingerprint
        .clone_from(&desired.release.contract_fingerprint);
    step.envelope_digest =
        crate::machine_protocol::plugin_step::digest(&serde_json::to_vec(&desired).unwrap());
    for change in ["before", "lost", "rejected", "applied"] {
        let control = MachineControl::default();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let original = control.install("machine-test".into(), "same-epoch".into(), false, 19, tx);
        let (replacement, mut other_commands) = tokio::sync::mpsc::unbounded_channel();
        if change == "before" {
            control.install(
                "machine-test".into(),
                "same-epoch".into(),
                false,
                19,
                replacement,
            );
            assert_eq!(
                dispatch(&control, &original, &desired, &step)
                    .await
                    .unwrap_err()
                    .certainty,
                CommandFailure::NotSent
            );
        } else {
            let (result, ()) = tokio::join!(
                dispatch(&control, &original, &desired, &step),
                async {
                    let MachineCommand::InstallPluginStep {
                        request_id,
                        plugin,
                        step: sent,
                    } = commands.recv().await.unwrap()
                    else {
                        panic!("only an exact install may be sent");
                    };
                    assert_eq!(*plugin, desired);
                    assert_eq!(*sent, step);
                    assert_eq!(request_id, format!("plugin-install-{}", step.operation_id));
                    if change == "lost" {
                        control.install(
                            "machine-test".into(),
                            "same-epoch".into(),
                            false,
                            19,
                            replacement,
                        );
                    }
                    // Even a late successful reply from the old connection cannot
                    // certify the result after its incarnation has been replaced.
                    control.record_remote(
                        &original,
                        MachineEvent::PluginInstallationStep { request_id, observation: Box::new(InstallObservation { admission_enabled: false, result: InstallLookup::Found { receipt: Box::new(machine_receipt(&step, if change == "rejected" { InstallOutcome::Rejected { reason: crate::machine_protocol::plugin_install::InstallRejection::TargetChanged } } else { applied() })) } }) },
                    );
                }
            );
            match change {
                "lost" => assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown),
                _ => assert!(matches!(
                    result.unwrap().result,
                    InstallLookup::Found { .. }
                )),
            }
        }
        assert!(commands.try_recv().is_err());
        assert!(
            other_commands.try_recv().is_err(),
            "no rebind, retry or inverse"
        );
    }
}
