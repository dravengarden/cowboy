//! The Machine connection handshake: challenge, signed hello verification,
//! encryption key binding, protocol negotiation and Welcome.

use super::*;

/// The results of an accepted Machine handshake that the connection needs.
pub(super) struct MachineHandshake {
    pub(super) store: Store,
    pub(super) hello: crate::machine_protocol::MachineHello,
    pub(super) challenge_id: String,
    pub(super) protocol: u16,
    pub(super) connection_mode: &'static str,
}

/// Challenge a connecting Machine and verify its signed hello against the
/// enrolled key. Returns the verified hello with its challenge id.
pub(super) async fn verified_machine_hello(
    socket: &mut WebSocket,
    store: &Store,
) -> Option<(crate::machine_protocol::MachineHello, String)> {
    let challenge_id = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "creating Machine challenge");
            return None;
        }
    };
    let nonce = match random_machine_token() {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "creating Machine nonce");
            return None;
        }
    };
    let expires_at_ms = now_ms().saturating_add(MACHINE_HANDSHAKE_TIMEOUT.as_millis() as i64);
    let challenge = crate::machine_protocol::MachineFrame::Challenge {
        challenge_id: challenge_id.clone(),
        nonce: nonce.clone(),
        expires_at_ms,
        proof_version: 3,
    };
    if send_json(socket, &challenge).await.is_err() {
        return None;
    }
    let incoming = match tokio::time::timeout(MACHINE_HANDSHAKE_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(_))) => {
            tracing::warn!("Machine handshake received a non-text hello");
            return None;
        }
        Ok(Some(Err(error))) => {
            tracing::warn!(%error, "reading Machine hello");
            return None;
        }
        Ok(None) => {
            tracing::warn!("Machine disconnected before sending hello");
            return None;
        }
        Err(_) => {
            tracing::warn!("Machine hello timed out while collecting host inventory");
            return None;
        }
    };
    let crate::machine_protocol::MachineFrame::Hello { hello } =
        (match serde_json::from_str(&incoming) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(%error, "invalid Machine hello");
                return None;
            }
        })
    else {
        return None;
    };
    if hello.challenge_id.as_deref() != Some(&challenge_id) {
        return None;
    }
    let signature = hello.challenge_signature.as_deref()?;
    let public_key = match store.machine_public_key(&hello.machine_id).await {
        Ok(Some(value)) => value,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, machine = %hello.machine_id, "loading Machine identity");
            return None;
        }
    };
    if now_ms() > expires_at_ms {
        return None;
    }
    let proof = if hello.max_protocol
        >= crate::machine_protocol::PLUGIN_HOST_EXECUTION_PROTOCOL_VERSION
    {
        crate::machine_protocol::challenge_proof_v3(&challenge_id, &nonce, expires_at_ms, &hello)
    } else if hello.max_protocol >= 3 {
        crate::machine_protocol::challenge_proof_v2(&challenge_id, &nonce, expires_at_ms, &hello)
    } else {
        crate::machine_protocol::challenge_proof_v1(&challenge_id, &nonce, expires_at_ms, &hello)
    };
    let signature = signature.to_owned();
    let verified = tokio::task::spawn_blocking(move || {
        crate::machine_auth::verify(&public_key, &proof, &signature)
    })
    .await;
    if !matches!(verified, Ok(Ok(true))) {
        tracing::warn!(machine = %hello.machine_id, "Machine challenge verification failed");
        return None;
    }
    Some((hello, challenge_id))
}

/// Challenge, verify and welcome a connecting Machine: issue a nonce, check
/// the signed hello against the enrolled key, bind its encryption key,
/// negotiate the protocol, record the connection and send Welcome. Returns
/// None after any refusal or failure, having done the same cleanup as before.
pub(super) async fn machine_handshake(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
) -> Option<MachineHandshake> {
    let Some(store) = state.store.as_ref().cloned() else {
        let _ = send_json(
            socket,
            &crate::machine_protocol::MachineFrame::Reject {
                reason: "Machine connections require persistence".to_owned(),
            },
        )
        .await;
        return None;
    };
    let (hello, challenge_id) = verified_machine_hello(socket, &store).await?;
    if hello.max_protocol >= 3 {
        let Some(encryption_public_key) = hello.encryption_public_key.as_deref() else {
            tracing::warn!(machine = %hello.machine_id, "protocol-three Machine omitted encryption key");
            return None;
        };
        if let Err(error) = store
            .bind_machine_encryption_public_key(&hello.machine_id, encryption_public_key)
            .await
        {
            tracing::warn!(%error, machine = %hello.machine_id, "Machine encryption key rejected");
            return None;
        }
    }
    let Some(protocol) = crate::machine_protocol::negotiate(
        crate::machine_protocol::MIN_MACHINE_PROTOCOL_VERSION,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        hello.min_protocol,
        hello.max_protocol,
    ) else {
        let _ = send_json(
            socket,
            &crate::machine_protocol::MachineFrame::Reject {
                reason: "no compatible Machine protocol".to_owned(),
            },
        )
        .await;
        return None;
    };
    let platform = match hello.platform {
        crate::machine_protocol::Platform::Linux => "linux",
        crate::machine_protocol::Platform::Macos => "macos",
    };
    let connection_mode = match hello.connection_mode {
        crate::machine_protocol::ConnectionMode::LocalUds => "local",
        crate::machine_protocol::ConnectionMode::OutboundTls => "outbound_wss",
    };
    let inventory = serde_json::json!({
        "components": &hello.components,
        "plugins": &hello.plugins,
        "provider_contracts": &hello.provider_contracts,
        "plugin_contracts": &hello.plugin_contracts,
        "workspaces": &hello.workspaces,
        "workspace_revision": &hello.workspace_revision,
        "workspace_owner": &hello.workspace_owner,
        "capacity": &hello.capacity,
    });
    if let Err(error) = store
        .machine_connected(
            &hello.machine_id,
            &challenge_id,
            platform,
            &hello.arch,
            connection_mode,
            &inventory,
        )
        .await
    {
        tracing::error!(%error, machine = %hello.machine_id, "recording Machine connection");
        return None;
    }
    if send_json(
        socket,
        &crate::machine_protocol::MachineFrame::Welcome {
            protocol,
            controller_epoch: 0,
            heartbeat_interval_ms: MACHINE_HEARTBEAT_MS,
            desired_components: state.desired_machine_components.current().automatic(),
        },
    )
    .await
    .is_err()
    {
        let _ = store
            .machine_disconnected(
                &hello.machine_id,
                &challenge_id,
                MACHINE_RECONNECT_GRACE_SECONDS,
            )
            .await;
        state.machine_snapshots.publish().await;
        return None;
    }
    Some(MachineHandshake {
        store,
        hello,
        challenge_id,
        protocol,
        connection_mode,
    })
}

#[cfg(test)]
#[test]
pub(super) fn runtime_reconciliation_covers_machine_retry_and_snapshot_settlement() {
    assert_eq!(
        RUNTIME_RECONCILIATION_GRACE,
        std::time::Duration::from_secs(45)
    );
    assert!(
        RUNTIME_RECONCILIATION_GRACE
            > std::time::Duration::from_secs(CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS)
    );
}
