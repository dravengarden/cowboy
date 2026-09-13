//! Explicit startup policy feeds the existing bounded queue. No timer, replay,
//! automatic activation from history, retry, or legacy fallback lives here.

use super::export::ExportScope;
use crate::machine_control::MachineControl;
use crate::machine_protocol::telemetry_export::ExportOutcome;
use crate::observability::{ExportReceipt, TelemetryExporter};
use crate::otlp::Signal;
use crate::plugin_catalog::PluginCatalog;
use crate::server::PluginLifecycleFences;
use crate::store::Store;
use crate::telemetry_plugin::background_policy::BackgroundPolicy;
use std::sync::Arc;

pub(in crate::server) fn exporter(
    policy: Arc<BackgroundPolicy>,
    store: Store,
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: PluginLifecycleFences,
) -> TelemetryExporter {
    Arc::new(move |batch| {
        let (policy, store, control, catalog, fences) = (
            policy.clone(),
            store.clone(),
            control.clone(),
            catalog.clone(),
            fences.clone(),
        );
        Box::pin(async move {
            let Some(payload) = batch.otlp else {
                // Managed mode supports only standard OTLP. Never invoke a
                // legacy host port, even for an older client's JSONL batch.
                return ExportReceipt::default();
            };
            let signal = payload.signal;
            if !policy.allows(signal) {
                // Intentionally disabled signal: no network admission, not a
                // delivery failure. Same accounting as a private disabled lane.
                return receipt(signal, true, 0);
            }
            let Ok((attempt, permit)) = policy.admit(payload) else {
                return ExportReceipt::default();
            };
            let Ok(scope) = ExportScope::background(attempt, permit, control, catalog, fences)
            else {
                return ExportReceipt::default();
            };
            let Ok(result) = scope.execute_background(&store).await else {
                return ExportReceipt::default();
            };
            match result.outcome {
                ExportOutcome::Delivered {} | ExportOutcome::Disabled {} => {
                    receipt(signal, true, 0)
                }
                ExportOutcome::Partial { rejected_items } => receipt(signal, true, rejected_items),
                ExportOutcome::NotAdmitted {} | ExportOutcome::Unknown {} => {
                    ExportReceipt::default()
                }
            }
        })
    })
}

fn receipt(signal: Signal, success: bool, rejected_items: u64) -> ExportReceipt {
    ExportReceipt {
        logs_delivered: signal == Signal::Logs && success,
        metrics_delivered: signal == Signal::Metrics && success,
        traces_delivered: signal == Signal::Traces && success,
        rejected_items,
    }
}
