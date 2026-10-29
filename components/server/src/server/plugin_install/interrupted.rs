//! Reboot-convergent recovery of one installation whose Machine process died
//! while it was still staging content.
//!
//! A restarted Machine with installation-writer admission durably completes an
//! abandoned `Pending/Staging` attempt as `Unknown/Staging/Interrupted`, under
//! its journal process lock, lifecycle lock and with no live continuation
//! (`install_attempts::finish_abandoned_staging`). Activation requires a durable
//! `Activating` receipt first, so this exact receipt proves the attempt stopped
//! before the slot changed. When the same live connection also reports the
//! original installation target unchanged, the slot is provably in its
//! pre-install state and the Service fence protects nothing: the Controller
//! records the receipt and the audited staging resolution itself instead of
//! leaving every session of that Provider refused until an Operator acts.
//!
//! This path is deliberately narrower than Operator reconciliation. It accepts
//! only the Machine's `Interrupted` staging evidence (never an effect failure,
//! expiry, a Pending receipt, or any later phase), only for a slot the
//! Controller itself holds as `NeedsReconcile`, and only through the existing
//! snapshot compare-and-swap writers. It never sends `InstallPluginStep`,
//! infers anything from inventory, or rewrites the original operation.

use super::*;
use crate::machine_protocol::plugin_install::{
    InstallPhase as MachinePhase, InstallReceipt, InstallTarget, InstallUncertainty,
};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::Actor;
use crate::plugin_operation::installation::{
    InstallStagingResolutionIntent, InstallStagingResolutionPermit, InstallStagingResolutionReceipt,
};
use std::time::Duration;

/// Audit identity recorded on a resolution the Controller completed from
/// Machine-proven interruption evidence. It is an existing closed `Actor`
/// variant so older Controllers still validate the record after a rollback.
pub(super) const RECOVERY_ACCOUNT: &str = "cowboy-controller:interrupted-staging";

/// A Machine connection may come up before the original observer has given up
/// on its lost reply, so the slot can turn `NeedsReconcile` shortly after.
const ATTEMPT_DELAYS: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(300),
];

/// The two exact Machine observations this recovery needs, on one captured
/// connection.
pub(super) trait InterruptedStagingMachine: Sync {
    fn current(&self) -> bool;
    fn receipt(
        &self,
        step: &InstallStep,
    ) -> impl std::future::Future<Output = Option<InstallReceipt>> + Send;
    /// The installation target, only while installation admission is enabled.
    fn target(
        &self,
        query: &InstallTargetQuery,
    ) -> impl std::future::Future<Output = Option<InstallTarget>> + Send;
}

struct LiveMachine<'a> {
    control: &'a MachineControl,
    connection: ConnectionToken,
}

impl InterruptedStagingMachine for LiveMachine<'_> {
    fn current(&self) -> bool {
        self.control.is_current(&self.connection)
    }

    async fn receipt(&self, step: &InstallStep) -> Option<InstallReceipt> {
        match self
            .control
            .plugin_installation_step(&self.connection, step, None)
            .await
        {
            Ok(InstallObservation {
                result: InstallLookup::Found { receipt },
                ..
            }) => Some(*receipt),
            _ => None,
        }
    }

    async fn target(&self, query: &InstallTargetQuery) -> Option<InstallTarget> {
        match self
            .control
            .plugin_installation_target(&self.connection, query)
            .await
        {
            Ok(InstallTargetObservation::Observed {
                admission_enabled: true,
                target,
                ..
            }) => Some(target),
            _ => None,
        }
    }
}

fn interrupted_staging(receipt: &InstallReceipt) -> bool {
    receipt.outcome
        == (InstallOutcome::Unknown {
            phase: MachinePhase::Staging,
            reason: InstallUncertainty::Interrupted,
        })
}

/// Run on every authenticated Machine connection. Each attempt only reads the
/// in-memory fence map unless one of this Machine's slots needs reconciliation.
pub(in crate::server) async fn recover_interrupted_staging(
    state: Arc<AppState>,
    machine_id: String,
    connection: ConnectionToken,
) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    for delay in ATTEMPT_DELAYS {
        tokio::time::sleep(delay).await;
        if !state.machine_control.is_current(&connection) {
            return;
        }
        let plugins: Vec<String> = state
            .plugin_lifecycle_fences
            .read()
            .iter()
            .filter(|((machine, _), fence)| {
                machine == &machine_id && **fence == PluginFenceState::NeedsReconcile
            })
            .map(|((_, plugin), _)| plugin.clone())
            .collect();
        for plugin in plugins {
            let machine = LiveMachine {
                control: &state.machine_control,
                connection: connection.clone(),
            };
            match recover_slot(
                store,
                &state.plugin_lifecycle_fences,
                &state.service_id,
                &machine_id,
                &plugin,
                &machine,
            )
            .await
            {
                Ok(Some(resolution)) => tracing::info!(
                    machine = %machine_id,
                    %plugin,
                    operation = %resolution.intent.operation_id,
                    resolution = %resolution.intent.resolution_id,
                    "resolved an installation the Machine proved was interrupted while staging"
                ),
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    machine = %machine_id,
                    %plugin,
                    %error,
                    "interrupted installation staging was not resolved; the slot remains fenced"
                ),
            }
        }
    }
}

