//! Removal of a revoked named-only account's sandbox on every Machine that
//! may hold it (`store::provider_accounts::revocation::account_removals`).
//! Runs only once every planned session is stopped and every native profile
//! is cleaned up; completion waits for every Machine's receipt.
use crate::store::{
    Store,
    provider_accounts::revocation::account_removals::{
        self, Evidence, RemoveObservation, RemoveRequest,
    },
};
use anyhow::{Context as _, Result};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Removed,
    /// Offline, no reply yet, or a Machine that cannot remove accounts yet.
    Pending,
    /// The daemon answered without confirming both removals.
    Unconfirmed,
}

#[derive(Debug, Serialize)]
pub(super) struct Target {
    machine: String,
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// One Machine's destructive `remove_account`, for the Controller.
pub(super) trait Remover: Send + Sync {
    /// Whether `machine` is connected; the closure checks it is still the
    /// same connection when the receipt is recorded.
    fn connection(&self, machine: &str) -> Option<Box<dyn Fn() -> bool + Send + Sync>>;
    /// The fingerprint of `machine`'s current enrollment.
    fn enrollment(&self, machine: &str) -> futures::future::BoxFuture<'static, Option<String>>;
    fn remove(
        &self,
        machine: &str,
        request: &RemoveRequest,
    ) -> futures::future::BoxFuture<'static, Result<RemoveObservation, String>>;
}

/// The production [`Remover`]: the Machine's account-sandbox daemon
/// `remove_account` (`MachineCommand::AccountSandboxRemove`, answered only by
/// `MachineEvent::AccountSandboxRemoveObserved` for the same request). A
/// Machine below protocol 48, offline or refusing keeps the removal pending.
pub(super) struct MachineRemover(pub(super) Arc<super::super::super::AppState>);

impl Remover for MachineRemover {
    fn connection(&self, machine: &str) -> Option<Box<dyn Fn() -> bool + Send + Sync>> {
        let (connection, _) = self.0.machine_control.connection_protocol(machine)?;
        let state = Arc::clone(&self.0);
        Some(Box::new(move || {
            state.machine_control.is_current(&connection)
        }))
    }

    fn enrollment(&self, machine: &str) -> futures::future::BoxFuture<'static, Option<String>> {
        let (state, machine) = (Arc::clone(&self.0), machine.to_owned());
        Box::pin(async move {
            let key = state
                .store
                .as_ref()?
                .machine_encryption_public_key(&machine)
                .await
                .ok()??;
            Some(crate::admin::hex_sha256(key.as_bytes()))
        })
    }

    fn remove(
        &self,
        machine: &str,
        request: &RemoveRequest,
    ) -> futures::future::BoxFuture<'static, Result<RemoveObservation, String>> {
        let (state, machine, request) = (Arc::clone(&self.0), machine.to_owned(), request.clone());
        Box::pin(async move {
            state
                .machine_control
                .account_sandbox_remove(&machine, &request)
                .await
        })
    }
}

pub(super) async fn run(
    store: &Store,
    remover: &dyn Remover,
    owner: &str,
    operation: &str,
) -> Result<Vec<Target>> {
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let mut targets = Vec::new();
    for machine in account_removals::required_machines(&receipt) {
        if receipt.account_removals.contains_key(&machine) {
            targets.push(Target {
                machine,
                state: State::Removed,
                detail: None,
            });
            continue;
        }
        let request = account_removals::request(&receipt, operation, &machine)
            .context("account_removal_scope_mismatch")?;
        let (state, detail) =
            remove_one(store, remover, owner, operation, &machine, &request).await;
        targets.push(Target {
            machine,
            state,
            detail,
        });
    }
    Ok(targets)
}

async fn remove_one(
    store: &Store,
    remover: &dyn Remover,
    owner: &str,
    operation: &str,
    machine: &str,
    request: &RemoveRequest,
) -> (State, Option<String>) {
    let Some(current) = remover.connection(machine) else {
        return (State::Pending, Some("machine_offline".into()));
    };
    let Some(enrollment) = remover.enrollment(machine).await else {
        return (State::Pending, Some("machine_unavailable".into()));
    };
    let observation = match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        remover.remove(machine, request),
    )
    .await
    {
        Ok(Ok(observation)) => observation,
        Ok(Err(detail)) => return (State::Pending, Some(detail)),
        Err(_) => return (State::Pending, Some("timeout".into())),
    };
    if observation.request != *request || !(observation.home_removed && observation.user_removed) {
        return (
            State::Unconfirmed,
            Some("account_removal_unconfirmed".into()),
        );
    }
    let evidence = Evidence {
        machine_id: machine.to_owned(),
        observation,
        observed_at_ms: crate::usage::now_ms(),
        recipient_fingerprint: Some(enrollment),
    };
    match store
        .record_provider_account_removal(owner, operation, &evidence, current)
        .await
    {
        Ok(_) => (State::Removed, None),
        Err(error) => (State::Pending, Some(error.to_string())),
    }
}

#[cfg(test)]
mod tests;
