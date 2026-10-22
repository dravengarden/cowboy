//! Local vault revocation only: never evidence of Machine cleanup or logout
//! at the upstream Provider. The caller owns account-revocation admission.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub enum Revocation {
    Applied {
        generation: u64,
    },
    /// Authenticated current empty vault, not proof of which request wiped it.
    AlreadySignedOut {
        generation: u64,
    },
    Superseded,
}

impl ProviderAuthService {
    /// Revocation recovery may observe an already authenticated empty vault
    /// after a lost database commit. This does not claim a new write occurred.
    pub fn discard_revoking_grant(&self, receipt: &Receipt) -> Result<bool> {
        if self.discard_cancelled_grant(receipt)? {
            return Ok(true);
        }
        let providers = self.providers.read();
        let Some(stored) = providers.get(&receipt.provider_id) else {
            return Ok(false);
        };
        let promoted = receipt
            .expected_generation
            .checked_add(1)
            .context("grant generation overflow")?;
        if stored.action != ProviderAuthAction::Wipe
            || stored.authentication_state != ServiceAuthenticationState::SignedOut
            || stored.promotion_commitment.is_some()
            || stored.auth_generation <= promoted
        {
            return Ok(false);
        }
        let plaintext = self.open_vault(stored)?;
        validate_vault_plaintext(stored, &plaintext)?;
        Ok(true)
    }

    /// Seal only an authenticated tombstone for this historical recipient.
    /// Caller must separately admit the current account and Machine enrollment.
    pub fn seal_revoked_grant(
        &self,
        receipt: &Receipt,
        recipient: &str,
    ) -> Result<SealedProviderAuth> {
        ensure!(
            crate::admin::hex_sha256(recipient.as_bytes()) == receipt.recipient_fingerprint,
            "Provider revocation recipient changed"
        );
        let Phase::Committed { generation } = receipt.phase else {
            anyhow::bail!("Provider grant is not committed");
        };
        ensure!(
            receipt.expected_generation.checked_add(1) == Some(generation),
            "Provider grant generation mismatch"
        );
        let mut intent = receipt.clone();
        intent.phase = Phase::Prepared;
        self.validate_grant_intent(&intent)?;
        let stored = self
            .providers
            .read()
            .get(&receipt.provider_id)
            .cloned()
            .context("Provider grant vault missing")?;
        let plaintext = self.open_vault(&stored)?;
        validate_vault_plaintext(&stored, &plaintext)?;
        ensure!(
            stored.action == ProviderAuthAction::Wipe
                && stored.authentication_state == ServiceAuthenticationState::SignedOut
                && stored.promotion_commitment.is_none()
                && stored.auth_generation > generation,
            "Provider grant has not been locally revoked"
        );
        let envelope = self.seal_for_machine(&receipt.provider_id, recipient)?;
        ensure!(
            envelope.action == ProviderAuthAction::Wipe
                && envelope.auth_generation == stored.auth_generation,
            "Provider revocation changed"
        );
        Ok(envelope)
    }

    /// Lookup a historical local recipient even if its Machine was re-enrolled
    /// or revoked. This never creates a vault or authorizes remote delivery.
    pub fn existing_revocation_grant(&self, receipt: &Receipt) -> Result<Arc<Self>> {
        ensure!(
            self.account_id.is_default() && !receipt.account_id.is_default(),
            "named account lookup requires the root vault"
        );
        let account = self
            .accounts
            .lock()
            .get(&receipt.account_id)
            .cloned()
            .context("Provider account vault missing")?;
        let grant = account
            .grants
            .lock()
            .iter()
            .find(|(key, _)| {
                crate::admin::hex_sha256(key.as_bytes()) == receipt.recipient_fingerprint
            })
            .map(|(_, grant)| Arc::clone(grant))
            .context("Provider recipient vault missing")?;
        Ok(grant)
    }

    pub fn revoke_committed_grant(&self, receipt: &Receipt) -> Result<Revocation> {
        let Phase::Committed { generation } = receipt.phase else {
            anyhow::bail!("Provider grant is not committed");
        };
        ensure!(
            receipt.expected_generation.checked_add(1) == Some(generation),
            "Provider grant generation mismatch"
        );
        let mut intent = receipt.clone();
        intent.phase = Phase::Prepared;
        let hash = self.validate_grant_intent(&intent)?;
        let mut providers = self.providers.write();
        let current = providers
            .get(&receipt.provider_id)
            .context("Provider grant vault missing")?;
        // Authenticate even a claimed tombstone before trusting its metadata.
        let plaintext = self.open_vault(current)?;
        validate_vault_plaintext(current, &plaintext)?;
        if current.action == ProviderAuthAction::Wipe
            && current.authentication_state == ServiceAuthenticationState::SignedOut
            && current.promotion_commitment.is_none()
            && current.auth_generation > generation
        {
            return Ok(Revocation::AlreadySignedOut {
                generation: current.auth_generation,
            });
        }
        if current.action != ProviderAuthAction::Apply
            || current.promotion_commitment.as_ref() != Some(&hash)
            || current.auth_generation < generation
        {
            return Ok(Revocation::Superseded);
        }
        let mut tombstone = current.clone();
        tombstone.auth_generation = current
            .auth_generation
            .checked_add(1)
            .filter(|next| *next != u64::MAX)
            .context("Provider auth generation exhausted")?;
        let (nonce, ciphertext) = self.seal_vault(
            &receipt.provider_id,
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
        let generation = tombstone.auth_generation;
        self.replace_locked(&mut providers, tombstone)?;
        Ok(Revocation::Applied { generation })
    }
}
