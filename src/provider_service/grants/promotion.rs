//! Complete a verified native candidate without treating a journal entry as
//! runtime authority. Uncertain DB results retain the vault proof for recovery.
use super::*;
use crate::store::provider_accounts::grants::Outcome;

impl ProviderAuthService {
    /// The caller verifies the signed native candidate and prepares its exact
    /// intent first. A terminal operation is history only: it never writes a
    /// new credential generation. Distribution requires separate live admission.
    pub(crate) async fn promote_prepared_grant(
        &self,
        store: &crate::store::Store,
        owner: &str,
        intent: &Receipt,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
    ) -> Result<Recovery> {
        self.validate_grant_intent(intent)?;
        validate_candidate(package, bundle, intent)?;
        let current = store
            .provider_grant_receipt(owner, &intent.operation_id)
            .await?
            .context("Provider grant operation not found")?;
        let mut prepared = current.clone();
        prepared.phase = Phase::Prepared;
        ensure!(&prepared == intent, "Provider grant intent mismatch");
        if current.phase != Phase::Prepared {
            return self.settle_grant(current);
        }
        let proof = self.commit_grant(package, bundle, intent)?;
        // Do not roll back on an unknown outcome: the DB may have committed.
        // Recovery authenticates the durable proof and resolves the same intent.
        self.finish_grant(store, owner, intent, &proof).await
    }

    pub(crate) async fn finish_grant(
        &self,
        store: &crate::store::Store,
        owner: &str,
        intent: &Receipt,
        proof: &VerifiedCommit,
    ) -> Result<Recovery> {
        match store
            .finish_verified_provider_grant(owner, intent, proof)
            .await?
        {
            Outcome::Applied(receipt) => self.settle_grant(receipt),
            Outcome::Rejected(code) => {
                anyhow::bail!("Provider grant finalization rejected: {code}")
            }
        }
    }

    pub(super) fn settle_grant(&self, receipt: Receipt) -> Result<Recovery> {
        let cleanup_applied =
            receipt.phase == Phase::Cancelled && self.discard_cancelled_grant(&receipt)?;
        Ok(Recovery {
            receipt,
            cleanup_applied,
        })
    }
}
