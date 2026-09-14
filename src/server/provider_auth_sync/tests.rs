use super::*;
use crate::machine_protocol::{MachineEvent, ProviderAuthAction};
use std::pin::pin;
use tokio::sync::mpsc;

fn envelope(generation: u64) -> SealedProviderAuth {
    // A transport fixture only; real Machine signature/materialization checks
    // are independently required. No production credentials or runtime spawn.
    SealedProviderAuth {
        envelope_schema: 1,
        provider_id: "fixture-provider".into(),
        auth_generation: generation,
        auth_contract_fingerprint: "fixture-contract".into(),
        projection_schema: "fixture-projection".into(),
        action: ProviderAuthAction::Apply,
        ephemeral_public_key: "fixture-public".into(),
        nonce: "fixture-nonce".into(),
        ciphertext: "fixture-ciphertext".into(),
        service_public_key: "fixture-service".into(),
        signature: "fixture-signature".into(),
    }
}

fn connect(
    control: &MachineControl,
    machine: &str,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(machine.into(), "fixture-epoch".into(), false, 18, tx),
        rx,
    )
}

fn request(rx: &mut mpsc::UnboundedReceiver<MachineCommand>) -> (String, u64) {
    let MachineCommand::ApplyProviderAuth {
        request_id,
        envelope,
    } = rx.try_recv().unwrap()
    else {
        panic!("unexpected command");
    };
    (request_id, envelope.auth_generation)
}

fn reply(control: &MachineControl, connection: &ConnectionToken, request_id: String) {
    control.record_remote(
        connection,
        MachineEvent::CommandResult {
            request_id,
            accepted: true,
            detail: None,
        },
    );
}

#[tokio::test]
async fn simultaneous_distribution_shares_one_pending_apply() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    let mut resealed = envelope(1);
    resealed.nonce = "another-nonce".into();
    resealed.ciphertext = "same-Service-generation-new-seal".into();
    let mut follower = pin!(sync.apply(&control, &connection, resealed));
    assert!(futures::poll!(&mut first).is_pending());
    let (id, generation) = request(&mut rx);
    assert_eq!(generation, 1);
    assert!(futures::poll!(&mut follower).is_pending());
    assert!(
        rx.try_recv().is_err(),
        "same authority must not enqueue twice"
    );
    reply(&control, &connection, id);
    assert_eq!(first.await, Ok(1));
    assert_eq!(follower.await, Ok(1));
}

#[tokio::test]
async fn rejection_is_shared_but_later_repair_is_a_new_request() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    let mut follower = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    let (id, _) = request(&mut rx);
    assert!(futures::poll!(&mut follower).is_pending());
    control.record_remote(
        &connection,
        MachineEvent::CommandResult {
            request_id: id.clone(),
            accepted: false,
            detail: Some("fixture projection refused".into()),
        },
    );
    assert_eq!(first.await, Err("fixture projection refused".into()));
    assert_eq!(follower.await, Err("fixture projection refused".into()));
    assert!(sync.flights.lock().is_empty());
    let mut repair = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut repair).is_pending());
    let (repair_id, _) = request(&mut rx);
    assert_ne!(repair_id, id);
    reply(&control, &connection, repair_id);
    assert_eq!(repair.await, Ok(1));
    // A successful result is not cached either: repair may need to reconstruct
    // lost materialization without changing the Service generation.
    let mut later = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut later).is_pending());
    reply(&control, &connection, request(&mut rx).0);
    assert_eq!(later.await, Ok(1));
}

