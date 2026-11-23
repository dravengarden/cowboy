use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::{MachineCommand, MachineEvent, ProviderAuthAction};
use crate::server::provider_auth_sync::Coordinator;
use std::pin::pin;
use tokio::sync::mpsc;

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
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/grok/provider.json"
        )))
        .unwrap();
    let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
    let bundle = PortableCredentialBundle {
        portable_schema: package.manifest.authentication.portable_schema.clone(),
        method_id: "xai-account".into(),
        values: ["auth_json"]
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
            service.logout("grok").unwrap();
        }
        let sealed = service
            .seal_for_machine("grok", machine.encryption_public_key())
            .unwrap();
        let resealed = service
            .seal_for_machine("grok", machine.encryption_public_key())
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
    let status = service.status("grok").unwrap();
    assert_eq!(status.auth_generation, 3);
    assert_eq!(
        status.authentication_state,
        ServiceAuthenticationState::SignedOut
    );
}
