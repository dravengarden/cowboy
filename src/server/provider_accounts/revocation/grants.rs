//! Local vault stage. No result here establishes remote wipe or native logout.
use super::*;
use crate::provider_service::{ProviderAuthService, grants::revocation::Revocation};
use crate::store::provider_accounts::grants::Phase;
use crate::store::provider_accounts::revocation::LocalWipe;

#[derive(Debug)]
pub(super) enum LocalCleanup {
    Committed(Revocation),
    Cancelled { cleanup_applied: bool },
}

pub(super) struct GrantCleanup {
    pub operation_id: String,
    pub outcome: Result<LocalCleanup>,
}

pub(super) async fn revoke_local_grants(
    store: &Store,
    vault: &ProviderAuthService,
    owner: &str,
    operation: &str,
) -> Result<Vec<GrantCleanup>> {
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let plan = receipt.revocation.context("revocation_plan_missing")?;
    let results = super::batch::bounded_pass(&plan.grants, |target| async move {
        store
            .with_provider_local_wipe(owner, operation, &target.operation_id, |grant| {
                let scope = vault.existing_revocation_grant(grant)?;
                match grant.phase {
                    Phase::Committed { .. } => match scope.revoke_committed_grant(grant)? {
                        Revocation::Applied { generation }
                        | Revocation::AlreadySignedOut { generation } => {
                            Ok(LocalWipe::Wiped { generation })
                        }
                        Revocation::Superseded => anyhow::bail!("grant_superseded"),
                    },
                    Phase::Cancelled => {
                        anyhow::ensure!(
                            scope.discard_revoking_grant(grant)?,
                            "grant_cleanup_unconfirmed"
                        );
                        Ok(LocalWipe::CancelledDiscarded)
                    }
                    Phase::Prepared => anyhow::bail!("grant_cancellation_required"),
                }
            })
            .await
            .map(|evidence| match evidence {
                LocalWipe::Wiped { generation } => {
                    LocalCleanup::Committed(Revocation::AlreadySignedOut { generation })
                }
                LocalWipe::CancelledDiscarded => LocalCleanup::Cancelled {
                    cleanup_applied: true,
                },
            })
    })
    .await;
    Ok(plan
        .grants
        .into_iter()
        .zip(results)
        .map(|(target, outcome)| GrantCleanup {
            operation_id: target.operation_id,
            outcome,
        })
        .collect())
}
