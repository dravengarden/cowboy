//! One connection-bound RPC. No automatic retry, old-protocol downgrade, or
//! result-history retention of telemetry payloads.
use super::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineCommand, MachineControl, Reply,
    ReplyKind, RequestBinding,
};
use crate::machine_protocol::telemetry_export::{ATTEMPT_BUDGET, ExportAttempt, ExportReceipt};

impl MachineControl {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn telemetry_export_target_current(
        &self,
        token: &ConnectionToken,
        attempt: &ExportAttempt,
    ) -> bool {
        let live = self.live.read();
        attempt.validate().is_ok()
            && attempt.machine_id == token.0.machine_id
            && live.connections.get(&token.0.machine_id).is_some_and(|c| {
                c.token.same(token)
                    && c.protocol
                        >= crate::machine_protocol::TELEMETRY_BOUND_EXPORT_PROTOCOL_VERSION
            })
            && attempt.binding.selection.as_ref().is_some_and(|target| {
                live.telemetry_installation_matches(&token.0.machine_id, target)
            })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn export_bound_telemetry(
        &self,
        token: &ConnectionToken,
        attempt: &ExportAttempt,
    ) -> Result<ExportReceipt, CommandRequestError> {
        let fail = |certainty, detail: &str| CommandRequestError {
            certainty,
            detail: detail.into(),
        };
        if attempt.validate().is_err() || attempt.machine_id != token.0.machine_id {
            return Err(fail(CommandFailure::NotSent, "invalid managed export"));
        }
        let request_id = self
            .request_id("telemetry-export")
            .map_err(|_| fail(CommandFailure::NotSent, "export identity unavailable"))?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::ExportBoundTelemetry {
                    request_id: request_id.clone(),
                    attempt: Box::new(attempt.clone()),
                },
                ReplyKind::TelemetryExport,
                Some(RequestBinding::TelemetryExport(token, attempt)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "managed export channel or target unavailable",
                )
            })?;
        match tokio::time::timeout(ATTEMPT_BUDGET, rx).await {
            Ok(Ok(Reply::TelemetryExport(Some(receipt)))) if receipt.matches(attempt) => {
                Ok(*receipt)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "managed export receipt unavailable",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::telemetry_export::{ExportOutcome, fixture};
    use crate::machine_protocol::{MachineEvent, PluginInstallationState, PluginInventory};
    use tokio::sync::mpsc;

    fn connect(
        control: &MachineControl,
        protocol: u16,
    ) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let token = control.install(
            "machine-test".into(),
            "same-epoch".into(),
            false,
            protocol,
            tx,
        );
        let target = fixture().binding.selection.unwrap();
        control.record_remote(
            &token,
            MachineEvent::PluginInventory {
                plugins: vec![PluginInventory {
                    plugin_id: target.plugin_id,
                    plugin_version: target.plugin_version,
                    generation_digest: target.generation_digest.into(),
                    contract_fingerprint: target.contract_fingerprint.into(),
                    installation_revision: Some(target.installation_revision),
                    plugin_kind: cowboy_plugin_sdk::PluginKind::TelemetryBackend,
                    state: PluginInstallationState::Active,
                    rollback_generation_digest: None,
                    active_session_leases: 0,
                    auth_generation: None,
                    replica_state: crate::machine_protocol::ProviderReplicaState::Current,
                    materialization_state:
                        crate::machine_protocol::ProviderMaterializationState::Current,
                    detail: None,
                }],
                observed_at_ms: 1,
            },
        );
        (token, rx)
    }

    #[tokio::test]
    async fn managed_export_never_downgrades_or_retargets_old_connections() {
        for protocol in 1..16 {
            let control = MachineControl::default();
            let (token, mut commands) = connect(&control, protocol);
            assert!(!control.telemetry_export_target_current(&token, &fixture()));
            assert_eq!(
                control
                    .export_bound_telemetry(&token, &fixture())
                    .await
                    .unwrap_err()
                    .certainty,
                CommandFailure::NotSent
            );
            assert!(commands.try_recv().is_err());
        }
        let control = MachineControl::default();
        let (token, mut commands) = connect(&control, 16);
        let request = fixture();
        let send = control.export_bound_telemetry(&token, &request);
        let replace = async {
            let MachineCommand::ExportBoundTelemetry { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!()
            };
            let (_new, mut incoming) = connect(&control, 16);
            control.record_remote(
                &token,
                MachineEvent::TelemetryExported {
                    request_id,
                    receipt: Some(Box::new(ExportReceipt {
                        request_digest: request.request_digest().unwrap(),
                        outcome: ExportOutcome::Delivered {},
                    })),
                },
            );
            assert!(
                incoming.try_recv().is_err(),
                "a missing reply never resends to the replacement"
            );
        };
        let (result, ()) = tokio::join!(send, replace);
        assert_eq!(result.unwrap_err().certainty, CommandFailure::Unknown);
        assert!(control.live.read().pending.is_empty());
        assert_eq!(
            control
                .export_bound_telemetry(&token, &request)
                .await
                .unwrap_err()
                .certainty,
            CommandFailure::NotSent
        );
    }

    #[tokio::test]
    async fn managed_export_accepts_only_its_exact_typed_receipt_and_hides_late_results() {
        for forged in [false, true] {
            let control = MachineControl::default();
            let (token, mut commands) = connect(&control, 16);
            let request = fixture();
            let send = control.export_bound_telemetry(&token, &request);
            let respond = async {
                let MachineCommand::ExportBoundTelemetry {
                    request_id,
                    attempt,
                } = commands.recv().await.unwrap()
                else {
                    panic!()
                };
                assert_eq!(*attempt, request);
                control.record_remote(
                    &token,
                    MachineEvent::CommandResult {
                        request_id: request_id.clone(),
                        accepted: true,
                        detail: None,
                    },
                );
                assert_eq!(control.live.read().pending.len(), 1);
                let receipt = ExportReceipt {
                    request_digest: if forged {
                        crate::machine_protocol::telemetry_binding::binding_digest(
                            b"different payload",
                        )
                    } else {
                        request.request_digest().unwrap()
                    },
                    outcome: ExportOutcome::Delivered {},
                };
                for _ in 0..2 {
                    control.record_remote(
                        &token,
                        MachineEvent::TelemetryExported {
                            request_id: request_id.clone(),
                            receipt: Some(Box::new(receipt.clone())),
                        },
                    );
                }
            };
            let (result, ()) = tokio::join!(send, respond);
            assert_eq!(result.is_ok(), !forged);
            assert!(control.live.read().pending.is_empty());
            assert!(
                !control
                    .events("machine-test")
                    .iter()
                    .any(|e| matches!(e, MachineEvent::TelemetryExported { .. }))
            );
        }
    }
}
