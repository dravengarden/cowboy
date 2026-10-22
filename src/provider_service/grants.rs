//! Operation-bound native login commits. Recovery reads the same authenticated
//! vault entry as credential delivery; a separate sidecar cannot claim success.
#![allow(dead_code)] // Named login remains gated until admission/revocation are wired.

use super::*;
use crate::store::provider_accounts::grants::{Phase, Receipt};

mod promotion;

/// Canonical candidate binding. Only this digest, never the serialized bundle,
/// belongs in the DB journal. The caller separately verifies the signed release
/// and native identity evidence before preparing the operation.
pub fn candidate_fingerprint(
    package: &ProviderPackage,
    bundle: &PortableCredentialBundle,
    identity_fingerprint: &str,
) -> Result<String> {
    validate_bundle(&package.manifest.authentication, bundle)?;
    ensure!(
        serde_json::to_vec(bundle)?.len() <= MAX_BUNDLE_BYTES,
        "credential bundle is too large"
    );
    Ok(crate::admin::hex_sha256(&serde_json::to_vec(&(
        "cowboy-native-login-candidate-v1",
        package,
        bundle,
        identity_fingerprint,
    ))?))
}

fn validate_candidate(
    package: &ProviderPackage,
    bundle: &PortableCredentialBundle,
    intent: &Receipt,
) -> Result<()> {
    crate::provider_account::portable_credentials_allowed(&intent.account_id, &package.manifest.id)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        package.manifest.id == intent.provider_id,
        "Provider grant Provider mismatch"
    );
    ensure!(
        candidate_fingerprint(package, bundle, &intent.identity_fingerprint)?
            == intent.candidate_fingerprint,
        "Provider grant candidate mismatch"
    );
    Ok(())
}
mod refresh;
pub mod revocation;

/// Recovery describes history only. It is never dispatch authorization.
#[derive(Debug, PartialEq, Eq)]
pub struct Recovery {
    pub receipt: Receipt,
    pub cleanup_applied: bool,
}

/// Constructible only after a durable vault write or authenticated vault read.
pub struct VerifiedCommit {
    intent_hash: String,
    generation: u64,
}

impl VerifiedCommit {
    pub fn generation_for(&self, intent: &Receipt) -> Result<u64> {
        ensure!(
            self.intent_hash == intent_hash(intent)?,
            "Provider grant intent mismatch"
        );
        Ok(self.generation)
    }
}

fn intent_hash(intent: &Receipt) -> Result<String> {
    ensure!(
        intent.phase == Phase::Prepared,
        "Provider grant is not prepared"
    );
    intent.commitment()
}

impl ProviderAuthService {
    /// Resolve an exact committed operation, then recheck current authority.
    /// Missing routes never fall back to legacy credentials or another account.
    pub async fn seal_current_grant(
        &self,
        store: &crate::store::Store,
        machine: &str,
        provider: &str,
    ) -> Result<SealedProviderAuth> {
        let commitment = self
            .providers
            .read()
            .get(provider)
            .and_then(|stored| stored.promotion_commitment.clone())
            .context("Provider grant has no operation-bound credentials")?;
        let (owner, operation) = store
            .provider_grant_route(&commitment)
            .await?
            .context("Provider grant has no committed route")?;
        store
            .with_provider_grant_admission(&owner, &operation, machine, |admission| {
                ensure!(
                    admission.receipt().provider_id == provider
                        && admission.receipt().account_id == self.account_id,
                    "Provider grant route scope mismatch"
                );
                self.seal_admitted_grant(admission)
            })
            .await
    }

    /// Called only inside current admission. The authenticated current vault
    /// generation must belong to, and not predate, this committed login lineage.
    pub fn seal_admitted_grant(
        &self,
        admission: &crate::store::provider_accounts::grants::Admission,
    ) -> Result<SealedProviderAuth> {
        let mut intent = admission.receipt().clone();
        let Phase::Committed { generation } = intent.phase else {
            anyhow::bail!("Provider grant is not committed");
        };
        intent.phase = Phase::Prepared;
        let hash = self.validate_grant_intent(&intent)?;
        self.seal_for_machine_with_admission(
            &intent.provider_id,
            admission.recipient(),
            Some((&hash, generation)),
        )
    }

    /// Resume one durable operation after an uncertain response or restart.
    /// The Controller supplies the currently enrolled recipient; this method
    /// neither opens new grants nor executes native login/credential refresh.
    pub async fn reconcile_grant(
        &self,
        store: &crate::store::Store,
        owner: &str,
        operation: &str,
        machine_id: &str,
        enrolled_key: &str,
    ) -> Result<Recovery> {
        let receipt = store
            .provider_grant_receipt(owner, operation)
            .await?
            .context("Provider grant operation not found")?;
        ensure!(
            receipt.machine_id == machine_id
                && receipt.recipient_fingerprint
                    == crate::admin::hex_sha256(enrolled_key.as_bytes()),
            "Provider grant enrollment changed"
        );
        let grant = self.existing_machine_grant(&receipt.account_id, enrolled_key)?;
        if receipt.phase == Phase::Prepared
            && let Some(proof) = grant.recover_grant(&receipt)?
        {
            return grant.finish_grant(store, owner, &receipt, &proof).await;
        }
        grant.settle_grant(receipt)
    }