/// `Ok(None)` when this slot is not exactly an interrupted staging attempt
/// (or something else owns it); the fence is then left as it was.
pub(super) async fn recover_slot(
    store: &Store,
    fences: &PluginLifecycleFences,
    service: &str,
    machine_id: &str,
    plugin: &str,
    machine: &impl InterruptedStagingMachine,
) -> anyhow::Result<Option<InstallStagingResolutionReceipt>> {
    let received = TimeSample::now();
    let mut unresolved = Vec::new();
    for operation in store
        .plugin_install_history(service, machine_id, plugin)
        .await?
    {
        if reconciliation_candidate(&operation)
            && operation.attention_from == Some(InstallPhase::Installing)
            && store
                .plugin_install_staging_resolution(&operation.intent.operation_id)
                .await?
                .is_none()
        {
            unresolved.push(operation.intent.operation_id);
        }
    }
    // Exactly one unresolved attempt can own a slot; anything else is left to
    // an Operator rather than guessed at.
    let [operation_id] = unresolved.as_slice() else {
        return Ok(None);
    };
    // Same exclusion as Operator reconciliation: a live install, uninstall or
    // concurrent reconciliation holds a different fence state. Dropping this
    // guard on any early return restores `NeedsReconcile`.
    let Ok(mut fence) = InstallationReconciliationFence::acquire(
        fences,
        (machine_id.to_owned(), plugin.to_owned()),
    ) else {
        return Ok(None);
    };
    // Re-read the exact snapshot under the fence.
    let Some(mut before) = store.plugin_install_operation(operation_id).await? else {
        return Ok(None);
    };
    if before.intent.service_id != service
        || before.intent.machine_id != machine_id
        || before.intent.plugin_id != plugin
        || !reconciliation_candidate(&before)
        || before.attention_from != Some(InstallPhase::Installing)
        || store
            .plugin_install_staging_resolution(operation_id)
            .await?
            .is_some()
    {
        return Ok(None);
    }
    let step = before.intent.machine_step()?;
    let Some(receipt) = machine.receipt(&step).await else {
        return Ok(None);
    };
    if !machine.current() || !interrupted_staging(&receipt) || !receipt.matches(&step) {
        return Ok(None);
    }
    if before.machine_receipt.as_ref() != Some(&receipt) {
        // Recording exact Machine evidence needs no effect authority; the
        // store compares the complete snapshot and refuses a later phase.
        before = commit_reconciled_receipt(store, &before, &receipt).await?;
    }
    let target = machine.target(&step.target_query()).await;
    if !machine.current() || target.is_none() || target != before.intent.machine_target {
        return Ok(None);
    }
    let target = target.expect("checked above");
    let intent = InstallStagingResolutionIntent {
        schema: 1,
        resolution_id: format!(
            "install-staging-{}",
            crate::product_auth::new_session_token()?
        ),
        operation_id: before.intent.operation_id.clone(),
        service_id: service.to_owned(),
        actor: Actor::Admin {
            account: RECOVERY_ACCOUNT.to_owned(),
        },
        machine_id: machine_id.to_owned(),
        plugin_id: plugin.to_owned(),
        operation_digest: crate::machine_protocol::plugin_step::digest(&serde_json::to_vec(
            &before,
        )?),
        observed_target: target.clone(),
    };
    ensure!(
        intent.matches(&before, &target)?,
        "interrupted staging evidence no longer matches the installation"
    );
    let budget = OperationBudget::new(
        received.deadline_ms(Duration::from_mins(1)),
        Duration::from_mins(1),
        received,
    );
    let permit = InstallStagingResolutionPermit::new(intent, budget);
    let resolution = match store.resolve_plugin_install_staging(&permit, &before).await {
        Ok(resolution) => resolution,
        // A COMMIT response can be lost; accept only this exact resolution.
        Err(error) => match store
            .plugin_install_staging_resolution(operation_id)
            .await?
        {
            Some(saved) if saved.intent == *permit.intent() => saved,
            _ => return Err(error),
        },
    };
    let previous = matches!(
        before.intent.machine_target,
        Some(InstallTarget::Removed { .. })
    )
    .then_some(PluginFenceState::Uninstalled);
    fence.finish(previous);
    Ok(Some(resolution))
}
