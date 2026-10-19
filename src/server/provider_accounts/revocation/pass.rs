//! One resumable revocation pass. Never reports upstream logout as completed.
use super::*;
use crate::provider_service::{ProviderAuthService, grants::revocation::Revocation};
use serde::Serialize;
use std::future::Future;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum State {
    Pending,
    CancellationRequested,
    CleanupRecorded,
    NotDispatched,
    Stopped,
    LocalWipeRecorded,
    Superseded,
    Refused,
    Failed,
}

#[derive(Debug, Serialize)]
pub(super) struct Target {
    id: String,
    state: State,
}

#[derive(Debug, Serialize)]
pub(super) struct Report {
    status: &'static str,
    logins: Vec<Target>,
    sessions: Vec<Target>,
    local_grants: Vec<Target>,
    native_logout: State,
}

impl Report {
    pub(super) fn sessions_stopped(&self) -> bool {
        self.sessions
            .iter()
            .all(|target| target.state == State::Stopped)
    }
}

/// The caller supplies its owner-authorized login canceller. Each stage still
/// performs its own admission checks; this initial snapshot is not authority.
pub(super) async fn run<F, Fut>(
    store: &Store,
    router: &RuntimeRouter,
    vault: &ProviderAuthService,
    owner: &str,
    operation: &str,
    cancel_login: F,
) -> Result<Report>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let plan = receipt
        .revocation
        .as_ref()
        .context("revocation_plan_missing")?;
    let registry = store.provider_accounts(owner).await?;
    anyhow::ensure!(
        registry
            .accounts
            .iter()
            .any(|account| account.id == receipt.account_id
                && account.revoking
                && !account.enabled),
        "account_not_revoking"
    );
    let logins = batch::bounded_pass(&plan.logins, |target| async {
        let never_dispatched = store
            .provider_login_receipt(owner, &target.operation_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|login| {
                login.input.account_id == receipt.account_id
                    && login.input.machine_id == target.machine_id
                    && login.cancel_requested
                    && login.never_dispatched()
            });
        Target {
            id: target.operation_id.clone(),
            state: if never_dispatched {
                State::NotDispatched
            } else if recorded_login_cleanup(&receipt, target) {
                State::CleanupRecorded
            } else {
                match tokio::time::timeout(
                    Duration::from_secs(2),
                    cancel_login(target.operation_id.clone()),
                )
                .await
                {
                    Ok(Ok(())) => State::CancellationRequested,
                    Ok(Err(_)) => State::Failed,
                    Err(_) => State::Pending,
                }
            },
        }
    });
    let (logins, sessions) = tokio::join!(
        logins,
        batch::stop_sessions(store, router, owner, operation)
    );
    let sessions: Vec<_> = sessions?
        .into_iter()
        .map(|target| Target {
            id: target.session_id,
            state: match target.outcome {
                Ok(Step::Stopped(_)) => State::Stopped,
                Ok(Step::Pending) => State::Pending,
                Ok(Step::Refused(_)) => State::Refused,
                Err(_) => State::Failed,
            },
        })
        .collect();
    // Do not erase central grant material while a planned session may still
    // be running. Failed/offline targets remain visible and retryable.
    let local_grants = if sessions.iter().all(|target| target.state == State::Stopped) {
        grants::revoke_local_grants(store, vault, owner, operation)
            .await?
            .into_iter()
            .map(|target| Target {
                id: target.operation_id,
                state: match target.outcome {
                    Ok(grants::LocalCleanup::Committed(
                        Revocation::Applied { .. } | Revocation::AlreadySignedOut { .. },
                    )) => State::LocalWipeRecorded,
                    Ok(grants::LocalCleanup::Committed(Revocation::Superseded)) => {
                        State::Superseded
                    }
                    Ok(grants::LocalCleanup::Cancelled {
                        cleanup_applied: true,
                    }) => State::LocalWipeRecorded,
                    Ok(grants::LocalCleanup::Cancelled {
                        cleanup_applied: false,
                    }) => State::Pending,
                    Err(error) => cleanup_error_state(&error),
                },
            })
            .collect()
    } else {
        plan.grants
            .iter()
            .map(|target| Target {
                id: target.operation_id.clone(),
                state: State::Pending,
            })
            .collect()
    };
    Ok(Report {
        status: "revoking",
        logins,
        sessions,
        local_grants,
        native_logout: State::Pending,
    })
}

