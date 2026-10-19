//! Cancel a durable native login after its in-memory executor was lost.
use super::*;
use crate::store::provider_accounts::logins::Receipt;

pub(super) async fn recover(
    store: &Store,
    control: &MachineControl,
    receipt: &Receipt,
) -> Result<bool, &'static str> {
    if receipt.provider_id != "claude-code"
        || receipt.native_generation.is_none()
        || receipt.input.account_id.is_default()
        || receipt.never_dispatched()
    {
        return Ok(false);
    }
    if !receipt.cancel_requested {
        return Err("cancellation_not_requested");
    }
    let fingerprint = receipt
        .dispatch_recipient_fingerprint
        .as_deref()
        .ok_or("authentication_cleanup_required")?;
    check_enrollment(store, &receipt.input.machine_id, fingerprint).await?;
    let (connection, protocol) = control
        .connection_protocol(&receipt.input.machine_id)
        .ok_or("authentication_cleanup_required")?;
    if protocol < crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION {
        return Err("authentication_cleanup_required");
    }
    let correlation = format!("login-cancel-{}", uuid::Uuid::new_v4());
    control
        .command_on_connection(
            &connection,
            correlation.clone(),
            crate::machine_protocol::MachineCommand::CancelLoginRequest {
                request_id: correlation,
                login_request_id: receipt.request_id.clone(),
            },
        )
        .await
        .map_err(|_| "authentication_cleanup_required")?;
    check_enrollment(store, &receipt.input.machine_id, fingerprint).await?;
    if !control.is_current(&connection) {
        return Err("authentication_connection_changed");
    }
    // Correlated acceptance only. No profile deletion, logout, new dispatch,
    // completion receipt or process-exit evidence is produced here.
    Ok(true)
}

async fn check_enrollment(
    store: &Store,
    machine: &str,
    fingerprint: &str,
) -> Result<(), &'static str> {
    let key = store
        .machine_encryption_public_key(machine)
        .await
        .map_err(|_| "unavailable")?;
    if !key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == fingerprint) {
        return Err("machine_enrollment_changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::{
        MachineCommand, MachineEvent, PROVIDER_ACCOUNT_PROTOCOL_VERSION,
    };

    #[tokio::test]
    async fn durable_cancel_without_executor_is_correlated_and_enrollment_bound() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(
            &format!(
                "sqlite://{}?mode=rwc",
                dir.path().join("cancel.sqlite").display()
            ),
            dir.path().join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let key = base64::engine::general_purpose::STANDARD.encode(
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([7; 32])).as_bytes(),
        );
        let enrollment = store
            .create_machine_enrollment("native", "Native", 60)
            .await
            .unwrap();
        store
            .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
            .await
            .unwrap();
        let receipt: Receipt = serde_json::from_value(json!({
            "request_id":"original-login", "provider_id":"claude-code", "cancel_requested":true,
            "native_generation":1, "dispatch_registry_revision":1,
            "dispatch_recipient_fingerprint":crate::admin::hex_sha256(key.as_bytes()),
            "input":{"account_id":"11111111-1111-4111-8111-111111111111", "machine_id":"native", "provider_version":"1", "generation_digest":"digest", "method":"native"}
        })).unwrap();
        let control = MachineControl::default();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install(
            "native".into(),
            "epoch".into(),
            false,
            PROVIDER_ACCOUNT_PROTOCOL_VERSION,
            tx,
        );
        let mut bad = receipt.clone();
        bad.dispatch_recipient_fingerprint = Some("old-enrollment".into());
        assert_eq!(
            recover(&store, &control, &bad).await,
            Err("machine_enrollment_changed")
        );
        assert!(rx.try_recv().is_err());
        for accepted in [false, true] {
            let (result, ()) = tokio::join!(recover(&store, &control, &receipt), async {
                let MachineCommand::CancelLoginRequest {
                    request_id,
                    login_request_id,
                } = rx.recv().await.unwrap()
                else {
                    panic!("cancel expected");
                };
                assert_eq!(login_request_id, receipt.request_id);
                assert_ne!(request_id, login_request_id);
                control.record_remote(
                    &connection,
                    MachineEvent::CommandResult {
                        request_id,
                        accepted,
                        detail: None,
                    },
                );
            });
            assert_eq!(result.is_ok(), accepted);
        }
        store.revoke_machine("native").await.unwrap();
        assert_eq!(
            recover(&store, &control, &receipt).await,
            Err("machine_enrollment_changed")
        );
        assert!(rx.try_recv().is_err());
    }
}