    /// Reconcile a DB-cancelled promotion using compare-and-swap. Never erase a
    /// newer login or refresh. Retain a wipe tombstone for replica cleanup.
    pub fn discard_cancelled_grant(&self, cancelled: &Receipt) -> Result<bool> {
        ensure!(
            cancelled.phase == Phase::Cancelled,
            "Provider grant is not cancelled"
        );
        let mut intent = cancelled.clone();
        intent.phase = Phase::Prepared;
        let hash = self.validate_grant_intent(&intent)?;
        let mut providers = self.providers.write();
        let Some(current) = providers.get(&intent.provider_id) else {
            return Ok(false);
        };
        if current.promotion_commitment.as_ref() != Some(&hash)
            || intent.expected_generation.checked_add(1) != Some(current.auth_generation)
        {
            return Ok(false);
        }
        // Authenticate before treating any metadata as proof of this operation.
        self.open_vault(current)?;
        let mut tombstone = current.clone();
        tombstone.auth_generation = current
            .auth_generation
            .checked_add(1)
            .filter(|value| *value != u64::MAX)
            .context("Provider auth generation exhausted")?;
        let (nonce, ciphertext) = self.seal_vault(
            &intent.provider_id,
            tombstone.auth_generation,
            &current.auth_contract_fingerprint,
            &[],
            None,
        )?;
        tombstone.nonce = nonce;
        tombstone.ciphertext = ciphertext;
        tombstone.promotion_commitment = None;
        tombstone.action = ProviderAuthAction::Wipe;
        tombstone.authentication_state = ServiceAuthenticationState::SignedOut;
        tombstone.distribution_state = ServiceDistributionState::Revoking;
        tombstone.account_label = None;
        tombstone.updated_at_ms = now_ms();
        self.replace_locked(&mut providers, tombstone)?;
        Ok(true)
    }

    fn validate_grant_intent(&self, intent: &Receipt) -> Result<String> {
        ensure!(
            !self.account_id.is_default() && self.account_id == intent.account_id,
            "Provider grant account mismatch"
        );
        let recipient = self
            .recipient_key
            .as_ref()
            .context("Provider grant requires a recipient")?;
        ensure!(
            crate::admin::hex_sha256(recipient.as_bytes()) == intent.recipient_fingerprint,
            "Provider grant recipient mismatch"
        );
        intent_hash(intent)
    }

    /// Authenticate exact durable state without replaying a login or write.
    /// None means no matching commit exists; callers must not guess success from
    /// a generation number. The DB and current enrollment still govern delivery.
    pub fn recover_grant(&self, intent: &Receipt) -> Result<Option<VerifiedCommit>> {
        let hash = self.validate_grant_intent(intent)?;
        let providers = self.providers.read();
        let Some(stored) = providers.get(&intent.provider_id) else {
            return Ok(None);
        };
        if stored.promotion_commitment.as_ref() != Some(&hash)
            || intent.expected_generation.checked_add(1) != Some(stored.auth_generation)
            || stored.action != ProviderAuthAction::Apply
        {
            return Ok(None);
        }
        self.open_vault(stored)?;
        Ok(Some(VerifiedCommit {
            intent_hash: hash,
            generation: stored.auth_generation,
        }))
    }

    /// The caller has already verified the fresh native candidate, signed
    /// release and DB intent. Commit one Machine grant, never a shared scope.
    pub fn commit_grant(
        &self,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
        intent: &Receipt,
    ) -> Result<VerifiedCommit> {
        let hash = self.validate_grant_intent(intent)?;
        validate_candidate(package, bundle, intent)?;
        validate_bundle(&package.manifest.authentication, bundle)?;
        let plaintext = serde_json::to_vec(bundle)?;
        ensure!(
            plaintext.len() <= MAX_BUNDLE_BYTES,
            "credential bundle is too large"
        );
        let generation = intent
            .expected_generation
            .checked_add(1)
            .filter(|value| *value != u64::MAX)
            .context("Provider auth generation exhausted")?;
        let fingerprint = &package.manifest.compatibility.auth_contract_fingerprint;
        let mut providers = self.providers.write();
        if let Some(stored) = providers.get(&intent.provider_id)
            && stored.promotion_commitment.as_ref() == Some(&hash)
            && stored.auth_generation == generation
        {
            ensure!(
                &stored.auth_contract_fingerprint == fingerprint
                    && self.open_vault(stored)? == plaintext,
                "Provider grant replay changed credentials"
            );
            return Ok(VerifiedCommit {
                intent_hash: hash,
                generation,
            });
        }
        ensure!(
            providers
                .get(&intent.provider_id)
                .map_or(0, |stored| stored.auth_generation)
                == intent.expected_generation,
            "Provider authentication generation changed; restart authentication"
        );
        let (nonce, ciphertext) = self.seal_vault(
            &intent.provider_id,
            generation,
            fingerprint,
            &plaintext,
            Some(&hash),
        )?;
        self.replace_locked(
            &mut providers,
            StoredProviderAuthentication {
                vault_schema: self.vault_schema(),
                provider_id: intent.provider_id.clone(),
                account_id: self.account_id.clone(),
                recipient_key: self.recipient_key.clone(),
                promotion_commitment: Some(hash.clone()),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: fingerprint.clone(),
                portable_schema: package.manifest.authentication.portable_schema.clone(),
                projection_schema: package.manifest.authentication.projection_schema.clone(),
                action: ProviderAuthAction::Apply,
                nonce,
                ciphertext,
                account_label: None,
                updated_at_ms: now_ms(),
            },
        )?;
        self.transient.write().remove(&intent.provider_id);
        Ok(VerifiedCommit {
            intent_hash: hash,
            generation,
        })
    }
}