fn recorded_login_cleanup(
    receipt: &crate::store::provider_accounts::Receipt,
    target: &crate::store::provider_accounts::revocation::OperationTarget,
) -> bool {
    receipt
        .login_stops
        .get(&target.operation_id)
        .is_some_and(|evidence| {
            evidence.login_operation == target.operation_id
                && evidence.machine_id == target.machine_id
                && evidence.observation.request.account_id == receipt.account_id
                && evidence.observation.candidate_removed
                && evidence.observation.process_started == evidence.observation.leader_exited
        })
}

fn cleanup_error_state(error: &anyhow::Error) -> State {
    match error.to_string().as_str() {
        "session_stop_unconfirmed"
        | "login_cleanup_unconfirmed"
        | "native_cleanup_unconfirmed"
        | "session_tree_exit_unconfirmed"
        | "login_tree_exit_unconfirmed" => State::Pending,
        _ => State::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_matching_durable_cleanup_suppresses_cancellation() {
        use crate::store::provider_accounts::{
            Receipt,
            revocation::{OperationTarget, login_stops::Evidence},
        };
        let account = "11111111-1111-4111-8111-111111111111";
        let mut receipt: Receipt = serde_json::from_value(serde_json::json!({
            "registry": {"revision": 1, "accounts": [], "defaults": {}}, "account_id": account,
        }))
        .unwrap();
        let target = OperationTarget {
            operation_id: "login".into(),
            machine_id: "machine".into(),
        };
        assert!(!recorded_login_cleanup(&receipt, &target));
        let original: Evidence = serde_json::from_value(serde_json::json!({
            "login_operation": "login", "machine_id": "machine", "recipient_fingerprint": "original",
            "observation": {"request": {"account_id": account, "provider_id": "grok", "login_request_id": "request"},
                "process_started": false, "leader_exited": false, "candidate_removed": true},
        })).unwrap();
        for mode in [
            "valid",
            "machine",
            "operation",
            "account",
            "removed",
            "exit",
        ] {
            let mut evidence = original.clone();
            match mode {
                "machine" => evidence.machine_id = "other".into(),
                "operation" => evidence.login_operation = "other".into(),
                "account" => {
                    evidence.observation.request.account_id =
                        uuid::Uuid::new_v4().to_string().try_into().unwrap();
                }
                "removed" => evidence.observation.candidate_removed = false,
                "exit" => evidence.observation.process_started = true,
                _ => (),
            }
            receipt.login_stops.insert("login".into(), evidence);
            assert_eq!(recorded_login_cleanup(&receipt, &target), mode == "valid");
        }
    }

    #[test]
    fn cleanup_waits_are_pending_but_scope_and_storage_errors_remain_failures() {
        for reason in [
            "session_stop_unconfirmed",
            "session_tree_exit_unconfirmed",
            "login_tree_exit_unconfirmed",
            "login_cleanup_unconfirmed",
            "native_cleanup_unconfirmed",
        ] {
            assert_eq!(
                cleanup_error_state(&anyhow::anyhow!(reason)),
                State::Pending
            );
        }
        for reason in [
            "login_cleanup_scope_mismatch",
            "session_stop_scope_mismatch",
            "session_tree_exit_scope_mismatch",
            "disk full",
            "machine_enrollment_changed",
        ] {
            assert_eq!(cleanup_error_state(&anyhow::anyhow!(reason)), State::Failed);
        }
    }
}
