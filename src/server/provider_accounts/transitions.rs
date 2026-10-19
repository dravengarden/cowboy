//! Bounded migration stop reconciliation. Public migration remains gated.
#![allow(dead_code)]
use crate::{
    provider_account::WorkerExitRequest, remote_runtime::account_stop::Outcome,
    store::provider_accounts::revocation::stops::Evidence,
};
use anyhow::{Context as _, Result, ensure};
use std::sync::Arc;

/// Reserve immutable intent only. The Store rechecks the owner, exact original
/// session binding and current target login under its transaction locks.
pub(in crate::server) async fn plan(
    state: &super::super::AppState,
    owner: &str,
    operation: &str,
    request: crate::store::provider_accounts::transitions::Request,
) -> Result<(), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if let Some(old) = store
        .native_session_transition(owner, operation)
        .await
        .map_err(|_| "unavailable")?
    {
        return if old.request == request {
            Ok(())
        } else {
            Err("operation_reused")
        };
    }
    let target = store
        .provider_login_receipt(owner, &request.target_login_operation)
        .await
        .map_err(|_| "unavailable")?
        .and_then(|receipt| receipt.native_completion)
        .ok_or("native_completion_required")?;
    let intent = crate::store::provider_accounts::transitions::Intent { request, target };
    store
        .plan_native_session_transition(owner, operation, &intent)
        .await
        .map_err(|_| "transition_not_admitted")
}

pub(in crate::server) async fn status(
    state: &super::super::AppState,
    owner: &str,
    operation: &str,
) -> Result<serde_json::Value, &'static str> {
    let stage = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .native_session_transition_stage(owner, operation)
        .await
        .map_err(|_| "unavailable")?;
    Ok(
        serde_json::json!({"stage":stage.unwrap_or_else(|| "unknown".into()),
        "evidence":"historical","worker_state":"not_observed"}),
    )
}

/// A bounded retry of the SAME durable intent. A timeout never proves that a
/// queued command failed or that the worker stopped; status remains queryable.
pub(in crate::server) async fn reconcile(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<serde_json::Value, &'static str> {
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        complete(state, owner, operation),
    )
    .await;
    let mut value = status(state, owner, operation).await?;
    value["reconciliation"] = serde_json::json!(match outcome {
        Ok(Ok(true)) => "completed",
        Ok(Err(_)) => "not_admitted",
        Ok(Ok(false)) | Err(_) => "unconfirmed",
    });
    Ok(value)
}

/// Resume the migration through Broker admission. True is durable migration
/// completion only; this operation does not start a worker.
pub(super) async fn complete(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<bool> {
    use crate::remote_runtime::account_stop::ReadmissionOutcome;
    let store = state.store.as_ref().context("store_unavailable")?;
    if store
        .native_session_transition_finished(owner, operation)
        .await?
    {
        return Ok(true);
    }
    if !project(state, owner, operation).await? {
        return Ok(false);
    }
    let intent = store
        .native_session_transition(owner, operation)
        .await?
        .context("transition_not_found")?;
    let session = state
        .supervisor
        .readmission_session(&intent.request.session_id, &intent.target.profile)
        .map_err(anyhow::Error::msg)?;
    let stopped = store
        .native_session_transition_stop(owner, operation)
        .await?
        .and_then(|evidence| evidence.tree_exit)
        .context("transition_stop_missing")?;
    let admission = crate::provider_account::PreparedBinding {
        operation: operation.into(),
        stopped,
        target: intent.target.profile.clone(),
    };
    let machine = &intent.request.machine_id;
    let runtime = state
        .runtime_router
        .runtime(machine)
        .context("machine_offline")?;
    let epoch = runtime
        .account_stop_connection()
        .context("runtime_offline")?;
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(machine)
        .context("machine_offline")?;
    ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "account_protocol_unavailable"
    );
    let current = || {
        ensure!(
            runtime.account_stop_connection() == Some(epoch)
                && state
                    .runtime_router
                    .runtime(machine)
                    .is_some_and(|active| Arc::ptr_eq(&active, &runtime))
                && state.machine_control.is_current(&connection)
                && runtime.account_stop_recipient().as_ref()
                    == Some(&intent.target.recipient_fingerprint),
            "machine_connection_changed"
        );
        let meta = state
            .hub
            .session_info(&intent.request.session_id)
            .context("session_unavailable")?
            .meta;
        ensure!(
            !meta.closing
                && meta.status == crate::core::Status::Exited
                && meta.owner_user_id.as_deref() == Some(owner)
                && meta.machine_id == *machine
                && meta.provider_account_id == admission.target.account_id
                && meta.provider == admission.target.provider_id
                && meta.provider_version == admission.target.provider_version
                && meta.provider_generation_digest == admission.target.generation_digest
                && meta.provider_auth_generation == Some(admission.target.generation)
                && meta.agent_session_id.is_none()
                && meta.cwd == session.cwd
                && meta.execution_binding == session.execution_binding
                && meta.system == session.system
                && meta.provider_behavior == session.provider_behavior,
            "native_session_binding_changed"
        );
        Ok(())
    };
    let pending = store
        .with_native_session_binding(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            &intent.request.session_id,
            || {
                std::future::ready((|| {
                    current()?;
                    runtime.readmit_account_session(session.clone(), admission.clone())
                })())
            },
        )
        .await?;
    match pending.wait(std::time::Duration::from_secs(30)).await {
        ReadmissionOutcome::Admitted => {}
        ReadmissionOutcome::Refused(reason) => {
            anyhow::bail!("migration_readmission_refused: {reason}")
        }
        ReadmissionOutcome::Unconfirmed => return Ok(false),
    }
    store
        .finish_native_session_transition(owner, operation, &admission, current)
        .await?;
    Ok(true)
}

