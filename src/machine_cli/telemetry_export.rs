//! Capture the one-shot execution lease before detached scheduling.
use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportOutcome, ExportReceipt};
use std::sync::Arc;

pub(crate) fn export(
    request_id: String,
    attempt: ExportAttempt,
    store: Arc<MachinePluginStore>,
    execution: &PluginExecutionScope,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    match execution.telemetry_export(&attempt) {
        Ok(invocation) => {
            tokio::spawn(async move {
                let receipt = store.export_bound_telemetry(invocation).await;
                let _ = events.send(MachineEvent::TelemetryExported {
                    request_id,
                    receipt: Some(Box::new(receipt)),
                });
            });
        }
        Err(_) => {
            let receipt = attempt.request_digest().ok().map(|request_digest| {
                Box::new(ExportReceipt {
                    request_digest,
                    outcome: ExportOutcome::NotAdmitted {},
                })
            });
            let _ = events.send(MachineEvent::TelemetryExported {
                request_id,
                receipt,
            });
        }
    }
}