#[tokio::test]
async fn independent_authorities_do_not_merge() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let other_service = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let (other_connection, mut other_rx) = connect(&control, "machine-2");
    let mut other_provider = envelope(1);
    other_provider.provider_id = "another-provider".into();
    let mut futures = [
        Box::pin(sync.apply(&control, &connection, envelope(1))),
        Box::pin(sync.apply(&control, &connection, envelope(2))),
        Box::pin(sync.apply(&control, &connection, other_provider)),
        Box::pin(sync.apply(&control, &other_connection, envelope(1))),
        Box::pin(other_service.apply(&control, &connection, envelope(1))),
    ];
    for future in &mut futures {
        assert!(futures::poll!(future).is_pending());
    }
    for expected_generation in [1, 2, 1, 1] {
        let (id, generation) = request(&mut rx);
        assert_eq!(generation, expected_generation);
        reply(&control, &connection, id);
    }
    reply(&control, &other_connection, request(&mut other_rx).0);
    for (future, generation) in futures.into_iter().zip([1, 2, 1, 1, 1]) {
        assert_eq!(future.await, Ok(generation));
    }
}

#[tokio::test]
async fn conflicting_generation_metadata_is_not_joined() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    let (id, _) = request(&mut rx);
    for field in 0..5 {
        let mut conflicting = envelope(1);
        match field {
            0 => conflicting.envelope_schema += 1,
            1 => conflicting.auth_contract_fingerprint = "different-contract".into(),
            2 => conflicting.projection_schema = "different-projection".into(),
            3 => conflicting.action = ProviderAuthAction::Wipe,
            4 => conflicting.service_public_key = "different-service".into(),
            _ => unreachable!(),
        }
        assert_eq!(
            sync.apply(&control, &connection, conflicting).await,
            Err("conflicting Provider auth synchronization identity".into())
        );
        assert!(rx.try_recv().is_err());
    }
    reply(&control, &connection, id);
    assert_eq!(first.await, Ok(1));
}

#[tokio::test]
async fn replacement_even_with_the_same_epoch_never_joins_or_retargets() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (old, mut old_rx) = connect(&control, "machine-1");
    let mut first = Box::pin(sync.apply(&control, &old, envelope(1)));
    let mut old_follower = pin!(sync.apply(&control, &old, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    let (old_id, _) = request(&mut old_rx);
    assert!(futures::poll!(&mut old_follower).is_pending());
    let (current, mut rx) = connect(&control, "machine-1");
    assert!(!old.same(&current));
    assert_eq!(
        sync.apply(&control, &old, envelope(1)).await,
        Err(DISCONNECTED.into())
    );
    assert!(rx.try_recv().is_err(), "must not retarget a stale attempt");
    let mut replacement = pin!(sync.apply(&control, &current, envelope(1)));
    assert!(futures::poll!(&mut replacement).is_pending());
    let (new_id, _) = request(&mut rx);
    assert_ne!(old_id, new_id);
    // Both an old request ID and a forged new request ID on the stale token
    // fail to complete the new connection's operation.
    reply(&control, &old, old_id);
    reply(&control, &old, new_id.clone());
    assert!(futures::poll!(&mut replacement).is_pending());
    drop(first);
    assert_eq!(old_follower.await, Err(DISCONNECTED.into()));
    assert_eq!(sync.flights.lock().len(), 1);
    let mut new_follower = pin!(sync.apply(&control, &current, envelope(1)));
    assert!(futures::poll!(&mut new_follower).is_pending());
    assert!(
        rx.try_recv().is_err(),
        "old cleanup must not erase the new flight"
    );
    reply(&control, &current, new_id);
    assert_eq!(replacement.await, Ok(1));
    assert_eq!(new_follower.await, Ok(1));
}

#[tokio::test]
async fn disconnect_after_ack_before_owner_poll_cannot_report_current() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    let mut follower = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut follower).is_pending());
    reply(&control, &connection, request(&mut rx).0);
    let (_new_connection, mut new_rx) = connect(&control, "machine-1");
    assert_eq!(first.await, Err(DISCONNECTED.into()));
    assert_eq!(follower.await, Err(DISCONNECTED.into()));
    assert!(new_rx.try_recv().is_err());
    assert!(sync.flights.lock().is_empty());
}

