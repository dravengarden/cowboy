//! Persistence and least-privilege diagnostics for the core installer.

use super::*;
use crate::plugin_operation::Actor;
use crate::plugin_operation::installation::InstallOperation;

pub(super) struct Progress<'a> {
    store: &'a Store,
    intent: &'a InstallIntent,
    phase: InstallPhase,
}

impl<'a> Progress<'a> {
    pub(super) fn machine_step(&self) -> anyhow::Result<InstallStep> {
        self.intent.machine_step()
    }

    pub(super) async fn machine_receipt(
        &mut self,
        receipt: &crate::machine_protocol::plugin_install::InstallReceipt,
    ) -> anyhow::Result<()> {
        let saved = self
            .store
            .record_plugin_install_receipt(self.intent, receipt)
            .await?;
        self.phase = saved.phase;
        Ok(())
    }

    pub(super) fn new(store: &'a Store, intent: &'a InstallIntent) -> Self {
        Self {
            store,
            intent,
            phase: InstallPhase::Prepared,
        }
    }

    pub(super) async fn advance(
        &mut self,
        to: InstallPhase,
        problem: Option<InstallProblem>,
    ) -> anyhow::Result<()> {
        self.store
            .advance_plugin_install(self.intent, self.phase, to, problem)
            .await?;
        self.phase = to;
        Ok(())
    }

    pub(super) async fn abort(
        &mut self,
        fence: &mut InstallationFence,
        problem: InstallProblem,
    ) -> anyhow::Result<Outcome> {
        self.advance(InstallPhase::Aborted, Some(problem)).await?;
        fence.disposition = Disposition::Previous;
        Ok(Outcome::NotDispatched)
    }

    pub(super) async fn storage_failure(&self) {
        // A lost COMMIT response can have advanced further than `self.phase`.
        // Observe the same intent and record uncertainty only; never repeat
        // its effect or overwrite a terminal/previously uncertain result.
        if let Ok(Some(op)) = self
            .store
            .plugin_install_operation(&self.intent.operation_id)
            .await
            && op.intent == *self.intent
            && !op.phase.terminal()
            && op.phase != InstallPhase::NeedsAttention
        {
            let _ = self
                .store
                .advance_plugin_install(
                    self.intent,
                    op.phase,
                    InstallPhase::NeedsAttention,
                    Some(InstallProblem::StorageFailure),
                )
                .await;
        }
    }
}

#[derive(Serialize)]
pub(super) struct Evidence<'a> {
    evidence_schema: u16,
    // Deliberately project only the closed outcome. The complete receipt's
    // actor-bound plan, target and deadline remain private durable evidence.
    machine_receipt: Option<&'a InstallOutcome>,
    operation_id: &'a str,
    phase: InstallPhase,
    problem: Option<InstallProblem>,
    attention_from: Option<InstallPhase>,
    plugin_kind: cowboy_plugin_sdk::PluginKind,
    plugin_version: &'a str,
    generation_digest: &'a str,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl<'a> From<&'a InstallOperation> for Evidence<'a> {
    fn from(op: &'a InstallOperation) -> Self {
        Self {
            evidence_schema: op.intent.schema,
            machine_receipt: op.machine_receipt.as_ref().map(|receipt| &receipt.outcome),
            operation_id: &op.intent.operation_id,
            phase: op.phase,
            problem: op.problem,
            attention_from: op.attention_from,
            plugin_kind: op.intent.plugin_kind,
            plugin_version: &op.intent.plugin_version,
            generation_digest: &op.intent.generation_digest,
            created_at_ms: op.created_at_ms,
            updated_at_ms: op.updated_at_ms,
        }
    }
}

pub(super) fn duplicate_response(
    op: &InstallOperation,
    service: &str,
    actor: &Actor,
    machine: &str,
    plugin: &str,
    request: &PluginInstallRequest,
) -> Response {
    if op.intent.service_id != service
        || &op.intent.actor != actor
        || op.intent.machine_id != machine
        || op.intent.plugin_id != plugin
        || op.intent.plugin_version != request.version
        || op.intent.generation_digest != request.digest
        || op.intent.operation_id != request.operation_id
    {
        return (
            StatusCode::CONFLICT,
            "Plugin operation identity already has different inputs",
        )
            .into_response();
    }
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "detail": "This Plugin operation already exists. No installation was repeated.",
            "execution_authorized": false,
            "operation": Evidence::from(op),
        })),
    )
        .into_response()
}

pub(in crate::server) async fn api_machine_plugin_install_operations(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store.plugin_install_history(&state.service_id, &machine, &plugin).await {
        Ok(operations) => Json(serde_json::json!({
            "schema": "dravengarden.cowboy.plugin-install-history/v2",
            "admission_enabled": DURABLE_INSTALL_ENABLED,
            "execution_authorized": false,
            "requires_reconciliation": state.plugin_lifecycle_fences.read().get(&(machine, plugin)) == Some(&PluginFenceState::NeedsReconcile),
            "operations": operations.iter().map(Evidence::from).collect::<Vec<_>>(),
        })).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Plugin installation evidence is unavailable").into_response(),
    }
}
