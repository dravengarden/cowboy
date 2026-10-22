//! Diagnostic evidence inventory, never completion or dispatch authority.
use super::{LocalWipe, grant_admission, grants};
use crate::store::{Store, provider_accounts::Receipt};
use anyhow::{Context as _, Result, ensure};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Readiness {
    pub account_completed: bool,
    pub missing: Vec<Missing>,
    /// Public lifecycle entry remains gated; completion uses a locked writer.
    pub completion_admission: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Missing {
    pub kind: &'static str,
    pub target: String,
}

impl Store {
    /// Read immutable plan locators and durable historical evidence. Concurrent
    /// progress may make this inventory conservative; it never authorizes work.
    pub async fn provider_revocation_readiness(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Readiness> {
        let receipt = self
            .provider_account_receipt(owner, operation)
            .await?
            .context("revocation_operation_missing")?;
        let plan = receipt
            .revocation
            .as_ref()
            .context("revocation_plan_missing")?;
        let mut missing = Vec::new();
        for target in &plan.sessions {
            let stopped = self
                .provider_account_stop(owner, operation, &target.session_id)
                .await?;
            missing.extend(session_missing(&receipt, target, stopped.as_ref()));
        }
        for target in &plan.logins {
            let login = self
                .provider_login_receipt(owner, &target.operation_id)
                .await?
                .context("login_operation_missing")?;
            let cleanup_valid =
                grant_admission::validate_login_cleanup(&receipt, target, &login, operation)
                    .is_ok();
            if !cleanup_valid {
                missing.push(Missing {
                    kind: "login_cleanup",
                    target: target.operation_id.clone(),
                });
            }
            // A leader-exit receipt and native logout command success do not
            // cover descendants. Never silently upgrade those existing proofs.
            let never_spawned = cleanup_valid
                && (login.never_dispatched()
                    || receipt
                        .login_stops
                        .get(&target.operation_id)
                        .is_some_and(|e| {
                            !e.observation.process_started
                                && !e.observation.leader_exited
                                && e.observation.candidate_removed
                        }));
            let native_trees_exited = cleanup_valid
                && login
                    .native_logout
                    .as_ref()
                    .is_some_and(|e| e.observation.process_trees_exited);
            let portable_tree_exited = cleanup_valid
                && receipt
                    .login_stops
                    .get(&target.operation_id)
                    .is_some_and(|e| {
                        e.observation.process_tree_exited && e.observation.leader_exited
                    });
            if !never_spawned && !native_trees_exited && !portable_tree_exited {
                missing.push(Missing {
                    kind: "login_process_tree_exit",
                    target: target.operation_id.clone(),
                });
            }
        }
        for target in &plan.grants {
            let grant = self
                .provider_grant_receipt(owner, &target.operation_id)
                .await?
                .context("grant_operation_missing")?;
            ensure!(
                grant.account_id == receipt.account_id
                    && grant.machine_id == target.machine_id
                    && grant.operation_id == target.operation_id,
                "grant_scope_mismatch"
            );
            missing.extend(grant_missing(&receipt, &grant));
        }
        Ok(Readiness {
            account_completed: receipt.revocation_completed,
            missing,
            completion_admission: "internal_only",
        })
    }
}

#[cfg(test)]
mod tests;

pub(super) fn grant_missing(receipt: &Receipt, grant: &grants::Receipt) -> Vec<Missing> {
    let local = match (&grant.phase, receipt.local_wipes.get(&grant.operation_id)) {
        (grants::Phase::Cancelled, Some(LocalWipe::CancelledDiscarded)) => true,
        (grants::Phase::Committed { generation }, Some(LocalWipe::Wiped { generation: wiped })) => {
            grant.expected_generation.checked_add(1) == Some(*generation)
                && wiped > generation
                && *wiped < u64::MAX
        }
        _ => false,
    };
    let mut missing = Vec::new();
    if !local {
        missing.push(Missing {
            kind: "local_grant_wipe",
            target: grant.operation_id.clone(),
        });
    }
    match grant.phase {
        grants::Phase::Prepared => missing.push(Missing {
            kind: "grant_cancellation",
            target: grant.operation_id.clone(),
        }),
        grants::Phase::Cancelled => (), // Never admitted for delivery.
        grants::Phase::Committed { generation } => {
            let remote = receipt
                .remote_wipes
                .get(&grant.operation_id)
                .is_some_and(|e| {
                    e.grant_operation == grant.operation_id
                        && e.machine_id == grant.machine_id
                        && e.recipient_fingerprint == grant.recipient_fingerprint
                        && e.generation > generation
                        && e.generation < u64::MAX
                        && grant.expected_generation.checked_add(1) == Some(generation)
                });
            if !remote {
                missing.push(Missing {
                    kind: "remote_grant_wipe",
                    target: grant.operation_id.clone(),
                });
            }
        }
    }
    missing
}

pub(super) fn session_missing(
    receipt: &Receipt,
    target: &super::SessionTarget,
    stopped: Option<&super::stops::Evidence>,
) -> Vec<Missing> {
    let mut missing = Vec::new();
    let scoped_stop = stopped.as_ref().is_some_and(|e| {
        e.account_id == receipt.account_id
            && e.session_id == target.session_id
            && target.machine_id.as_ref() == Some(&e.machine_id)
    });
    if !scoped_stop {
        missing.push(Missing {
            kind: "session_stop",
            target: target.session_id.clone(),
        });
    }
    // Existing acknowledgements share one format across systemd and
    // Direct mode. Direct mode observes only the owned leader, so this
    // historical receipt cannot certify descendant termination.
    if !scoped_stop
        || !stopped
            .as_ref()
            .and_then(|e| e.tree_exit.as_ref())
            .is_some_and(|tree| {
                tree.request.account_id == receipt.account_id
                    && tree.request.session_id == target.session_id
                    && !tree.launch_id.is_nil()
            })
    {
        missing.push(Missing {
            kind: "session_process_tree_exit",
            target: target.session_id.clone(),
        });
    }
    missing
}