#[tokio::test]
async fn disconnect_after_owner_completion_still_fences_observer_result() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    let mut follower = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut follower).is_pending());
    reply(&control, &connection, request(&mut rx).0);
    assert_eq!(first.await, Ok(1));
    let (_new_connection, mut new_rx) = connect(&control, "machine-1");
    assert_eq!(follower.await, Err(DISCONNECTED.into()));
    assert!(new_rx.try_recv().is_err());
}

#[tokio::test]
async fn cancelling_owner_wakes_observers_without_retrying() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = Box::pin(sync.apply(&control, &connection, envelope(1)));
    let mut follower = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    let (old_id, _) = request(&mut rx);
    assert!(futures::poll!(&mut follower).is_pending());
    drop(first);
    assert_eq!(follower.await, Err(CANCELLED.into()));
    assert!(rx.try_recv().is_err());
    assert!(sync.flights.lock().is_empty());
    let mut repair = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut repair).is_pending());
    let (repair_id, _) = request(&mut rx);
    assert_ne!(repair_id, old_id);
    reply(&control, &connection, old_id);
    assert!(futures::poll!(&mut repair).is_pending());
    reply(&control, &connection, repair_id);
    assert_eq!(repair.await, Ok(1));
}

#[tokio::test]
async fn cancelling_observer_does_not_cancel_owner_or_other_observers() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    let mut cancelled = Box::pin(sync.apply(&control, &connection, envelope(1)));
    let mut surviving = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut cancelled).is_pending());
    assert!(futures::poll!(&mut surviving).is_pending());
    drop(cancelled);
    assert_eq!(sync.flights.lock()[0].result.receiver_count(), 1);
    reply(&control, &connection, request(&mut rx).0);
    assert_eq!(first.await, Ok(1));
    assert_eq!(surviving.await, Ok(1));
    assert!(rx.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn late_observers_share_the_original_ninety_second_deadline() {
    use std::time::Duration;
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    let mut first = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    request(&mut rx);
    tokio::time::advance(Duration::from_secs(89)).await;
    let mut follower = pin!(sync.apply(&control, &connection, envelope(1)));
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut follower).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(first.await, Err("Machine command timed out".into()));
    assert_eq!(follower.await, Err("Machine command timed out".into()));
    tokio::time::advance(Duration::from_secs(180)).await;
    assert!(
        rx.try_recv().is_err(),
        "timeout must not schedule another Apply"
    );
    assert!(sync.flights.lock().is_empty());
}

#[test]
fn flight_and_observer_budgets_are_bounded_and_reclaimed() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, _rx) = connect(&control, "machine-1");
    let mut owners = (0..MAX_FLIGHTS)
        .map(|generation| {
            sync.begin(&connection, Identity::from(&envelope(generation as u64)))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sync.begin(&connection, Identity::from(&envelope(1000)))
            .err(),
        Some("Provider auth sync flight budget exceeded".into())
    );
    // Full owner capacity still permits bounded observers of an existing flight.
    let mut observers = (0..MAX_OBSERVERS)
        .map(|_| {
            sync.begin(&connection, Identity::from(&envelope(1)))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sync.begin(&connection, Identity::from(&envelope(1))).err(),
        Some("Provider auth sync observer budget exceeded".into())
    );
    drop(observers.pop());
    observers.push(
        sync.begin(&connection, Identity::from(&envelope(1)))
            .unwrap(),
    );
    drop(owners.pop());
    owners.push(
        sync.begin(&connection, Identity::from(&envelope(1000)))
            .unwrap(),
    );
    drop(owners);
    drop(observers);
    assert!(sync.flights.lock().is_empty());
}

#[test]
fn completed_owner_cannot_cache_success_or_erase_a_new_repair() {
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, _rx) = connect(&control, "machine-1");
    let Admission::Owner(first) = sync
        .begin(&connection, Identity::from(&envelope(1)))
        .unwrap()
    else {
        panic!("first request must own the flight");
    };
    // Model another thread entering begin between result publication and Drop.
    first.flight.result.send_replace(Some(Ok(())));
    let Admission::Owner(repair) = sync
        .begin(&connection, Identity::from(&envelope(1)))
        .unwrap()
    else {
        panic!("completed result must not replace real repair");
    };
    drop(first);
    assert_eq!(sync.flights.lock().len(), 1);
    assert!(matches!(
        sync.begin(&connection, Identity::from(&envelope(1)))
            .unwrap(),
        Admission::Observer(_)
    ));
    drop(repair);
    assert!(sync.flights.lock().is_empty());
}