/// Resume after a DB commit (including after Controller restart). A receipt
/// alone is insufficient: recheck current native and stored session admission.
pub(super) async fn project(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<bool> {
    if !commit(state, owner, operation).await? {
        return Ok(false);
    }
    let store = state.store.as_ref().context("store_unavailable")?;
    let intent = store
        .native_session_transition(owner, operation)
        .await?
        .context("transition_not_found")?;
    let binding = store
        .native_session_transition_commit(owner, operation)
        .await?
        .context("transition_commit_missing")?;
    let mut expected = state
        .hub
        .session_info(&intent.request.session_id)
        .context("session_unavailable")?
        .meta;
    // Only the original or exact committed binding can pass Hub's comparison.
    expected.owner_user_id = Some(owner.into());
    expected.provider_account_id = intent.request.account_id.clone();
    expected.machine_id = intent.request.machine_id.clone();
    expected.provider = intent.request.provider.clone();
    expected.provider_version = intent.request.from_version.clone();
    expected.provider_generation_digest = intent.request.from_digest.clone();
    expected.provider_auth_generation = Some(intent.request.from_generation);
    store
        .with_native_session_binding(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            &intent.request.session_id,
            || {
                std::future::ready(
                    state
                        .hub
                        .project_native_binding(
                            &expected,
                            &intent.target.profile,
                            &binding.behavior,
                        )
                        .map_err(anyhow::Error::msg),
                )
            },
        )
        .await?;
    Ok(true)
}

/// Commit metadata only. True means the DB receipt exists, never that the
/// Machine accepted the replacement or a worker started. No Hub projection.
pub(super) async fn commit(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<bool> {
    let store = state.store.as_ref().context("store_unavailable")?;
    if store
        .native_session_transition_commit(owner, operation)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    if stop(state, owner, operation).await?.is_none() {
        return Ok(false);
    }
    let intent = store
        .native_session_transition(owner, operation)
        .await?
        .context("transition_not_found")?;
    let machine = &intent.request.machine_id;
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(machine)
        .context("machine_offline")?;
    ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "account_protocol_unavailable"
    );
    let registered = store
        .list_machines()
        .await?
        .into_iter()
        .find(|entry| &entry.id == machine && !entry.revoked)
        .context("machine_unavailable")?;
    let inventory: Vec<crate::machine_protocol::PluginInventory> = serde_json::from_value(
        registered
            .inventory
            .get("plugins")
            .or_else(|| registered.inventory.get("providers"))
            .cloned()
            .context("provider_inventory_unavailable")?,
    )?;
    let resolved = super::super::resolve_provider_generation_with_auth(
        &state.provider_catalog,
        &state.plugin_catalog,
        &inventory,
        &intent.request.provider,
        super::super::SchedulingAuthentication::Native(&intent.target.profile),
    )
    .map_err(anyhow::Error::msg)?;
    let binding = crate::store::provider_accounts::transitions::BindingCommit {
        behavior: resolved.behavior,
    };
    let stopped = store
        .native_session_transition_stop(owner, operation)
        .await?
        .and_then(|evidence| evidence.tree_exit)
        .context("transition_worker_exit_missing")?;
    state
        .machine_control
        .prepare_native_binding(
            &connection,
            machine,
            &crate::provider_account::PreparedBinding {
                operation: operation.into(),
                stopped,
                target: intent.target.profile.clone(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    store
        .commit_native_session_transition_checked(owner, operation, &binding, || {
            ensure!(
                state.machine_control.is_current(&connection),
                "machine_connection_changed"
            );
            let fence = state.plugin_lifecycle_fences.read();
            ensure!(
                !fence.contains_key(&(machine.clone(), intent.request.provider.clone())),
                "provider_changing"
            );
            let package = state
                .provider_catalog
                .package(
                    &intent.request.provider,
                    &intent.target.profile.provider_version,
                    &intent.target.profile.generation_digest,
                )
                .context("target_package_unavailable")?;
            ensure!(
                package.manifest.runtime.behavior == binding.behavior,
                "target_behavior_changed"
            );
            let current = state
                .hub
                .session_info(&intent.request.session_id)
                .context("session_unavailable")?
                .meta;
            ensure!(
                !current.closing
                    && current.owner_user_id.as_deref() == Some(owner)
                    && current.provider_account_id == intent.request.account_id
                    && current.machine_id == *machine
                    && current.provider == intent.request.provider
                    && current.provider_version == intent.request.from_version
                    && current.provider_generation_digest == intent.request.from_digest
                    && current.provider_auth_generation == Some(intent.request.from_generation),
                "native_session_binding_changed"
            );
            Ok(())
        })
        .await?;
    Ok(true)
}

/// Caller must authenticate the owner and authorize this migration operation.
/// None means unconfirmed, never stopped. Repeating uses the same stop identity.
pub(super) async fn stop(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<Option<Evidence>> {
    let store = state.store.as_ref().context("store_unavailable")?;
    if let Some(evidence) = store
        .native_session_transition_stop(owner, operation)
        .await?
    {
        return Ok(Some(evidence));
    }
    let intent = store
        .native_session_transition(owner, operation)
        .await?
        .context("transition_not_found")?;
    let machine = &intent.request.machine_id;
    let Some(runtime) = state.runtime_router.runtime(machine) else {
        return Ok(None);
    };
    let Some(epoch) = runtime.account_stop_connection() else {
        return Ok(None);
    };
    let Some((connection, protocol)) = state.machine_control.connection_protocol(machine) else {
        return Ok(None);
    };
    ensure!(
        protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        "account_protocol_unavailable"
    );
    let current = || {
        runtime.account_stop_connection() == Some(epoch)
            && state
                .runtime_router
                .runtime(machine)
                .is_some_and(|active| Arc::ptr_eq(&active, &runtime))
            && state.machine_control.is_current(&connection)
    };
    let pending = store
        .dispatch_native_session_transition_stop(owner, operation, |current_intent| {
            ensure!(
                current_intent == &intent && current(),
                "machine_connection_changed"
            );
            ensure!(
                runtime.account_stop_recipient().as_ref()
                    == Some(&intent.target.recipient_fingerprint),
                "machine_enrollment_changed"
            );
            // Namespace prevents collisions with account-wide revocation commands.
            runtime.quiesce_account_session(
                &format!("migration:{operation}"),
                &intent.request.session_id,
                &intent.request.account_id,
            )
        })
        .await?;
    match pending.wait(std::time::Duration::from_secs(30)).await {
        Outcome::Stopped => {}
        Outcome::Refused(reason) => anyhow::bail!("migration_stop_refused: {reason}"),
        Outcome::Unconfirmed => return Ok(None),
    }
    ensure!(current(), "machine_connection_changed");
    let tree = state
        .machine_control
        .query_worker_exit(
            &connection,
            machine,
            &WorkerExitRequest {
                stop_command_id: Some(crate::provider_account::account_stop_command_id(
                    &format!("migration:{operation}"),
                    &intent.request.session_id,
                    &intent.request.account_id,
                )),
                account_id: intent.request.account_id.clone(),
                session_id: intent.request.session_id.clone(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    let evidence = Evidence {
        account_id: intent.request.account_id.clone(),
        session_id: intent.request.session_id.clone(),
        machine_id: machine.clone(),
        recipient_fingerprint: intent.target.recipient_fingerprint.clone(),
        tree_exit: Some(tree),
    };
    store
        .record_native_session_transition_stop(owner, operation, &evidence, current)
        .await?;
    Ok(Some(evidence))
}
