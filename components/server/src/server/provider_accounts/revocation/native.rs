//! Reconcile original native logout operations; never infer upstream revocation.
use crate::{
    machine_control::MachineControl,
    provider_account::{NativeLogoutObservation, NativeLogoutRequest},
    store::{
        Store,
        provider_accounts::{logins::native_logout::Completion, revocation::OperationTarget},
    },
};
use anyhow::{Context as _, Result, ensure};
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    NotApplicable,
    CommandSucceeded,
    ProfileCleaned,
    CommandFailed,
    Unconfirmed,
}

#[derive(Debug, Serialize)]
pub(super) struct Target {
    operation: String,
    machine: String,
    state: State,
}

pub(super) fn permits_cleanup(targets: &[Target]) -> bool {
    targets
        .iter()
        .all(|target| matches!(target.state, State::NotApplicable | State::ProfileCleaned))
}

pub(super) async fn run(
    store: &Store,
    control: &MachineControl,
    owner: &str,
    operation: &str,
) -> Result<Vec<Target>> {
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let plan = receipt.revocation.context("revocation_plan_missing")?;
    Ok(super::batch::bounded_pass(&plan.logins, |target| async {
        // A short observation wait keeps offline/slow Machines from starving
        // other targets. Dropping it never cancels the remote command; a retry
        // observes the Machine's durable original-operation reservation.
        let state = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            deliver(store, control, owner, operation, target),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(State::Unconfirmed);
        Target {
            operation: target.operation_id.clone(),
            machine: target.machine_id.clone(),
            state,
        }
    })
    .await)
}

async fn deliver(
    store: &Store,
    control: &MachineControl,
    owner: &str,
    operation: &str,
    target: &OperationTarget,
) -> Result<State> {
    let login = store
        .provider_login_receipt(owner, &target.operation_id)
        .await?
        .context("login_operation_missing")?;
    ensure!(
        login.input.machine_id == target.machine_id,
        "login_machine_mismatch"
    );
    if login.never_dispatched() {
        return Ok(State::NotApplicable);
    }
    let Some(generation) = native_generation(&login)? else {
        return Ok(State::NotApplicable);
    };
    let proposal = Completion {
        revocation_operation: operation.into(),
        machine_id: target.machine_id.clone(),
        recipient_fingerprint: login
            .dispatch_recipient_fingerprint
            .clone()
            .context("login_enrollment_unknown")?,
        observation: NativeLogoutObservation {
            process_trees_exited: false,
            profile_cleaned: false,
            request: NativeLogoutRequest {
                account_id: login.input.account_id,
                provider_id: login.provider_id,
                generation,
                login_request_id: login.request_id,
                operation_id: operation.into(),
            },
            succeeded: None,
        },
    };
    if let Some(recorded) = login.native_logout {
        let mut expected = proposal.clone();
        expected.observation.succeeded = recorded.observation.succeeded;
        expected.observation.profile_cleaned = recorded.observation.profile_cleaned;
        expected.observation.process_trees_exited = recorded.observation.process_trees_exited;
        ensure!(recorded == expected, "native_logout_scope_mismatch");
        if recorded.observation.succeeded != Some(true) || recorded.observation.profile_cleaned {
            return Ok(outcome(&recorded.observation));
        }
    }
    let (connection, _) = control
        .connection_protocol(&target.machine_id)
        .context("machine_offline")?;
    let pending = store
        .with_native_logout_dispatch(owner, &target.operation_id, &proposal, || {
            control
                .begin_native_logout(
                    &connection,
                    &target.machine_id,
                    &proposal.observation.request,
                )
                .map_err(anyhow::Error::msg)
        })
        .await?;
    let Some(pending) = pending else {
        let recorded = store
            .provider_login_receipt(owner, &target.operation_id)
            .await?
            .and_then(|login| login.native_logout)
            .context("native_logout_receipt_missing")?;
        return Ok(outcome(&recorded.observation));
    };
    let observation = pending.await.map_err(anyhow::Error::msg)?;
    let completion = Completion {
        observation,
        ..proposal
    };
    let receipt = store
        .record_native_logout_completion(owner, &target.operation_id, &completion, || {
            control.is_current(&connection)
        })
        .await?;
    Ok(outcome(
        &receipt
            .native_logout
            .context("native_logout_receipt_missing")?
            .observation,
    ))
}

fn native_generation(
    login: &crate::store::provider_accounts::logins::Receipt,
) -> Result<Option<u64>> {
    // Old native receipts can predate generation reservations. Missing metadata
    // is not proof that the native profile never existed.
    ensure!(
        login.native_generation.is_some()
            || (login.provider_id != "claude-code" && login.native_completion.is_none()),
        "native_generation_unknown"
    );
    Ok(login.native_generation)
}

fn outcome(observation: &NativeLogoutObservation) -> State {
    match observation.succeeded {
        Some(true) if observation.profile_cleaned => State::ProfileCleaned,
        Some(true) => State::CommandSucceeded,
        Some(false) => State::CommandFailed,
        None => State::Unconfirmed,
    }
}

#[cfg(test)]
mod tests;