#[tokio::test]
#[cfg(feature = "machine-host")]
async fn real_sealed_replicas_preserve_replay_and_generation_advance() {
    use crate::machine_plugins::MachinePluginStore;
    use crate::machine_protocol::{
        Platform, PortableCredentialBundle, ProviderMaterializationState,
    };
    use crate::provider_service::{ProviderAuthService, ServiceAuthenticationState};
    use base64::Engine as _;

    let root = tempfile::tempdir().unwrap();
    let service = ProviderAuthService::open(&root.path().join("service")).unwrap();
    let machine = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../../../plugins/gemini/provider.json")).unwrap();
    let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
    let bundle = PortableCredentialBundle {
        portable_schema: package.manifest.authentication.portable_schema.clone(),
        method_id: "code-assist".into(),
        values: ["settings_json", "oauth_creds_json"]
            .map(|key| {
                (
                    key.into(),
                    base64::engine::general_purpose::STANDARD.encode(b"{}"),
                )
            })
            .into_iter()
            .collect(),
    };
    let control = MachineControl::default();
    let sync = Coordinator::default();
    let (connection, mut rx) = connect(&control, "machine-1");
    service.commit(&package, &bundle, None, Some(0)).unwrap();
    // This exercises real vault sealing, signature verification and replicas.
    // An uninstalled Plugin does not decrypt/materialize the replica here;
    // installed runtime projection and native workers are separate gates.
    for (generation, advanced) in [(1, true), (1, false), (2, true), (3, true), (3, false)] {
        if generation == 2 {
            service.commit(&package, &bundle, None, Some(1)).unwrap();
        } else if generation == 3 && advanced {
            service.logout("gemini").unwrap();
        }
        let sealed = service
            .seal_for_machine("gemini", machine.encryption_public_key())
            .unwrap();
        let resealed = service
            .seal_for_machine("gemini", machine.encryption_public_key())
            .unwrap();
        assert_ne!(sealed.nonce, resealed.nonce);
        assert_ne!(sealed.ciphertext, resealed.ciphertext);
        let mut first = pin!(sync.apply(&control, &connection, sealed));
        let mut follower = pin!(sync.apply(&control, &connection, resealed));
        assert!(futures::poll!(&mut first).is_pending());
        assert!(futures::poll!(&mut follower).is_pending());
        let MachineCommand::ApplyProviderAuth {
            request_id,
            envelope,
        } = rx.try_recv().unwrap()
        else {
            panic!("unexpected command");
        };
        let receipt = machine.apply_auth(&envelope).await.unwrap();
        assert_eq!(receipt.auth_generation_advanced, advanced);
        assert_eq!(receipt.auth_generation, generation);
        assert_eq!(
            receipt.materialization_state,
            ProviderMaterializationState::NotInstalled
        );
        assert_eq!(
            envelope.action,
            if generation == 3 {
                ProviderAuthAction::Wipe
            } else {
                ProviderAuthAction::Apply
            }
        );
        reply(&control, &connection, request_id);
        assert_eq!(first.await, Ok(generation));
        assert_eq!(follower.await, Ok(generation));
        assert!(rx.try_recv().is_err());
    }
    let status = service.status("gemini").unwrap();
    assert_eq!(status.auth_generation, 3);
    assert_eq!(
        status.authentication_state,
        ServiceAuthenticationState::SignedOut
    );
}
